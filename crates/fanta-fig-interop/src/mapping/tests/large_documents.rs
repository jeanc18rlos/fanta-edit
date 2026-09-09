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
                                KiwiValue::Array(vec![o(
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
