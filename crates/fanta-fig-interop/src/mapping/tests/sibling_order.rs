//! Sibling z-order from Figma's `parentIndex.position` (the fractional-index
//! string), not the `.fig` NodeChange STREAM order.
//!
//! Figma stores nodes in a flat array whose ORDER is unrelated to stacking; the
//! authoritative sibling order is `parentIndex.position`, a string compared
//! LEXICOGRAPHICALLY. The importer sorts each parent's children by it before
//! attaching, so the minted `IndexKey`s reflect Figma stacking. In fanta a
//! higher `IndexKey` paints later (on top), and `children_of` returns children
//! ascending (bottom-first) — so the LAST child in `children_of` is topmost,
//! which must correspond to the HIGHEST `position` (matching Figma + the
//! OpenPencil oracle, whose `sortChildren` sorts ascending by `position` then
//! paints first-to-last).

use super::*;

/// Wrap a set of children (each `(guid_lid, type, name, position)`) under a
/// single CANVAS, emitting the children in the given STREAM order (the order of
/// the vec) — distinct from their `position` strings so the position-sort is the
/// only thing that can produce the expected child order.
fn canvas_with_children(children: &[(u32, &str, &str, &str)]) -> FigDocument {
    let mut changes = vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".into())),
                ("name", KiwiValue::String("Page".to_owned())),
            ],
        ),
    ];
    for (lid, ty, name, pos) in children {
        changes.push(o(
            "NodeChange",
            vec![
                ("guid", guid(0, *lid)),
                ("parentIndex", parent_index_pos(0, 1, pos)),
                ("type", KiwiValue::Enum((*ty).into())),
                ("name", KiwiValue::String((*name).to_owned())),
                ("size", vector(100.0, 100.0)),
            ],
        ));
    }
    doc_from(changes)
}

/// The CANVAS page's direct children, by name, in z-ascending (bottom→top)
/// paint order.
fn child_names(doc: &Doc) -> Vec<String> {
    let canvas = doc.scene.roots()[0];
    doc.scene
        .children_of(Some(canvas))
        .iter()
        .map(|id| doc.scene.get(*id).unwrap().name.clone())
        .collect()
}

// =============================================================================
// (a) stream order != position order  ->  child order follows position
// =============================================================================

#[test]
fn sibling_order_follows_position_not_stream_order() {
    // Emit in STREAM order C, A, B but give positions A<B<C. The resulting
    // bottom→top child order must be A, B, C (position ascending), proving the
    // importer ignores stream order in favor of `parentIndex.position`.
    let fig = canvas_with_children(&[
        (10, "RECTANGLE", "C", "c"),
        (11, "RECTANGLE", "A", "a"),
        (12, "RECTANGLE", "B", "b"),
    ]);
    let (doc, _report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(
        child_names(&doc),
        vec!["A", "B", "C"],
        "children must sort by parentIndex.position (ascending), not stream order"
    );
}

#[test]
fn sibling_order_is_lexicographic_not_numeric() {
    // Fractional-index strings are compared as STRINGS. With single-char ASCII
    // positions, "{" (0x7B) > "Z" (0x5A) > "A" (0x41). A numeric parse would
    // choke / reorder these; lexicographic ordering yields A, Z, {.
    let fig = canvas_with_children(&[
        (10, "RECTANGLE", "brace", "{"),
        (11, "RECTANGLE", "upperA", "A"),
        (12, "RECTANGLE", "upperZ", "Z"),
    ]);
    let (doc, _report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(child_names(&doc), vec!["upperA", "upperZ", "brace"]);
}

// =============================================================================
// (b) a mask sibling + following siblings -> mask attaches to the correct run
//     AFTER reordering
// =============================================================================

#[test]
fn mask_run_follows_position_order_not_stream_order() {
    use fanta_doc::node::MaskType;
    // Figma mask semantics: a mask masks the FOLLOWING siblings (higher in the
    // child sequence) up to the next mask. The masked run is determined by the
    // POSITION order, not the stream order. Emit in stream order [B, Mask, A]
    // but positions Mask < A < B, so the bottom→top sequence is [Mask, A, B]:
    // the mask is the bottom-most child and its run is {A, B}.
    let mut changes = vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".into())),
                ("name", KiwiValue::String("Page".to_owned())),
            ],
        ),
    ];
    // Stream order: B (pos "c"), Mask (pos "a"), A (pos "b").
    changes.push(o(
        "NodeChange",
        vec![
            ("guid", guid(0, 12)),
            ("parentIndex", parent_index_pos(0, 1, "c")),
            ("type", KiwiValue::Enum("RECTANGLE".into())),
            ("name", KiwiValue::String("B".to_owned())),
            ("size", vector(100.0, 100.0)),
        ],
    ));
    changes.push(o(
        "NodeChange",
        vec![
            ("guid", guid(0, 10)),
            ("parentIndex", parent_index_pos(0, 1, "a")),
            ("type", KiwiValue::Enum("ROUNDED_RECTANGLE".into())),
            ("name", KiwiValue::String("Mask".to_owned())),
            ("size", vector(100.0, 100.0)),
            ("mask", KiwiValue::Bool(true)),
        ],
    ));
    changes.push(o(
        "NodeChange",
        vec![
            ("guid", guid(0, 11)),
            ("parentIndex", parent_index_pos(0, 1, "b")),
            ("type", KiwiValue::Enum("RECTANGLE".into())),
            ("name", KiwiValue::String("A".to_owned())),
            ("size", vector(100.0, 100.0)),
        ],
    ));
    let fig = doc_from(changes);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();

    // After the position sort the child sequence is Mask, A, B (bottom→top).
    assert_eq!(child_names(&doc), vec!["Mask", "A", "B"]);

    // The mask is at sequence index 0, so the renderer's mask-run finder
    // (`paint_child_sequence`) masks indices 1..3 = {A, B}. Re-derive that run
    // here from the resolved child order to prove the reorder put the mask
    // FIRST (a mask emitted LAST in the stream would have masked nothing).
    let canvas = doc.scene.roots()[0];
    let kids: Vec<_> = doc.scene.children_of(Some(canvas)).to_vec();
    let mask_idx = kids
        .iter()
        .position(|id| doc.scene.get(*id).unwrap().is_mask)
        .expect("a mask child is present");
    assert_eq!(
        mask_idx, 0,
        "mask must be the bottom-most child after reorder"
    );
    let run_len = kids.len() - (mask_idx + 1);
    assert_eq!(run_len, 2, "mask's following run is {{A, B}}");
    assert_eq!(
        doc.scene.get(kids[mask_idx]).unwrap().mask_type,
        MaskType::Alpha
    );
    assert_eq!(report.masks_imported, 1);
}

