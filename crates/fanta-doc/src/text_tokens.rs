//! `{{name}}` text variables: a token in a text node's content shows the
//! value of the variable it names. A token names a variable by its
//! `Collection/Name` path, or by a bare name only one variable has; a token
//! that names no variable stays as typed. String, number, boolean and enum
//! variables resolve; colors, typography and images do not.
//!
//! Tokens resolve wherever text is measured or drawn, from the content as
//! authored. The stored content always keeps the tokens.

use std::collections::BTreeMap;
use std::ops::Range;

use crate::{
    BoundProp, CanvasNode, ModeId, NodeData, NodeId, ResolvedVarValue, Scene, TextStyleRun,
    VariableCollectionId, VariableId, VariableRegistry, resolve_bound_value,
};

/// The `{{name}}` tokens in `content`: each token's byte range, braces
/// included, and its trimmed name. A name holds no braces or line breaks.
pub fn text_tokens(content: &str) -> impl Iterator<Item = (Range<usize>, &str)> + '_ {
    let mut cursor = 0;
    std::iter::from_fn(move || {
        while let Some(found) = content.get(cursor..).and_then(|rest| rest.find("{{")) {
            let start = cursor + found;
            let name_start = start + 2;
            let Some(length) = content.get(name_start..).and_then(|rest| rest.find("}}")) else {
                cursor = content.len();
                return None;
            };
            let raw = content.get(name_start..name_start + length)?;
            if raw.contains(['{', '}', '\n']) || raw.trim().is_empty() {
                // Not a token; look again from the second brace, which may
                // open one.
                cursor = start + 1;
                continue;
            }
            let end = name_start + length + 2;
            cursor = end;
            return Some((start..end, raw.trim()));
        }
        None
    })
}

/// Whether the node is text whose content may hold a token: the cheap check
/// before interpolating.
pub fn has_text_tokens(node: &CanvasNode) -> bool {
    match &node.data {
        NodeData::Text(text) => text.content.contains("{{"),
        NodeData::TextPath(text) => text.content.contains("{{"),
        _ => false,
    }
}

/// `content` with every token `resolve` knows replaced by its value, and
/// `runs` moved to follow the text: a value takes the style of its token's
/// first byte. `None` when no token was replaced.
pub fn interpolate_text(
    content: &str,
    runs: &[TextStyleRun],
    resolve: &mut dyn FnMut(&str) -> Option<String>,
) -> Option<(String, Vec<TextStyleRun>)> {
    let replacements: Vec<(Range<usize>, String)> = text_tokens(content)
        .filter_map(|(range, name)| resolve(name).map(|value| (range, value)))
        .collect();
    if replacements.is_empty() {
        return None;
    }
    let mut text = String::with_capacity(content.len());
    let mut copied = 0;
    for (range, value) in &replacements {
        text.push_str(content.get(copied..range.start)?);
        text.push_str(value);
        copied = range.end;
    }
    text.push_str(content.get(copied..)?);
    let remap = |offset: usize| -> usize {
        let mut removed = 0;
        let mut added = 0;
        for (range, value) in &replacements {
            if offset <= range.start {
                break;
            }
            if offset < range.end {
                return range.start + added - removed + value.len();
            }
            removed += range.len();
            added += value.len();
        }
        offset + added - removed
    };
    let runs = runs
        .iter()
        .filter_map(|run| {
            let start = remap(run.start);
            let end = remap(run.end);
            (end > start).then(|| TextStyleRun {
                start,
                end,
                style: run.style.clone(),
            })
        })
        .collect();
    Some((text, runs))
}

/// The variable a token names: the one variable whose `Collection/Name` path
/// is `name`, else the one variable called `name`.
pub fn text_token_variable(variables: &VariableRegistry, name: &str) -> Option<VariableId> {
    let mut by_path = None;
    let mut path_matches = 0;
    let mut by_name = None;
    let mut name_matches = 0;
    for variable in variables.variables.values() {
        if variable.name == name {
            by_name = Some(variable.id);
            name_matches += 1;
        }
        let in_path = variables
            .collections
            .get(&variable.collection)
            .and_then(|collection| name.strip_prefix(collection.name.as_str()))
            .and_then(|rest| rest.strip_prefix('/'))
            .is_some_and(|rest| rest == variable.name);
        if in_path {
            by_path = Some(variable.id);
            path_matches += 1;
        }
    }
    match (path_matches, name_matches) {
        (1, _) => by_path,
        (0, 1) => by_name,
        _ => None,
    }
}

