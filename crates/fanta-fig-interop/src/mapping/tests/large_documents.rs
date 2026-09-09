//! Scaling guards: import work that must stay linear in the node count.
//!
//! A `.fig` page routinely holds thousands of direct children, and a
//! design-system file thousands of component masters over tens of thousands of
//! nodes. A per-item step that touches "everything so far" — the per-master
//! scan of every mapped guid that override-path resolution used to do — turns
//! import quadratic. These tests pin the shapes at sizes that stay well under a
//! second in a debug build with linear work, while the quadratic version of the
//! same import is measured in tens of seconds; the bounds sit in between.

use super::*;
use std::time::{Duration, Instant};

const DOCUMENT_GUID: u32 = 0;
const CANVAS_GUID: u32 = 1;

fn document_and_canvas() -> Vec<KiwiValue> {
    vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, DOCUMENT_GUID)),
                ("type", KiwiValue::Enum("DOCUMENT".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, CANVAS_GUID)),
                ("parentIndex", parent_index(0, DOCUMENT_GUID)),
                ("type", KiwiValue::Enum("CANVAS".into())),
                ("name", KiwiValue::String("Page".to_owned())),
            ],
        ),
    ]
}

/// A CANVAS with `count` RECTANGLE children whose `position` strings run in
/// REVERSE of stream order, so the position sort and the z-order it mints are
/// both exercised at scale.
fn canvas_with_many_children(count: u32) -> FigDocument {
    let mut changes = document_and_canvas();
    for i in 0..count {
        // Zero-padded so lexicographic order equals numeric order; the highest
        // position is emitted FIRST in the stream.
        let position = format!("{:08}", count - 1 - i);
        changes.push(o(
            "NodeChange",
            vec![
                ("guid", guid(0, 10 + i)),
                ("parentIndex", parent_index_pos(0, CANVAS_GUID, &position)),
                ("type", KiwiValue::Enum("RECTANGLE".into())),
                ("name", KiwiValue::String(position)),
                ("size", vector(10.0, 10.0)),
            ],
        ));
    }
    doc_from(changes)
}

/// `masters` SYMBOL masters of `children_per_master` RECTANGLE children each,
/// plus one INSTANCE per master on the page carrying a visibility override
/// that addresses the master's first child. Every instance resolves its
/// override path against a DIFFERENT master, so per-master path building runs
/// `masters` times over a scene of `masters × (children + 2)` nodes.
fn many_masters_with_overriding_instances(masters: u32, children_per_master: u32) -> FigDocument {
    let mut changes = document_and_canvas();
    // Guid ranges: masters at 1_000_000+i, children at 2_000_000 + i*stride,
    // instances at 3_000_000+i.
    for master in 0..masters {
        let master_guid = 1_000_000 + master;
        changes.push(o(
            "NodeChange",
            vec![
                ("guid", guid(0, master_guid)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String(format!("Master {master}"))),
                ("size", vector(100.0, 100.0)),
            ],
        ));
        for child in 0..children_per_master {
            changes.push(o(
                "NodeChange",
                vec![
                    (
                        "guid",
                        guid(0, 2_000_000 + master * children_per_master + child),
                    ),
                    ("parentIndex", parent_index(0, master_guid)),
                    ("type", KiwiValue::Enum("RECTANGLE".into())),
                    ("size", vector(10.0, 10.0)),
                ],
            ));
        }
        changes.push(o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3_000_000 + master)),
                ("parentIndex", parent_index(0, CANVAS_GUID)),
                ("type", KiwiValue::Enum("INSTANCE".into())),
                ("size", vector(100.0, 100.0)),
                (
                    "symbolData",
                    o(
                        "SymbolData",
                        vec![
                            ("symbolID", guid(0, master_guid)),
                            (
                                "symbolOverrides",
                                KiwiValue::array(vec![o(
                                    "NodeChange",
                                    vec![
                                        (
                                            "guidPath",
                                            guid_path(0, 2_000_000 + master * children_per_master),
                                        ),
                                        ("visible", KiwiValue::Bool(false)),
                                    ],
                                )]),
                            ),
                        ],
                    ),
                ),
            ],
        ));
    }
    doc_from(changes)
}

fn timed_import(fig: &FigDocument) -> (Doc, MapReport, Duration) {
    let started = Instant::now();
    let (doc, report, _assets) = fig_to_doc(fig).unwrap();
    (doc, report, started.elapsed())
}