// =============================================================================
// (c) direction not inverted: overlapping rects, the higher-position sibling is
//     painted last (topmost) — matching Figma / OpenPencil
// =============================================================================

#[test]
fn higher_position_sibling_is_topmost_not_inverted() {
    // Two overlapping rects. In Figma the child with the HIGHER position string
    // is later in the children sequence → painted last → on top. Emit them in
    // the OPPOSITE stream order (top-most first) to ensure only the position
    // sort can produce the correct stacking. `children_of` is z-ascending, so
    // the LAST entry is the topmost.
    let fig = canvas_with_children(&[
        // stream order: TopMost first (pos "z"), then Bottom (pos "a").
        (10, "RECTANGLE", "TopMost", "z"),
        (11, "RECTANGLE", "Bottom", "a"),
    ]);
    let (doc, _report, _) = fig_to_doc(&fig).unwrap();
    let names = child_names(&doc); // bottom -> top
    assert_eq!(
        names,
        vec!["Bottom", "TopMost"],
        "ascending position => ascending IndexKey; highest position is topmost \
         (NOT inverted)"
    );
    // Cross-check via the raw IndexKey: the topmost child has the larger key.
    let canvas = doc.scene.roots()[0];
    let kids = doc.scene.children_of(Some(canvas));
    let bottom_key = doc.scene.get(kids[0]).unwrap().index.raw();
    let top_key = doc.scene.get(kids[1]).unwrap().index.raw();
    assert!(
        top_key > bottom_key,
        "topmost (highest position) must carry the higher IndexKey"
    );
}

// =============================================================================
// (d) missing / equal positions -> stable fallback to stream order, no panic
// =============================================================================

#[test]
fn equal_positions_fall_back_to_stable_stream_order() {
    // Three children all sharing the SAME position string. With no ordering
    // signal the importer must preserve stream order (stable) and not panic.
    let fig = canvas_with_children(&[
        (10, "RECTANGLE", "first", "m"),
        (11, "RECTANGLE", "second", "m"),
        (12, "RECTANGLE", "third", "m"),
    ]);
    let (doc, _report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(child_names(&doc), vec!["first", "second", "third"]);
}

#[test]
fn missing_positions_fall_back_to_stable_stream_order() {
    // Children whose ParentIndex carries NO `position` field at all. The
    // missing-position fallback is stable stream order.
    let mut changes = vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".into())),
                ("name", KiwiValue::String("Page".to_owned())),
            ],
        ),
    ];
    for (lid, name) in [(10u32, "alpha"), (11, "beta"), (12, "gamma")] {
        changes.push(o(
            "NodeChange",
            vec![
                ("guid", guid(0, lid)),
                ("parentIndex", parent_index_no_pos(0, 1)),
                ("type", KiwiValue::Enum("RECTANGLE".into())),
                ("name", KiwiValue::String(name.to_owned())),
                ("size", vector(100.0, 100.0)),
            ],
        ));
    }
    let fig = doc_from(changes);
    let (doc, _report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(child_names(&doc), vec!["alpha", "beta", "gamma"]);
}