/// A token's text in the mode `mode_anchor` resolves to, or `None` when the
/// token names no variable or one that is not text-like.
pub fn resolve_text_token(
    variables: &VariableRegistry,
    scene: &Scene,
    mode_anchor: NodeId,
    active_modes: &BTreeMap<VariableCollectionId, ModeId>,
    name: &str,
) -> Option<String> {
    let variable = text_token_variable(variables, name)?;
    match resolve_bound_value(variables, scene, mode_anchor, active_modes, variable)? {
        ResolvedVarValue::String { value } | ResolvedVarValue::Enum { value } => Some(value),
        ResolvedVarValue::Float { value } => Some(value.to_string()),
        ResolvedVarValue::Boolean { value } => Some(value.to_string()),
        ResolvedVarValue::Color { .. }
        | ResolvedVarValue::TextStyle { .. }
        | ResolvedVarValue::Image { .. } => None,
    }
}

/// Replace the tokens in a text or text-path node's content with their
/// values, in the mode `mode_anchor` resolves to. Returns whether anything
/// changed.
pub fn apply_text_tokens(
    node: &mut CanvasNode,
    variables: &VariableRegistry,
    scene: &Scene,
    mode_anchor: NodeId,
    active_modes: &BTreeMap<VariableCollectionId, ModeId>,
) -> bool {
    let (content, runs) = match &mut node.data {
        NodeData::Text(text) => (&mut text.content, &mut text.style_runs),
        NodeData::TextPath(text) => (&mut text.content, &mut text.style_runs),
        _ => return false,
    };
    if !content.contains("{{") {
        return false;
    }
    let Some((text, interpolated_runs)) = interpolate_text(content, runs, &mut |name| {
        resolve_text_token(variables, scene, mode_anchor, active_modes, name)
    }) else {
        return false;
    };
    *content = text;
    *runs = interpolated_runs;
    true
}

