//! Non-fatal authoring diagnostics over decoded FNX node values.
//!
//! `CanvasNode` flattens its `NodeData` variant, which defeats serde's
//! `deny_unknown_fields`: a typo'd attribute (`corner_raduis={8}`) decodes,
//! materializes, and renders as if it succeeded. This pass diffs every decoded
//! node's keys against [`fanta_doc::known_fields`] and turns each unknown key
//! into a [`SourceSeverity::Warning`] with a did-you-mean suggestion. Warnings
//! never fail the commit — unknown FUTURE fields must keep riding through
//! untouched — only an explicit caller policy (such as the harness `--deny`)
//! may promote them.

use super::types::{SourceDiagnostic, SourceSeverity};
use serde_json::Value;

/// Keys the decoder injects structurally on every node value. They are not
/// authored attributes and never count as unknown.
const STRUCTURAL_KEYS: [&str; 4] = ["type", "id", "parent", "index"];

/// The diagnostic code every unknown-attribute warning carries.
pub(crate) const UNKNOWN_ATTRIBUTE: &str = "source.unknown_attribute";

/// Diff every decoded node value's keys against the vocabulary table for its
/// `type` tag. A tag [`fanta_doc::known_fields`] has no table for (media,
/// instance, …) is skipped entirely — no table means no basis to warn, and a
/// false positive on every field would be worse than silence.
pub(crate) fn unknown_attribute_diagnostics(nodes: &[Value]) -> Vec<SourceDiagnostic> {
    let mut diagnostics = Vec::new();
    for node in nodes {
        let Some(object) = node.as_object() else {
            continue;
        };
        let Some(type_tag) = object.get("type").and_then(Value::as_str) else {
            continue;
        };
        let Some(known) = fanta_doc::known_fields(type_tag) else {
            continue;
        };
        let node_id = object.get("id").and_then(Value::as_str).map(str::to_owned);
        let node_name = object
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let tag = fanta_fnx::tag_for_type(type_tag).unwrap_or(type_tag);
        for key in object.keys() {
            if STRUCTURAL_KEYS.contains(&key.as_str()) || known.contains(key) {
                continue;
            }
            let suggestion = nearest(known.iter().map(String::as_str), key)
                .map(|candidate| format!("; did you mean `{candidate}`?"))
                .unwrap_or_default();
            let subject = match &node_name {
                Some(name) => format!("<{tag}> \"{name}\""),
                None => format!("<{tag}>"),
            };
            diagnostics.push(SourceDiagnostic {
                severity: SourceSeverity::Warning,
                code: UNKNOWN_ATTRIBUTE,
                node_id: node_id.clone(),
                node_name: node_name.clone(),
                message: format!(
                    "unknown attribute `{key}` on {subject}{suggestion} — the value is \
                     preserved in the source but ignored by the renderer"
                ),
            });
        }
    }
    diagnostics
}

/// The known field closest to `target` within edit distance 2, ties broken
/// lexicographically so messages are deterministic. Candidates whose length
/// differs by more than 2 cannot qualify and skip the quadratic distance.
///
/// Deliberately LOCAL: the sibling in `fanta_fnx::refs` is private to that
/// crate, and cross-crate reuse is not worth a public API for ~15 lines.
fn nearest<'a>(candidates: impl Iterator<Item = &'a str>, target: &str) -> Option<&'a str> {
    let target_len = target.chars().count();
    let mut best: Option<(usize, &str)> = None;
    for candidate in candidates {
        if candidate.chars().count().abs_diff(target_len) > 2 {
            continue;
        }
        let distance = edit_distance(candidate, target);
        if distance > 2 {
            continue;
        }
        let better = match best {
            None => true,
            Some((best_distance, best_name)) => {
                distance < best_distance || (distance == best_distance && candidate < best_name)
            }
        };
        if better {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, name)| name)
}

/// Optimal-string-alignment distance: Levenshtein plus adjacent transposition
/// as ONE edit, so the classic swap typo `corner_raduis` suggests
/// `corner_radius` (distance 1) over `corner_radii` (distance 2). Field names
/// are short, so the O(a·b) table is fine.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut table = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in table.iter_mut().enumerate() {
        row[0] = i;
    }
    for j in 0..=b.len() {
        table[0][j] = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let substitution = usize::from(a[i - 1] != b[j - 1]);
            let mut cost = (table[i - 1][j] + 1)
                .min(table[i][j - 1] + 1)
                .min(table[i - 1][j - 1] + substitution);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                cost = cost.min(table[i - 2][j - 2] + 1);
            }
            table[i][j] = cost;
        }
    }
    table[a.len()][b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn typo_warns_with_transposition_aware_suggestion() {
        let nodes = [json!({
            "type": "vector", "id": "01AAAAAAAAAAAAAAAAAAAAAAAA", "parent": null, "index": 1.0,
            "name": "Card", "corner_raduis": 8.0,
        })];
        let diagnostics = unknown_attribute_diagnostics(&nodes);
        assert_eq!(diagnostics.len(), 1);
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic.severity, SourceSeverity::Warning);
        assert_eq!(diagnostic.code, UNKNOWN_ATTRIBUTE);
        assert_eq!(
            diagnostic.node_id.as_deref(),
            Some("01AAAAAAAAAAAAAAAAAAAAAAAA")
        );
        assert_eq!(diagnostic.node_name.as_deref(), Some("Card"));
        assert!(
            diagnostic.message.contains("did you mean `corner_radius`?"),
            "the swap typo must suggest corner_radius, not corner_radii: {}",
            diagnostic.message
        );
        assert!(diagnostic.message.contains("preserved"));
    }

    #[test]
    fn known_and_structural_keys_produce_nothing() {
        let nodes = [json!({
            "type": "group", "id": "x", "parent": null, "index": 1.0,
            "name": "Home", "opacity": 0.5, "corner_radius": 4.0,
        })];
        assert!(unknown_attribute_diagnostics(&nodes).is_empty());
    }

    #[test]
    fn kinds_without_a_vocabulary_table_are_skipped() {
        let nodes = [json!({
            "type": "instance", "id": "x", "parent": null, "index": 1.0,
            "component": "y", "definitely_not_a_field": true,
        })];
        assert!(
            unknown_attribute_diagnostics(&nodes).is_empty(),
            "no table means skip validation, never false-positive"
        );
    }

    #[test]
    fn far_off_keys_warn_without_a_suggestion() {
        let nodes = [json!({
            "type": "text", "id": "x", "parent": null, "index": 1.0,
            "zzz_future_field": 1,
        })];
        let diagnostics = unknown_attribute_diagnostics(&nodes);
        assert_eq!(diagnostics.len(), 1);
        assert!(!diagnostics[0].message.contains("did you mean"));
        assert!(diagnostics[0].node_name.is_none());
    }

    #[test]
    fn osa_distance_counts_a_swap_as_one_edit() {
        assert_eq!(edit_distance("corner_raduis", "corner_radius"), 1);
        assert_eq!(edit_distance("corner_raduis", "corner_radii"), 2);
        assert_eq!(edit_distance("fillz", "fills"), 1);
        assert_eq!(edit_distance("same", "same"), 0);
    }
}