/// The page's children by name (the zero-padded position), bottom→top.
fn child_names(doc: &Doc) -> Vec<&str> {
    let canvas = doc.scene.roots()[0];
    doc.scene
        .children_of(Some(canvas))
        .iter()
        .map(|id| doc.scene.get(*id).unwrap().name.as_str())
        .collect()
}

#[test]
fn twenty_thousand_siblings_attach_in_position_order_within_budget() {
    let fig = canvas_with_many_children(20_000);
    let (doc, _report, elapsed) = timed_import(&fig);
    let names = child_names(&doc);
    assert_eq!(names.len(), 20_000);
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(
        names, sorted,
        "children attach bottom→top in ascending position order, not stream order"
    );
    doc.scene.validate().unwrap();
    eprintln!("20k siblings imported in {elapsed:?}");
    assert!(
        elapsed < Duration::from_secs(10),
        "20k-sibling import took {elapsed:?}"
    );
}

/// Three thousand masters, each instanced once with an override: per-master
/// path building must walk only that master's subtree. Rebuilding the whole
/// `guid → node` inverse per master (3k × 27k nodes) took ~15 s here.
#[test]
fn thousands_of_masters_with_overriding_instances_import_within_budget() {
    let fig = many_masters_with_overriding_instances(3_000, 8);
    let (doc, report, elapsed) = timed_import(&fig);
    assert_eq!(report.components, 3_000);
    assert_eq!(report.instances, 3_000);
    assert_eq!(
        report.instances_with_overrides, 3_000,
        "every instance resolved its override against its own master"
    );
    assert_eq!(report.overrides_applied, 3_000);
    doc.scene.validate().unwrap();
    eprintln!("3k masters + 3k overriding instances imported in {elapsed:?}");
    assert!(
        elapsed < Duration::from_secs(5),
        "many-masters import took {elapsed:?}"
    );
}

/// A CANVAS with `count` RECTANGLE children whose positions are a fixed
/// permutation of stream order (neither ascending nor reversed), so the
/// position sort has real work to do and no accidental run helps it.
fn canvas_with_shuffled_children(count: u32) -> FigDocument {
    // `count` and the stride are coprime, so `i * stride mod count` visits
    // every slot exactly once.
    const STRIDE: u64 = 7_919;
    let mut changes = document_and_canvas();
    for i in 0..count {
        let slot = (u64::from(i) * STRIDE) % u64::from(count);
        let position = format!("{slot:08}");
        changes.push(o(
            "NodeChange",
            vec![
                ("guid", guid(0, 10 + i)),
                ("parentIndex", parent_index_pos(0, CANVAS_GUID, &position)),
                ("type", KiwiValue::Enum("RECTANGLE".into())),
                ("name", KiwiValue::String(position)),
                ("size", vector(10.0, 10.0)),
            ],
        ));
    }
    doc_from(changes)
}

/// `frames` FRAMEs on the page, each with `children_per_frame` RECTANGLEs in
/// reverse position order. Every odd frame's children are emitted in the
/// stream BEFORE the frame itself, so the batch insert sees parents that
/// follow their children as well as ones that precede them.
fn canvas_with_frames(frames: u32, children_per_frame: u32) -> FigDocument {
    let mut changes = document_and_canvas();
    for frame in 0..frames {
        let frame_guid = 1_000_000 + frame;
        let frame_change = o(
            "NodeChange",
            vec![
                ("guid", guid(0, frame_guid)),
                (
                    "parentIndex",
                    parent_index_pos(0, CANVAS_GUID, &format!("{frame:08}")),
                ),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String(format!("Frame {frame}"))),
                ("size", vector(100.0, 100.0)),
            ],
        );
        let children_first = frame % 2 == 1;
        if !children_first {
            changes.push(frame_change.clone());
        }
        for child in 0..children_per_frame {
            let position = format!("{:08}", children_per_frame - 1 - child);
            changes.push(o(
                "NodeChange",
                vec![
                    (
                        "guid",
                        guid(0, 2_000_000 + frame * children_per_frame + child),
                    ),
                    ("parentIndex", parent_index_pos(0, frame_guid, &position)),
                    ("type", KiwiValue::Enum("RECTANGLE".into())),
                    ("name", KiwiValue::String(position)),
                    ("size", vector(10.0, 10.0)),
                ],
            ));
        }
        if children_first {
            changes.push(frame_change);
        }
    }
    doc_from(changes)
}

/// The whole page goes into the scene as one batch, so the scene revision
/// after import is a handful of edits however many nodes there are. A
/// per-node insert or reparent would put it in the tens of thousands.
const IMPORT_REVISION_CEILING: u64 = 64;