/// The text node `id` as it is drawn: its bound text content and style
/// applied, then its tokens. `None` when `id` is not text.
pub fn displayed_text_node(
    scene: &Scene,
    variables: &VariableRegistry,
    active_modes: &BTreeMap<VariableCollectionId, ModeId>,
    id: NodeId,
) -> Option<CanvasNode> {
    let node = scene.get(id)?;
    if !matches!(node.data, NodeData::Text(_) | NodeData::TextPath(_)) {
        return None;
    }
    let mut displayed = node.clone();
    for (property, variable) in &node.bindings {
        if matches!(property, BoundProp::TextStyle | BoundProp::TextContent)
            && let Some(value) = resolve_bound_value(variables, scene, id, active_modes, *variable)
        {
            property.apply_resolved(&mut displayed, value);
        }
    }
    apply_text_tokens(&mut displayed, variables, scene, id, active_modes);
    Some(displayed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Doc, GroupNode, Mode, Operation, TextNode, TextStyle, VarValue, Variable,
        VariableCollection, VariableType,
    };

    fn names(content: &str) -> Vec<(Range<usize>, &str)> {
        text_tokens(content).collect()
    }

    #[test]
    fn tokens_are_trimmed_names_between_double_braces() {
        assert_eq!(
            names("Hi {{ first name }}, {{plan}}!"),
            vec![(3..19, "first name"), (21..29, "plan")]
        );
        assert!(names("{{}} {{ }} {{ {x} }} {{a\nb}} {{open").is_empty());
        assert_eq!(names("{{{name}}}"), vec![(1..9, "name")]);
        assert_eq!(names("ünï {{ü}}"), vec![(6..12, "ü")]);
    }

    #[test]
    fn unmatched_tokens_stay_literal() {
        let interpolated = interpolate_text("{{a}} and {{b}}", &[], &mut |name| {
            (name == "b").then(|| "B".to_owned())
        });
        assert_eq!(
            interpolated.map(|(text, _)| text),
            Some("{{a}} and B".into())
        );
        assert_eq!(interpolate_text("{{a}}", &[], &mut |_| None), None);
    }

    #[test]
    fn style_runs_follow_text_around_replaced_tokens() {
        let bold = TextStyle {
            weight: 700,
            ..TextStyle::default()
        };
        // "Hi {{name}}!" with "Hi " plain, the token bold, "!" plain.
        let runs = vec![
            TextStyleRun {
                start: 0,
                end: 3,
                style: TextStyle::default(),
            },
            TextStyleRun {
                start: 3,
                end: 11,
                style: bold.clone(),
            },
            TextStyleRun {
                start: 11,
                end: 12,
                style: TextStyle::default(),
            },
        ];
        let (text, runs) = interpolate_text("Hi {{name}}!", &runs, &mut |_| {
            Some("Ada Lovelace".to_owned())
        })
        .expect("interpolated");
        assert_eq!(text, "Hi Ada Lovelace!");
        assert_eq!(
            runs.iter()
                .map(|run| (run.start, run.end))
                .collect::<Vec<_>>(),
            vec![(0, 3), (3, 15), (15, 16)]
        );
        assert_eq!(runs[1].style, bold);
        for run in &runs {
            assert!(text.is_char_boundary(run.start) && text.is_char_boundary(run.end));
        }
    }

    fn doc_with_variables() -> (Doc, VariableCollectionId, ModeId, ModeId) {
        let mut doc = Doc::new();
        let collection = VariableCollectionId::new();
        let light = ModeId::new();
        let dark = ModeId::new();
        doc.variables.collections.insert(
            collection,
            VariableCollection {
                id: collection,
                name: "Copy".into(),
                modes: vec![
                    Mode {
                        id: light,
                        name: "English".into(),
                    },
                    Mode {
                        id: dark,
                        name: "Spanish".into(),
                    },
                ],
                default_mode: light,
                variable_order: Vec::new(),
            },
        );
        for (name, english, spanish) in [("greeting", "Hello", "Hola"), ("count", "3", "4")] {
            let id = VariableId::new();
            let values = |value: &str| {
                if name == "count" {
                    VarValue::Float {
                        value: value.parse().unwrap_or_default(),
                    }
                } else {
                    VarValue::String {
                        value: value.to_owned(),
                    }
                }
            };
            doc.variables.variables.insert(
                id,
                Variable {
                    id,
                    collection,
                    name: name.into(),
                    ty: if name == "count" {
                        VariableType::Float
                    } else {
                        VariableType::String
                    },
                    values_by_mode: BTreeMap::from([
                        (light, values(english)),
                        (dark, values(spanish)),
                    ]),
                    scopes: Default::default(),
                    options: Default::default(),
                },
            );
        }
        (doc, collection, light, dark)
    }

    #[test]
    fn qualified_tokens_use_collection_paths_and_bare_tokens_need_a_unique_name() {
        let (mut doc, _, _, _) = doc_with_variables();
        let greeting = text_token_variable(&doc.variables, "greeting").expect("bare name");
        assert_eq!(
            text_token_variable(&doc.variables, "Copy/greeting"),
            Some(greeting)
        );
        assert_eq!(text_token_variable(&doc.variables, "Other/greeting"), None);
        let twin = VariableId::new();
        let mut copy = doc.variables.variables[&greeting].clone();
        copy.id = twin;
        doc.variables.variables.insert(twin, copy);
        assert_eq!(
            text_token_variable(&doc.variables, "greeting"),
            None,
            "an ambiguous bare name stays literal"
        );
    }

    #[test]
    fn tokens_resolve_in_the_mode_a_frame_pins() {
        let (mut doc, collection, _, spanish) = doc_with_variables();
        let mut frame = GroupNode {
            clip_size: Some([200.0, 100.0]),
            ..GroupNode::default()
        };
        frame.explicit_modes.insert(collection, spanish);
        let frame = CanvasNode::new(NodeData::Group(frame));
        let frame_id = frame.id;
        doc.apply(Operation::create_node(frame)).expect("frame");
        let mut label = CanvasNode::new(NodeData::Text(TextNode::new(
            "{{greeting}} x{{count}}",
            100.0,
            20.0,
        )));
        label.parent = Some(frame_id);
        let label_id = label.id;
        doc.apply(Operation::create_node(label)).expect("label");
        let displayed =
            displayed_text_node(&doc.scene, &doc.variables, &doc.active_modes, label_id)
                .expect("text");
        let NodeData::Text(text) = &displayed.data else {
            panic!("a text node");
        };
        assert_eq!(text.content, "Hola x4");
        let NodeData::Text(authored) = &doc.scene.get(label_id).expect("label").data else {
            panic!("a text node");
        };
        assert_eq!(
            authored.content, "{{greeting}} x{{count}}",
            "the source keeps its tokens"
        );
    }
}