#[test]
fn mixed_present_and_missing_positions_do_not_panic() {
    // A blend: some children carry a position, some don't. Missing positions
    // sort as the empty string (lowest), then present ones ascending; ties (the
    // two missing) keep stream order. No panic, deterministic order.
    let mut changes = vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".into())),
                ("name", KiwiValue::String("Page".to_owned())),
            ],
        ),
    ];
    // Stream: hasZ (pos "z"), noneA (no pos), hasB (pos "b"), noneC (no pos).
    changes.push(o(
        "NodeChange",
        vec![
            ("guid", guid(0, 10)),
            ("parentIndex", parent_index_pos(0, 1, "z")),
            ("type", KiwiValue::Enum("RECTANGLE".into())),
            ("name", KiwiValue::String("hasZ".to_owned())),
            ("size", vector(100.0, 100.0)),
        ],
    ));
    changes.push(o(
        "NodeChange",
        vec![
            ("guid", guid(0, 11)),
            ("parentIndex", parent_index_no_pos(0, 1)),
            ("type", KiwiValue::Enum("RECTANGLE".into())),
            ("name", KiwiValue::String("noneA".to_owned())),
            ("size", vector(100.0, 100.0)),
        ],
    ));
    changes.push(o(
        "NodeChange",
        vec![
            ("guid", guid(0, 12)),
            ("parentIndex", parent_index_pos(0, 1, "b")),
            ("type", KiwiValue::Enum("RECTANGLE".into())),
            ("name", KiwiValue::String("hasB".to_owned())),
            ("size", vector(100.0, 100.0)),
        ],
    ));
    changes.push(o(
        "NodeChange",
        vec![
            ("guid", guid(0, 13)),
            ("parentIndex", parent_index_no_pos(0, 1)),
            ("type", KiwiValue::Enum("RECTANGLE".into())),
            ("name", KiwiValue::String("noneC".to_owned())),
            ("size", vector(100.0, 100.0)),
        ],
    ));
    let fig = doc_from(changes);
    let (doc, _report, _) = fig_to_doc(&fig).unwrap();
    // Empty-string positions (missing) sort first, in stream order (noneA,
    // noneC), then present positions ascending (hasB "b", hasZ "z").
    assert_eq!(child_names(&doc), vec!["noneA", "noneC", "hasB", "hasZ"]);
}

// =============================================================================
// Direct unit tests of the `attach_order` helper (grouping + stability).
// =============================================================================