#[test]
fn fifty_thousand_shuffled_siblings_import_as_one_batch_within_budget() {
    let fig = canvas_with_shuffled_children(50_000);
    let (doc, report, elapsed) = timed_import(&fig);
    assert_eq!(report.mapped, 50_001, "page + every rectangle");
    let names = child_names(&doc);
    assert_eq!(names.len(), 50_000);
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(
        names, sorted,
        "children attach bottom→top in ascending position order"
    );
    doc.scene.validate().unwrap();
    assert!(
        doc.scene.revision() < IMPORT_REVISION_CEILING,
        "import made {} scene edits for 50k nodes — a per-node mutation is back",
        doc.scene.revision()
    );
    eprintln!("50k shuffled siblings imported in {elapsed:?}");
    assert!(
        elapsed < Duration::from_secs(20),
        "50k-sibling import took {elapsed:?}"
    );
}

#[test]
fn frames_whose_children_precede_them_attach_in_one_batch_within_budget() {
    let fig = canvas_with_frames(400, 50);
    let (doc, report, elapsed) = timed_import(&fig);
    assert_eq!(report.mapped, 1 + 400 + 400 * 50);
    let canvas = doc.scene.roots()[0];
    let frames = doc.scene.children_of(Some(canvas));
    assert_eq!(frames.len(), 400);
    for (position, frame) in frames.iter().enumerate() {
        let frame_node = doc.scene.get(*frame).unwrap();
        assert_eq!(frame_node.name, format!("Frame {position}"));
        let children: Vec<&str> = doc
            .scene
            .children_of(Some(*frame))
            .iter()
            .map(|id| doc.scene.get(*id).unwrap().name.as_str())
            .collect();
        assert_eq!(children.len(), 50);
        let mut sorted = children.clone();
        sorted.sort_unstable();
        assert_eq!(children, sorted, "frame {position} children out of order");
    }
    doc.scene.validate().unwrap();
    assert!(
        doc.scene.revision() < IMPORT_REVISION_CEILING,
        "import made {} scene edits",
        doc.scene.revision()
    );
    eprintln!("400 frames × 50 children imported in {elapsed:?}");
    assert!(
        elapsed < Duration::from_secs(10),
        "frames import took {elapsed:?}"
    );
}

#[test]
fn a_built_node_whose_guid_a_later_change_reclaims_still_reaches_the_scene() {
    // Pass 1 keys `guid_to_node` by guid, and a later change sharing a guid
    // overwrites the entry with `None` (structural, unsupported, or motion).
    // The node it built is then invisible to pass 2's attach loop, so it is in
    // neither the planned nor the dropped set — the batch must still carry it
    // rather than dropping content silently.
    let mut changes = document_and_canvas();
    changes.push(o(
        "NodeChange",
        vec![
            ("guid", guid(0, 10)),
            ("parentIndex", parent_index_pos(0, CANVAS_GUID, "00")),
            ("type", KiwiValue::Enum("RECTANGLE".into())),
            ("name", KiwiValue::String("Reclaimed".to_owned())),
            ("size", vector(10.0, 10.0)),
        ],
    ));
    // Same guid, a type the mapper does not build a node for.
    changes.push(o(
        "NodeChange",
        vec![
            ("guid", guid(0, 10)),
            ("parentIndex", parent_index_pos(0, CANVAS_GUID, "01")),
            ("type", KiwiValue::Enum("STICKY".into())),
        ],
    ));
    // A sibling that keeps its guid, so the page still has ordinary content.
    changes.push(o(
        "NodeChange",
        vec![
            ("guid", guid(0, 11)),
            ("parentIndex", parent_index_pos(0, CANVAS_GUID, "02")),
            ("type", KiwiValue::Enum("RECTANGLE".into())),
            ("name", KiwiValue::String("Kept".to_owned())),
            ("size", vector(10.0, 10.0)),
        ],
    ));

    let (doc, _, _) = fig_to_doc(&doc_from(changes)).expect("import");
    let mut names: Vec<String> = Vec::new();
    for root in doc.scene.roots().to_vec() {
        for id in doc.scene.descendants_of(root) {
            if let Some(node) = doc.scene.get(id) {
                names.push(node.name.clone());
            }
        }
    }
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    assert!(
        names.contains(&"Kept"),
        "the ordinary sibling must import, got {names:?}"
    );
    assert!(
        names.contains(&"Reclaimed"),
        "a node whose guid a later change reclaimed must not vanish, got {names:?}"
    );
}
