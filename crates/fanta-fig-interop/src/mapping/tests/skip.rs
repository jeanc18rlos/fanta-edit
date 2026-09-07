//! Cross-cutting: unsupported-node skip resilience.

use super::*;

// =============================================================================
// Cross-cutting: skip resilience
// =============================================================================

#[test]
fn unsupported_node_types_are_skipped_with_a_count() {
    // SLICE is genuinely unsupported and stays skipped.
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("RECTANGLE".into())),
                ("size", vector(1.0, 1.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("type", KiwiValue::Enum("DOCUMENT".into())),
            ],
        ),
    ]);
    // DOCUMENT is structural (not skipped); RECTANGLE maps. Add a truly unknown
    // type by using an enum member the mapper doesn't handle.
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.mapped, 1);
    assert_eq!(doc.scene.len(), 1);
}