#[test]
fn attach_order_groups_by_parent_then_sorts_by_position() {
    // Build the maps `fig_to_doc` would: parent P (stream idx 0) with three
    // children emitted out of position order, plus a sibling parent Q after.
    let order: Vec<String> = ["P", "P:c", "P:a", "P:b", "Q", "Q:b", "Q:a"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut parent: HashMap<String, Option<String>> = HashMap::new();
    let mut pos: HashMap<String, Option<String>> = HashMap::new();
    parent.insert("P".into(), None);
    parent.insert("Q".into(), None);
    pos.insert("P".into(), Some("p".into()));
    pos.insert("Q".into(), Some("q".into()));
    for (g, p) in [("P:c", "c"), ("P:a", "a"), ("P:b", "b")] {
        parent.insert(g.into(), Some("P".into()));
        pos.insert(g.into(), Some(p.into()));
    }
    for (g, p) in [("Q:b", "b"), ("Q:a", "a")] {
        parent.insert(g.into(), Some("Q".into()));
        pos.insert(g.into(), Some(p.into()));
    }

    let out = attach_order(&order, &parent, &pos);
    // Each PARENT's children stay contiguous, ordered by position within the
    // group, and the groups themselves follow their parent's stream
    // first-appearance (P's children before Q's). Root-parented nodes (parent
    // `None`) carry the synthetic max group key, so they trail — harmless, since
    // attach order only matters AMONG siblings sharing a resolved parent, and
    // P/Q (roots) are ordered among themselves by stream index (P before Q).
    assert_eq!(out, vec!["P:a", "P:b", "P:c", "Q:a", "Q:b", "P", "Q"]);
    // The load-bearing invariants: each parent's children are position-sorted,
    // and roots keep stream order among themselves.
    let p_kids: Vec<_> = out.iter().filter(|g| g.starts_with("P:")).collect();
    assert_eq!(p_kids, vec!["P:a", "P:b", "P:c"]);
    let roots: Vec<_> = out.iter().filter(|g| g.len() == 1).collect();
    assert_eq!(roots, vec!["P", "Q"]);
}

#[test]
fn attach_order_is_stable_for_equal_and_missing_positions() {
    let order: Vec<String> = ["P", "x", "y", "z"].iter().map(|s| s.to_string()).collect();
    let mut parent: HashMap<String, Option<String>> = HashMap::new();
    let mut pos: HashMap<String, Option<String>> = HashMap::new();
    parent.insert("P".into(), None);
    pos.insert("P".into(), Some("p".into()));
    // x, z share position "m"; y has no position. Missing ("") sorts first, then
    // the equal "m" pair keeps stream order (x before z).
    parent.insert("x".into(), Some("P".into()));
    parent.insert("y".into(), Some("P".into()));
    parent.insert("z".into(), Some("P".into()));
    pos.insert("x".into(), Some("m".into()));
    pos.insert("y".into(), None);
    pos.insert("z".into(), Some("m".into()));

    let out = attach_order(&order, &parent, &pos);
    // P (root) trails (max group key). Among P's children: missing position ("")
    // sorts first (y), then the equal "m" pair keeps stream order (x before z).
    assert_eq!(out, vec!["y", "x", "z", "P"]);
}

// =============================================================================
// The reorder is per-parent: distinct subtrees keep independent orderings.
// =============================================================================

#[test]
fn position_sort_is_scoped_per_parent() {
    // Two frames each with two children. Positions are per-parent (only
    // comparable among true siblings), so each frame's children sort
    // independently; the frames themselves keep stream order.
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".into())),
                ("name", KiwiValue::String("Page".to_owned())),
            ],
        ),
        // Frame F1 (pos "a") then F2 (pos "b") under the canvas.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 10)),
                ("parentIndex", parent_index_pos(0, 1, "a")),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("F1".to_owned())),
                ("size", vector(200.0, 200.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 20)),
                ("parentIndex", parent_index_pos(0, 1, "b")),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("F2".to_owned())),
                ("size", vector(200.0, 200.0)),
            ],
        ),
        // F1 children emitted high→low position to force the reorder.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 11)),
                ("parentIndex", parent_index_pos(0, 10, "y")),
                ("type", KiwiValue::Enum("RECTANGLE".into())),
                ("name", KiwiValue::String("F1-top".to_owned())),
                ("size", vector(50.0, 50.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 12)),
                ("parentIndex", parent_index_pos(0, 10, "x")),
                ("type", KiwiValue::Enum("RECTANGLE".into())),
                ("name", KiwiValue::String("F1-bottom".to_owned())),
                ("size", vector(50.0, 50.0)),
            ],
        ),
        // F2 children, also high→low.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 21)),
                ("parentIndex", parent_index_pos(0, 20, "y")),
                ("type", KiwiValue::Enum("RECTANGLE".into())),
                ("name", KiwiValue::String("F2-top".to_owned())),
                ("size", vector(50.0, 50.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 22)),
                ("parentIndex", parent_index_pos(0, 20, "x")),
                ("type", KiwiValue::Enum("RECTANGLE".into())),
                ("name", KiwiValue::String("F2-bottom".to_owned())),
                ("size", vector(50.0, 50.0)),
            ],
        ),
    ]);
    let (doc, _report, _) = fig_to_doc(&fig).unwrap();

    let canvas = doc.scene.roots()[0];
    let frames = doc.scene.children_of(Some(canvas));
    let frame_names: Vec<_> = frames
        .iter()
        .map(|id| doc.scene.get(*id).unwrap().name.clone())
        .collect();
    assert_eq!(frame_names, vec!["F1", "F2"], "frames keep their own order");

    let kids = |fname: &str| -> Vec<String> {
        let fid = frames
            .iter()
            .copied()
            .find(|id| doc.scene.get(*id).unwrap().name == fname)
            .unwrap();
        doc.scene
            .children_of(Some(fid))
            .iter()
            .map(|id| doc.scene.get(*id).unwrap().name.clone())
            .collect()
    };
    assert_eq!(kids("F1"), vec!["F1-bottom", "F1-top"]);
    assert_eq!(kids("F2"), vec!["F2-bottom", "F2-top"]);
}
