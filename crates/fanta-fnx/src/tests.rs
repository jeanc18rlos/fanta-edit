//! Round-trip tests: a subtree of node values must survive
//! `tree_from_nodes → print_doc → parse_doc → nodes_from_tree` unchanged.

use super::*;
use serde_json::{Value, json};

/// Compare two node-value sets ignoring order (nodes are keyed by `id`).
fn assert_same_nodes(a: &[Value], b: &[Value]) {
    let key = |v: &Value| v.get("id").and_then(Value::as_str).unwrap().to_owned();
    let mut a = a.to_vec();
    let mut b = b.to_vec();
    a.sort_by_key(key);
    b.sort_by_key(key);
    assert_eq!(a, b, "round-trip changed the node set");
}

fn round_trip(nodes: &[Value]) -> Vec<Value> {
    let tree = tree_from_nodes(nodes).expect("decompose");
    let text = print_doc("Card", &tree.root);
    let root = parse_doc(&text).expect("parse");
    nodes_from_tree(&root, &tree.sidecar, tree.root_parent.as_deref()).expect("recompose")
}

/// A representative subtree: a frame root with nested frame, text, vector —
/// exercising strings, numbers, bools, arrays, nested objects (incl. a color
/// object), and an opaque `meta` blob.
fn sample() -> Vec<Value> {
    vec![
        json!({
            "type": "group", "id": "ROOT0000000000000000000000", "parent": null, "index": 1.0,
            "name": "Card", "opacity": 0.9, "corner_radius": 8.0,
            "background": { "kind": "solid", "color": { "r": 37, "g": 99, "b": 235, "a": 255 } },
            "meta": { "authoredBy": "test", "z": [1, 2, 3] }
        }),
        json!({
            "type": "text", "id": "TEXT0000000000000000000000", "parent": "ROOT0000000000000000000000",
            "index": 1.0, "name": "Title", "content": "Hello, world",
            "style": { "font_family": "Inter", "size_px": 16.0, "weight": 600, "italic": false }
        }),
        json!({
            "type": "vector", "id": "VEC00000000000000000000000", "parent": "ROOT0000000000000000000000",
            "index": 2.0, "name": "Box",
            "fills": [{ "kind": "solid", "color": { "r": 0, "g": 0, "b": 0, "a": 255 } }],
            "corner_radius": 4.0
        }),
        json!({
            "type": "group", "id": "INNER000000000000000000000", "parent": "ROOT0000000000000000000000",
            "index": 3.0, "name": "Inner", "auto_layout": { "mode": "horizontal", "spacing": 12.0 }
        }),
        json!({
            "type": "instance", "id": "INST0000000000000000000000", "parent": "INNER000000000000000000000",
            "index": 1.0, "name": "Btn", "component": "COMP0000000000000000000000",
            "local_size": [120.0, 40.0]
        }),
    ]
}

#[test]
fn full_subtree_round_trips_losslessly() {
    let nodes = sample();
    let back = round_trip(&nodes);
    assert_same_nodes(&nodes, &back);
}

#[test]
fn embedded_objects_print_identically_across_insertion_orders() {
    let unsorted = r#"{"z":[{"z":9,"a":"$Colors/Accent"},null,3,false],"a":{"z":21.762165069580078,"a":"literal reference"}}"#;
    let sorted = r#"{"a":{"a":"literal reference","z":21.762165069580078},"z":[{"a":"$Colors/Accent","z":9},null,3,false]}"#;
    let mut root = FnxElement::new("Frame");
    root.attrs.insert(
        "meta".to_owned(),
        serde_json::from_str(unsorted).expect("unsorted metadata"),
    );
    root.attrs.insert(
        "background".to_owned(),
        serde_json::from_str(r#"{"kind":"solid","color":{"r":37,"g":99,"b":235,"a":255}}"#)
            .expect("solid fill"),
    );
    let mut instance = FnxElement::new("Instance");
    instance.attrs.insert(
        "component".to_owned(),
        Value::String("COMP0000000000000000000000".to_owned()),
    );
    root.children.push(instance);
    let original_metadata = serde_json::to_string(&root.attrs["meta"]).expect("original metadata");
    let source = print_doc("Ordering", &root);
    assert_eq!(parse_doc(&source).expect("printed source parses"), root);
    assert_eq!(
        serde_json::to_string(&root.attrs["meta"]).expect("metadata after print"),
        original_metadata,
        "printing must not reorder the caller's opaque metadata"
    );
    assert!(source.contains(r##"background={{"color": fnxColor("#2563EB"), "kind": "solid"}}"##));
    assert!(source.contains(r#"meta={{"a": {"a": "literal reference", "z": 21.762165069580078}, "z": [{"a": "$Colors/Accent", "z": 9}, null, 3, false]}}"#));
    assert!(source.contains(r#"component="COMP0000000000000000000000""#));

    root.attrs.insert(
        "meta".to_owned(),
        serde_json::from_str(sorted).expect("sorted metadata"),
    );
    root.attrs.insert(
        "background".to_owned(),
        serde_json::from_str(r#"{"color":{"a":255,"b":235,"g":99,"r":37},"kind":"solid"}"#)
            .expect("reordered solid fill"),
    );
    assert_eq!(print_doc("Ordering", &root), source);
}

#[test]
fn duplicate_node_ids_are_rejected_before_tree_indexing() {
    let mut nodes = sample();
    let duplicate = nodes[0]["id"].clone();
    nodes[1]["id"] = duplicate;
    assert!(matches!(
        tree_from_nodes(&nodes),
        Err(FnxError::Parse(message)) if message.contains("duplicate node id")
    ));
}

#[test]
fn printed_source_looks_like_react() {
    let nodes = sample();
    let tree = tree_from_nodes(&nodes).unwrap();
    let text = print_doc("Card", &tree.root);
    assert!(text.starts_with(
        "/** @jsxRuntime classic */\n/** @jsx fnxElement */\nimport { AiArtifact, Audio, Boolean, Ellipse, Embed, Frame, Image, Instance, Model3D, NodeGraph, Rect, Text, TextPath, Vector, Video } from \"../../fnx\";\n"
    ));
    assert!(text.contains("export default function Card()"));
    assert!(text.contains("<Frame"));
    assert!(text.contains("<Text"));
    assert!(text.contains("<Vector"));
    assert!(text.contains("<Instance"));
    // Nesting: the inner frame's instance child closes inside it.
    assert!(text.contains("</Frame>"));
    // Color sugar: the root background {r:37,g:99,b:235,a:255} renders as a
    // compact TSX-valid helper call, not a verbose JSON object.
    assert!(
        text.contains("fnxColor(\"#2563EB\")"),
        "color should render through fnxColor: {text}"
    );
    // The opaque ULIDs are NOT in the readable source — they live in the sidecar.
    assert!(
        !text.contains("ROOT0000000000000000000000"),
        "node ids must not leak into the source: {text}"
    );
    assert_eq!(tree.sidecar.len(), nodes.len());
    assert_eq!(tree.sidecar[0].id, "ROOT0000000000000000000000");
}

#[test]
fn generated_color_sugar_is_valid_tsx_and_legacy_colors_still_parse() {
    let nodes = sample();
    let tree = tree_from_nodes(&nodes).unwrap();
    let text = print_doc("Card", &tree.root);
    assert!(text.contains("fnxColor(\"#2563EB\")"));
    assert!(!text.contains("color: #2563EB"));

    let legacy = text.replace("fnxColor(\"#2563EB\")", "#2563EB");
    let root = parse_doc(&legacy).unwrap();
    let decoded = nodes_from_tree(&root, &tree.sidecar, tree.root_parent.as_deref()).unwrap();
    assert_same_nodes(&decoded, &nodes);
}

#[test]
fn canonicalizer_upgrades_a_pre_header_generated_file_exactly_once() {
    let nodes = sample();
    let (current, _) = encode_subtree(&nodes, "Card").unwrap();
    let current_header = format!(
        "{}\n{}\n{}\n",
        crate::canonicalize::JSX_RUNTIME_PRAGMA,
        crate::canonicalize::JSX_FACTORY_PRAGMA,
        crate::canonicalize::FNX_TAG_IMPORT
    );
    let legacy = current
        .strip_prefix(&current_header)
        .expect("generated source has the current header");

    let upgraded = canonicalize_legacy_source(legacy).expect("legacy header should upgrade");
    assert_eq!(upgraded, current);
    assert_eq!(canonicalize_legacy_source(&upgraded), None);
}

#[test]
fn canonicalizer_preserves_user_comments_and_imports_when_seeding_the_header() {
    let legacy = r#"// license: keep this exact
import { projectHelper } from "./helpers";
// @generated fanta source — ids live in the sidecar
export default function Card() {
  return <Frame name="Card" />;
}
"#;

    let upgraded = canonicalize_legacy_source(legacy).expect("missing header should upgrade");
    assert!(upgraded.starts_with(
        "/** @jsxRuntime classic */\n/** @jsx fnxElement */\n// license: keep this exact\n"
    ));
    assert!(
        upgraded
            .contains("import { projectHelper } from \"./helpers\";\n// @generated fanta source")
    );
    assert!(upgraded.contains(&format!(
        "{}\nimport {{ projectHelper }} from \"./helpers\";",
        crate::canonicalize::FNX_TAG_IMPORT
    )));
    assert_eq!(upgraded.matches("@jsxRuntime classic").count(), 1);
    assert_eq!(upgraded.matches("@jsx fnxElement").count(), 1);
    assert_eq!(upgraded.matches("from \"../../fnx\"").count(), 1);
    assert_eq!(canonicalize_legacy_source(&upgraded), None);
}

#[test]
fn canonicalizer_augments_the_intermediate_type_only_header() {
    let intermediate = format!(
        "{}\n{}\nimport type {{}} from \"../../fnx\";\n// @generated fanta source\nexport default () => <Frame name=\"Card\" />;\n",
        crate::canonicalize::JSX_RUNTIME_PRAGMA,
        crate::canonicalize::JSX_FACTORY_PRAGMA
    );

    let upgraded =
        canonicalize_legacy_source(&intermediate).expect("type-only import has no runtime tags");
    assert!(upgraded.contains(&format!(
        "{}\nimport type {{}} from \"../../fnx\";",
        crate::canonicalize::FNX_TAG_IMPORT
    )));
    assert_eq!(
        upgraded
            .matches(crate::canonicalize::FNX_TAG_IMPORT)
            .count(),
        1
    );
    assert_eq!(upgraded.matches("import type {} from").count(), 1);
    assert_eq!(canonicalize_legacy_source(&upgraded), None);
}

#[test]
fn canonicalizer_rewrites_only_bare_colors_in_fnx_attribute_expressions() {
    let source = format!(
        "{}\n{}\n{}\n{}",
        crate::canonicalize::JSX_RUNTIME_PRAGMA,
        crate::canonicalize::JSX_FACTORY_PRAGMA,
        crate::canonicalize::FNX_TAG_IMPORT,
        r##"// @generated fanta source
const outside = #A1B2C3;
const quotedTag = "<Frame fill={#A1B2C3} />";
// <Frame fill={#A1B2C3} />
export default function Card() {
  return (
    <Frame
      fill={{color: #a1b2c3}}
      translucent={#01020380}
      nested={[#abcdef, {color:#10203040}]}
      existing={fnxColor("#112233")}
      doubleQuoted={"#445566"}
      singleQuoted={'#778899'}
      template={`#AABBCC`}
      comments={{before: /* #DDEEFF */ #ccddee, after: #123456 // #654321
      }}
      malformed={[#FFF, #12345, #1234567, #123456789, #123456px]}
    >
      #FEDCBA
    </Frame>
  );
}
"##
    );

    let upgraded = canonicalize_legacy_source(&source).expect("legacy colors should upgrade");
    for color in [
        "#A1B2C3",
        "#01020380",
        "#ABCDEF",
        "#10203040",
        "#CCDDEE",
        "#123456",
    ] {
        assert!(
            upgraded.contains(&format!("fnxColor(\"{color}\")")),
            "missing canonical {color}: {upgraded}"
        );
    }
    for preserved in [
        "const outside = #A1B2C3;",
        "\"<Frame fill={#A1B2C3} />\"",
        "// <Frame fill={#A1B2C3} />",
        "fnxColor(\"#112233\")",
        "doubleQuoted={\"#445566\"}",
        "singleQuoted={'#778899'}",
        "template={`#AABBCC`}",
        "/* #DDEEFF */",
        "// #654321",
        "[#FFF, #12345, #1234567, #123456789, #123456px]",
        "      #FEDCBA",
    ] {
        assert!(
            upgraded.contains(preserved),
            "canonicalizer changed {preserved:?}: {upgraded}"
        );
    }
    assert_eq!(canonicalize_legacy_source(&upgraded), None);
}

#[test]
fn canonicalizer_ignores_non_fnx_and_malformed_source() {
    assert_eq!(
        canonicalize_legacy_source(
            r##"const value = { color: #AABBCC, example: "<Frame fill={#112233} />" };"##
        ),
        None
    );

    let malformed = format!(
        "{}\n{}\n{}\nexport default () => <Frame fill={{{{ color: #AABBCC }};",
        crate::canonicalize::JSX_RUNTIME_PRAGMA,
        crate::canonicalize::JSX_FACTORY_PRAGMA,
        crate::canonicalize::FNX_TAG_IMPORT
    );
    assert_eq!(canonicalize_legacy_source(&malformed), None);
}

#[test]
fn canonicalized_legacy_source_decodes_to_the_same_nodes() {
    let nodes = sample();
    let (current, sidecar) = encode_subtree(&nodes, "Card").unwrap();
    let current_header = format!(
        "{}\n{}\n{}\n",
        crate::canonicalize::JSX_RUNTIME_PRAGMA,
        crate::canonicalize::JSX_FACTORY_PRAGMA,
        crate::canonicalize::FNX_TAG_IMPORT
    );
    let legacy = current
        .strip_prefix(&current_header)
        .unwrap()
        .replace("fnxColor(\"#2563EB\")", "#2563eb")
        .replace("fnxColor(\"#000000\")", "#000000");
    let canonicalized =
        canonicalize_legacy_source(&legacy).expect("legacy source should canonicalize");

    assert_eq!(canonicalized, current);
    let legacy_nodes = decode_subtree(&legacy, &sidecar).unwrap();
    let canonical_nodes = decode_subtree(&canonicalized, &sidecar).unwrap();
    assert_same_nodes(&legacy_nodes, &nodes);
    assert_same_nodes(&canonical_nodes, &nodes);
}

#[test]
fn single_self_closing_node_round_trips() {
    let nodes = vec![json!({
        "type": "vector", "id": "ONE00000000000000000000000", "parent": null, "index": 1.0,
        "name": "Lonely"
    })];
    assert_same_nodes(&nodes, &round_trip(&nodes));
}

#[test]
fn root_parent_is_preserved() {
    // A component master root whose parent is the (external) components page.
    let nodes = vec![json!({
        "type": "group", "id": "MASTER00000000000000000000", "parent": "PAGEHIDDEN0000000000000000",
        "index": 5.0, "name": "Button"
    })];
    let tree = tree_from_nodes(&nodes).unwrap();
    assert_eq!(
        tree.root_parent.as_deref(),
        Some("PAGEHIDDEN0000000000000000")
    );
    let root = parse_doc(&print_doc("Button", &tree.root)).unwrap();
    let back = nodes_from_tree(&root, &tree.sidecar, tree.root_parent.as_deref()).unwrap();
    assert_same_nodes(&nodes, &back);
}

#[test]
fn sidecar_reconciliation_assigns_ids_and_source_order_to_added_elements() {
    let nodes = vec![
        json!({
            "type": "group", "id": "ROOT0000000000000000000000", "parent": null, "index": 7.0,
            "name": "Page"
        }),
        json!({
            "type": "vector", "id": "OLD00000000000000000000000", "parent": "ROOT0000000000000000000000",
            "index": 9.0, "name": "Existing"
        }),
    ];
    let (source, sidecar) = encode_subtree(&nodes, "Page").unwrap();
    let source = source.replace(
        "      <Vector name=\"Existing\" />\n",
        "      <Vector name=\"Existing\" />\n      <Vector name=\"Added\" />\n",
    );
    let reconciled = reconcile_sidecar(&source, &sidecar, || {
        "NEW00000000000000000000000".to_owned()
    })
    .unwrap();

    assert_eq!(reconciled.ids.len(), 3);
    assert_eq!(reconciled.ids[0].id, "ROOT0000000000000000000000");
    assert_eq!(reconciled.ids[0].index, json!(7.0));
    assert_eq!(reconciled.ids[1].id, "OLD00000000000000000000000");
    assert_eq!(reconciled.ids[1].index, json!(1.0));
    assert_eq!(reconciled.ids[2].id, "NEW00000000000000000000000");
    assert_eq!(reconciled.ids[2].index, json!(2.0));

    let decoded = decode_subtree(&source, &reconciled).unwrap();
    assert_eq!(
        decoded[2].get("parent"),
        Some(&json!("ROOT0000000000000000000000"))
    );
}

/// A root frame with three named children — the fixture for the mid-tree
/// reconciliation tests below.
fn reconcile_fixture() -> (String, FnxSidecar) {
    let nodes = vec![
        json!({
            "type": "group", "id": "ROOT0000000000000000000000", "parent": null, "index": 7.0,
            "name": "Page"
        }),
        json!({
            "type": "vector", "id": "AAA00000000000000000000000", "parent": "ROOT0000000000000000000000",
            "index": 1.0, "name": "A"
        }),
        json!({
            "type": "text", "id": "BBB00000000000000000000000", "parent": "ROOT0000000000000000000000",
            "index": 2.0, "name": "B"
        }),
        json!({
            "type": "vector", "id": "CCC00000000000000000000000", "parent": "ROOT0000000000000000000000",
            "index": 3.0, "name": "C"
        }),
    ];
    encode_subtree(&nodes, "Page").expect("encode fixture")
}

#[test]
fn reconciled_sidecar_indices_match_document_index_serialization() {
    let (source, sidecar) = reconcile_fixture();
    let edited = source.replace(
        "<Vector name=\"A\" />",
        "<Vector name=\"A\" />\n      <Vector name=\"Added\" />",
    );
    assert_ne!(edited, source);
    let reconciled =
        reconcile_sidecar(&edited, &sidecar, sequential_minter()).expect("reconcile inserted node");
    assert_eq!(reconciled.ids.len(), sidecar.ids.len() + 1);
    for (position, entry) in reconciled.ids.iter().skip(1).enumerate() {
        let native_index = fanta_doc::IndexKey::from_raw(position as f64 + 1.0);
        assert_eq!(
            serde_json::to_vec(&entry.index).expect("sidecar index bytes"),
            serde_json::to_vec(&native_index).expect("document index bytes"),
            "a reconciled index must survive materialization and ordinary Save byte-identically"
        );
    }
}

#[test]
fn sidecar_attribute_edit_preserves_fractional_index_bytes() {
    let (source, mut sidecar) = reconcile_fixture();
    for (entry, index) in sidecar
        .ids
        .iter_mut()
        .zip([7.125, 0.125, 1.0000000000000002, 9.75])
    {
        entry.index = json!(index);
    }
    let original_bytes = serde_json::to_vec(&sidecar).expect("fractional sidecar bytes");
    let edited = source.replacen("name=\"Page\"", "name=\"Page\" opacity={0.75}", 1);
    assert_ne!(edited, source);
    let mut minted = false;
    let reconciled = reconcile_sidecar(&edited, &sidecar, || {
        minted = true;
        "unexpected new identity".to_owned()
    })
    .expect("reconcile attribute edit");
    assert!(!minted);
    assert_eq!(reconciled, sidecar);
    assert_eq!(
        serde_json::to_vec(&reconciled).expect("unchanged fractional sidecar bytes"),
        original_bytes
    );
}

fn sequential_minter() -> impl FnMut() -> String {
    let mut counter = 0u32;
    move || {
        counter += 1;
        format!("MINTED{counter:020}")
    }
}

fn entry_ids(sidecar: &FnxSidecar) -> Vec<&str> {
    sidecar.ids.iter().map(|entry| entry.id.as_str()).collect()
}

fn strip_fingerprints(sidecar: &FnxSidecar) -> FnxSidecar {
    FnxSidecar {
        root_parent: sidecar.root_parent.clone(),
        ids: sidecar
            .ids
            .iter()
            .map(|entry| IdEntry {
                id: entry.id.clone(),
                index: entry.index.clone(),
                tag: None,
                name: None,
                parent_index: None,
            })
            .collect(),
    }
}

#[test]
fn replacing_a_mid_tree_element_mints_a_fresh_id_and_keeps_sibling_ids() {
    let (source, sidecar) = reconcile_fixture();
    let edited = source.replace("<Text name=\"B\" />", "<Boolean name=\"B\" />");
    assert_ne!(edited, source);

    let reconciled = reconcile_sidecar(&edited, &sidecar, sequential_minter()).unwrap();
    assert_eq!(
        entry_ids(&reconciled),
        [
            "ROOT0000000000000000000000",
            "AAA00000000000000000000000",
            "MINTED00000000000000000001",
            "CCC00000000000000000000000",
        ],
        "the replacement must not inherit the replaced element's identity"
    );
    assert_eq!(reconciled.ids[2].tag.as_deref(), Some("Boolean"));
}

#[test]
fn inserting_a_mid_tree_element_keeps_every_existing_id() {
    let (source, sidecar) = reconcile_fixture();
    let edited = source.replace(
        "<Vector name=\"A\" />",
        "<Vector name=\"A\" />\n      <Vector name=\"Inserted\" />",
    );
    assert_ne!(edited, source);

    let reconciled = reconcile_sidecar(&edited, &sidecar, sequential_minter()).unwrap();
    assert_eq!(
        entry_ids(&reconciled),
        [
            "ROOT0000000000000000000000",
            "AAA00000000000000000000000",
            "MINTED00000000000000000001",
            "BBB00000000000000000000000",
            "CCC00000000000000000000000",
        ],
        "elements after the insertion point must keep their ids"
    );
    // Indices are normalized to source order on structural change.
    assert_eq!(reconciled.ids[0].index, json!(7.0));
    assert_eq!(reconciled.ids[1].index, json!(1.0));
    assert_eq!(reconciled.ids[2].index, json!(2.0));
    assert_eq!(reconciled.ids[3].index, json!(3.0));
    assert_eq!(reconciled.ids[4].index, json!(4.0));

    // Fingerprints stay in the sidecar only — decoded nodes carry none of them.
    let decoded = decode_subtree(&edited, &reconciled).unwrap();
    assert_eq!(
        decoded[2].get("parent"),
        Some(&json!("ROOT0000000000000000000000"))
    );
    assert!(decoded.iter().all(|node| node.get("tag").is_none()));
}

#[test]
fn deleting_a_mid_tree_element_keeps_following_sibling_ids() {
    let (source, sidecar) = reconcile_fixture();
    let edited = source.replace("      <Text name=\"B\" />\n", "");
    assert_ne!(edited, source);

    let reconciled = reconcile_sidecar(&edited, &sidecar, || {
        unreachable!("a delete must not mint ids")
    })
    .unwrap();
    assert_eq!(
        entry_ids(&reconciled),
        [
            "ROOT0000000000000000000000",
            "AAA00000000000000000000000",
            "CCC00000000000000000000000",
        ],
        "elements after the deleted one must keep their ids"
    );
}

#[test]
fn pure_attribute_edit_with_equal_count_leaves_the_sidecar_unchanged() {
    let (source, sidecar) = reconcile_fixture();
    let edited = source.replace(
        "<Vector name=\"A\" />",
        "<Vector name=\"A\" opacity={0.5} />",
    );
    assert_ne!(edited, source);

    let reconciled = reconcile_sidecar(&edited, &sidecar, || {
        unreachable!("an attribute edit must not mint ids")
    })
    .unwrap();
    assert_eq!(reconciled, sidecar);
}

#[test]
fn renaming_the_root_keeps_its_id_even_alongside_structural_edits() {
    let (source, sidecar) = reconcile_fixture();
    let edited = source.replace("name=\"Page\"", "name=\"Renamed\"").replace(
        "<Vector name=\"A\" />",
        "<Vector name=\"A\" />\n      <Vector name=\"Inserted\" />",
    );
    assert_ne!(edited, source);

    let reconciled = reconcile_sidecar(&edited, &sidecar, sequential_minter()).unwrap();
    assert_eq!(
        entry_ids(&reconciled),
        [
            "ROOT0000000000000000000000",
            "AAA00000000000000000000000",
            "MINTED00000000000000000001",
            "BBB00000000000000000000000",
            "CCC00000000000000000000000",
        ],
        "the file's root is this subtree's externally referenced identity"
    );
    assert_eq!(reconciled.ids[0].name.as_deref(), Some("Renamed"));
}

/// Above the quadratic LCS cap (~2000 elements, i.e. `.fig`-import scale) the
/// alignment must switch to the anchor path, not the pre-fingerprint
/// positional pairing that rebinds every surviving element to its deleted
/// predecessor's id.
#[test]
fn deleting_above_the_lcs_cap_keeps_every_surviving_id() {
    let mut nodes = vec![json!({
        "type": "group", "id": "ROOT0000000000000000000000", "parent": null, "index": 1.0,
        "name": "Page"
    })];
    for i in 0..2100u32 {
        nodes.push(json!({
            "type": "vector", "id": format!("SHAPE{i:021}"), "parent": "ROOT0000000000000000000000",
            "index": f64::from(i + 1), "name": format!("shape-{i}")
        }));
    }
    let (source, sidecar) = encode_subtree(&nodes, "Page").expect("encode big page");
    let edited = source.replacen("      <Vector name=\"shape-0\" />\n", "", 1);
    assert_ne!(edited, source);

    let reconciled = reconcile_sidecar(&edited, &sidecar, || {
        unreachable!("a delete must not mint ids, even above the LCS cap")
    })
    .unwrap();
    assert_eq!(reconciled.ids.len(), 2100);
    assert_eq!(reconciled.ids[0].id, "ROOT0000000000000000000000");
    for (position, entry) in reconciled.ids.iter().enumerate().skip(1) {
        assert_eq!(
            entry.id,
            format!("SHAPE{position:021}"),
            "surviving element {position} must keep its id"
        );
    }
}

/// A reparent that happens to preserve the flattened pre-order (moving the
/// last child of a frame out to be the frame's following sibling) is a
/// structural change: it must not take the pure-attribute fast path, which
/// would keep stale sibling indices that contradict the source's z-order.
#[test]
fn preorder_preserving_reparent_normalizes_indices_and_keeps_ids() {
    let nodes = vec![
        json!({
            "type": "group", "id": "ROOT0000000000000000000000", "parent": null, "index": 7.0,
            "name": "Page"
        }),
        json!({
            "type": "group", "id": "AAA00000000000000000000000", "parent": "ROOT0000000000000000000000",
            "index": 5.0, "name": "A"
        }),
        json!({
            "type": "vector", "id": "BBB00000000000000000000000", "parent": "AAA00000000000000000000000",
            "index": 2.0, "name": "B"
        }),
    ];
    let (source, sidecar) = encode_subtree(&nodes, "Page").expect("encode fixture");
    assert_eq!(sidecar.ids[1].parent_index, Some(0));
    assert_eq!(sidecar.ids[2].parent_index, Some(1));

    let edited = source.replace(
        "      <Frame name=\"A\">\n        <Vector name=\"B\" />\n      </Frame>\n",
        "      <Frame name=\"A\" />\n      <Vector name=\"B\" />\n",
    );
    assert_ne!(edited, source);

    let reconciled = reconcile_sidecar(&edited, &sidecar, || {
        unreachable!("a reparent must not mint ids")
    })
    .unwrap();
    assert_ne!(
        reconciled, sidecar,
        "a reparent must not return the stale sidecar via the fast path"
    );
    assert_eq!(
        entry_ids(&reconciled),
        [
            "ROOT0000000000000000000000",
            "AAA00000000000000000000000",
            "BBB00000000000000000000000",
        ],
        "both elements keep their ids across the reparent"
    );
    // Sibling indices are normalized to source order (A before B under the
    // page), so the decoded z-order matches the edited source.
    assert_eq!(reconciled.ids[1].index, json!(1.0));
    assert_eq!(reconciled.ids[2].index, json!(2.0));
    assert_eq!(reconciled.ids[1].parent_index, Some(0));
    assert_eq!(reconciled.ids[2].parent_index, Some(0));
}

/// Renaming a layer in source is the most common benign edit; it must be an
/// attribute edit of the same node (like it is on canvas), not a retire+mint
/// that breaks id-keyed references and the three-way merge.
#[test]
fn renaming_a_mid_tree_element_keeps_its_id() {
    let (source, sidecar) = reconcile_fixture();
    let edited = source.replace("<Text name=\"B\" />", "<Text name=\"B2\" />");
    assert_ne!(edited, source);

    let reconciled = reconcile_sidecar(&edited, &sidecar, || {
        unreachable!("a rename must not mint ids")
    })
    .unwrap();
    assert_eq!(
        entry_ids(&reconciled),
        [
            "ROOT0000000000000000000000",
            "AAA00000000000000000000000",
            "BBB00000000000000000000000",
            "CCC00000000000000000000000",
        ],
        "a source-side rename keeps the element's identity"
    );
    assert_eq!(reconciled.ids[2].name.as_deref(), Some("B2"));
}

/// Swapping two identically named sibling frames (duplicate layer names are
/// ubiquitous in imported files) is a pure z-reorder: every id must survive.
/// The two frames are indistinguishable by fingerprint, so the frame ids
/// follow z-order, but the distinctive children pair up by their unique
/// fingerprints instead of retiring and minting.
#[test]
fn swapping_identical_frames_keeps_every_subtree_id() {
    let nodes = vec![
        json!({
            "type": "group", "id": "ROOT0000000000000000000000", "parent": null, "index": 1.0,
            "name": "Page"
        }),
        json!({
            "type": "group", "id": "FRAME100000000000000000000", "parent": "ROOT0000000000000000000000",
            "index": 1.0, "name": "Card"
        }),
        json!({
            "type": "vector", "id": "ICON0000000000000000000000", "parent": "FRAME100000000000000000000",
            "index": 1.0, "name": "Icon"
        }),
        json!({
            "type": "group", "id": "FRAME200000000000000000000", "parent": "ROOT0000000000000000000000",
            "index": 2.0, "name": "Card"
        }),
        json!({
            "type": "text", "id": "LABEL000000000000000000000", "parent": "FRAME200000000000000000000",
            "index": 1.0, "name": "Label"
        }),
    ];
    let (source, sidecar) = encode_subtree(&nodes, "Page").expect("encode fixture");
    let edited = source.replace(
        "      <Frame name=\"Card\">\n        <Vector name=\"Icon\" />\n      </Frame>\n      <Frame name=\"Card\">\n        <Text name=\"Label\" />\n      </Frame>\n",
        "      <Frame name=\"Card\">\n        <Text name=\"Label\" />\n      </Frame>\n      <Frame name=\"Card\">\n        <Vector name=\"Icon\" />\n      </Frame>\n",
    );
    assert_ne!(edited, source);

    let reconciled = reconcile_sidecar(&edited, &sidecar, || {
        unreachable!("a z-reorder must not mint ids")
    })
    .unwrap();
    assert_eq!(
        entry_ids(&reconciled),
        [
            "ROOT0000000000000000000000",
            "FRAME100000000000000000000",
            "LABEL000000000000000000000",
            "FRAME200000000000000000000",
            "ICON0000000000000000000000",
        ],
        "no id may retire or mint on a pure reorder"
    );
    // The children stay pinned to their (re-fingerprinted) parents.
    assert_eq!(reconciled.ids[2].parent_index, Some(1));
    assert_eq!(reconciled.ids[4].parent_index, Some(3));
}

/// Replacing the root's TAG keeps the root id (the file's root IS the
/// externally referenced page/component) but must not take the fast path:
/// the rebuilt sidecar re-fingerprints the root and normalizes indices
/// instead of persisting a stale tag.
#[test]
fn replacing_the_root_tag_keeps_the_pinned_id_but_rebuilds_the_sidecar() {
    let (source, sidecar) = reconcile_fixture();
    let edited = source
        .replace("<Frame name=\"Page\"", "<Boolean name=\"Page\"")
        .replace("</Frame>", "</Boolean>");
    assert_ne!(edited, source);

    let reconciled = reconcile_sidecar(&edited, &sidecar, || {
        unreachable!("the root pin must keep the root id without minting")
    })
    .unwrap();
    assert_ne!(
        reconciled, sidecar,
        "a root tag change must not return the stale sidecar via the fast path"
    );
    assert_eq!(
        entry_ids(&reconciled),
        [
            "ROOT0000000000000000000000",
            "AAA00000000000000000000000",
            "BBB00000000000000000000000",
            "CCC00000000000000000000000",
        ],
    );
    assert_eq!(
        reconciled.ids[0].tag.as_deref(),
        Some("Boolean"),
        "the persisted root fingerprint must track the new tag"
    );
    assert_eq!(reconciled.ids[1].index, json!(1.0));
}

#[test]
fn legacy_sidecar_without_fingerprints_keeps_positional_behavior() {
    let (source, sidecar) = reconcile_fixture();
    let legacy = strip_fingerprints(&sidecar);

    // Equal count: the historical early return, even though an element was
    // structurally replaced (the legacy sidecar cannot tell).
    let replaced = source.replace("<Text name=\"B\" />", "<Boolean name=\"B\" />");
    let reconciled = reconcile_sidecar(&replaced, &legacy, || {
        unreachable!("equal-count legacy reconciliation must not mint ids")
    })
    .unwrap();
    assert_eq!(reconciled, legacy);

    // Count change: old entries are consumed in pre-order and the trailing
    // element mints, shifting ids across the insertion exactly as before.
    let inserted = source.replace(
        "<Vector name=\"A\" />",
        "<Vector name=\"A\" />\n      <Vector name=\"Inserted\" />",
    );
    let reconciled = reconcile_sidecar(&inserted, &legacy, sequential_minter()).unwrap();
    assert_eq!(
        entry_ids(&reconciled),
        [
            "ROOT0000000000000000000000",
            "AAA00000000000000000000000",
            "BBB00000000000000000000000",
            "CCC00000000000000000000000",
            "MINTED00000000000000000001",
        ],
    );
    // The rebuild also upgrades the legacy sidecar with fingerprints.
    assert_eq!(reconciled.ids[2].tag.as_deref(), Some("Vector"));
    assert_eq!(reconciled.ids[2].name.as_deref(), Some("Inserted"));
}

#[test]
fn sidecar_fingerprints_are_optional_in_serde_and_omitted_when_absent() {
    let legacy: FnxSidecar = serde_json::from_str(r#"{"ids":[{"id":"X","index":1.0}]}"#)
        .expect("legacy sidecars without fingerprints must still parse");
    assert_eq!(legacy.ids[0].tag, None);
    assert_eq!(legacy.ids[0].name, None);
    assert_eq!(
        serde_json::to_value(&legacy).unwrap(),
        json!({ "ids": [{ "id": "X", "index": 1.0 }] })
    );
}

#[test]
fn every_nodedata_variant_has_a_tag() {
    // The exhaustive match below fails to COMPILE if a `NodeData` variant is
    // added without a matching `TYPE_TAGS` entry — turning a silent
    // unsaveable-document bug into a build error. (The persistence layer also
    // falls back to per-node JSON for an un-tagged type, so this is the
    // early-warning guard, not the only safety net.)
    fn serde_type(n: &fanta_doc::NodeData) -> &'static str {
        use fanta_doc::NodeData::*;
        match n {
            Group(_) => "group",
            Vector(_) => "vector",
            Text(_) => "text",
            TextPath(_) => "text_path",
            Bitmap(_) => "bitmap",
            Video(_) => "video",
            Audio(_) => "audio",
            NodeGraph(_) => "node_graph",
            Model3d(_) => "model3d",
            AiArtifact(_) => "ai_artifact",
            Instance(_) => "instance",
            Boolean(_) => "boolean",
            Embed(_) => "embed",
        }
    }
    let _ = serde_type;
    for ty in [
        "group",
        "vector",
        "text",
        "text_path",
        "bitmap",
        "video",
        "audio",
        "node_graph",
        "model3d",
        "ai_artifact",
        "instance",
        "boolean",
        "embed",
    ] {
        assert!(tag_for_type(ty).is_some(), "no JSX tag for NodeData `{ty}`");
    }
}

#[test]
fn tag_type_bijection_covers_all_variants() {
    for ty in [
        "group",
        "vector",
        "text",
        "text_path",
        "bitmap",
        "video",
        "audio",
        "node_graph",
        "model3d",
        "ai_artifact",
        "instance",
        "boolean",
        "embed",
    ] {
        let tag = tag_for_type(ty).expect("tag for type");
        assert_eq!(type_for_tag(tag), Some(ty), "bijection broken for {ty}");
    }
}

#[test]
fn string_attr_with_special_chars_round_trips() {
    let nodes = vec![json!({
        "type": "text", "id": "STR00000000000000000000000", "parent": null, "index": 1.0,
        "name": "Quote \"x\" < & >", "content": "line1\nline2\ttab"
    })];
    assert_same_nodes(&nodes, &round_trip(&nodes));
}

/// The decisive test: round-trip the per-node JSON of a REAL `Doc` (the exact
/// projection `fanta-format`'s project tree writes), proving the codec handles
/// the actual serde shapes (transform `[6]`, flattened `type`, `SmallVec`
/// fills/strokes, `NodeFlags`, fractional `index`) — not just hand-written JSON.
#[test]
fn real_doc_node_projection_round_trips() {
    use fanta_doc::{
        CanvasNode, Color, Doc, GroupNode, NodeData, Operation, PathData, Stroke, TextNode,
        TextPathAlignment, TextPathDirection, TextPathNode, TextPathSide, TextPathStart,
        VectorNode,
    };

    let mut doc = Doc::new();
    let mut root = CanvasNode::new(NodeData::Group(GroupNode {
        background: Some(fanta_doc::Fill::solid(Color::WHITE)),
        corner_radius: Some(12.0),
        ..Default::default()
    }));
    root.name = "Card".to_owned();
    root.opacity = 0.95f32.into();
    let root_id = root.id;
    doc.apply(Operation::create_node(root)).unwrap();

    let mut rect = CanvasNode::new(NodeData::Vector({
        let mut v = VectorNode::rect_solid(0.0, 0.0, 40.0, 40.0, Color::rgb(0x11, 0x22, 0x33));
        v.corner_radius = Some(4.0);
        v.strokes
            .push(Stroke::solid(Color::rgb(0xAA, 0xBB, 0xCC), 2.0));
        v
    }));
    rect.parent = Some(root_id);
    doc.apply(Operation::create_node(rect)).unwrap();

    let mut text = CanvasNode::new(NodeData::Text(TextNode::new("Hello", 10.0, 10.0)));
    text.parent = Some(root_id);
    doc.apply(Operation::create_node(text)).unwrap();

    let mut baseline = PathData::new();
    baseline.move_to(2.0, 3.0).quad_to(20.0, -4.0, 38.0, 3.0);
    let mut text_path = TextPathNode::new(baseline, "Around the bend");
    text_path.start = TextPathStart::new(0, 0.25).expect("valid start");
    text_path.alignment = TextPathAlignment::Center;
    text_path.direction = TextPathDirection::Reverse;
    text_path.side = TextPathSide::Flipped;
    let mut text_path = CanvasNode::new(NodeData::TextPath(text_path));
    text_path.parent = Some(root_id);
    doc.apply(Operation::create_node(text_path)).unwrap();

    // Pull the per-node Values exactly as write_project_tree does.
    let doc_val = serde_json::to_value(&doc).unwrap();
    let nodes: Vec<Value> = doc_val["scene"]["nodes"]
        .as_object()
        .expect("scene.nodes object")
        .values()
        .cloned()
        .collect();
    assert_eq!(nodes.len(), 4);

    let back = round_trip(&nodes);
    assert_same_nodes(&nodes, &back);

    // And the reconstructed values must deserialize back into real CanvasNodes.
    for v in &back {
        serde_json::from_value::<CanvasNode>(v.clone()).expect("reconstructed node deserializes");
    }
}

/// A pure-translation `transform` prints as readable `x`/`y` (not the raw 6-array)
/// and folds back exactly — the position sugar is lossless.
#[test]
fn translation_transform_sugars_to_x_y_and_round_trips() {
    let nodes = vec![json!({
        "type": "vector", "id": "VEC00000000000000000000000", "parent": null, "index": 1.0,
        "name": "Box", "transform": [1.0, 0.0, 0.0, 1.0, 24.0, 36.0]
    })];
    let tree = tree_from_nodes(&nodes).unwrap();
    let text = print_doc("Box", &tree.root);
    assert!(text.contains("x={24.0}"), "x sugar missing:\n{text}");
    assert!(text.contains("y={36.0}"), "y sugar missing:\n{text}");
    assert!(
        !text.contains("transform="),
        "raw transform leaked:\n{text}"
    );

    assert_same_nodes(&nodes, &round_trip(&nodes));
}

/// An identity translation still round-trips (sugars to `x={0} y={0}`), so a
/// node sitting at the origin keeps its `transform` byte-for-byte.
#[test]
fn identity_translation_round_trips() {
    let nodes = vec![json!({
        "type": "group", "id": "ROOT0000000000000000000000", "parent": null, "index": 1.0,
        "name": "Card", "transform": [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
    })];
    assert_same_nodes(&nodes, &round_trip(&nodes));
}

/// A transform carrying rotation/scale (non-identity 2×2) is NOT sugared — it
/// stays a verbatim `transform={[…]}` array and round-trips unchanged.
#[test]
fn non_translation_transform_is_left_verbatim() {
    let nodes = vec![json!({
        "type": "vector", "id": "VEC00000000000000000000000", "parent": null, "index": 1.0,
        "name": "Spun", "transform": [0.0, -1.0, 1.0, 0.0, 5.0, 7.0]
    })];
    let tree = tree_from_nodes(&nodes).unwrap();
    let text = print_doc("Spun", &tree.root);
    assert!(
        text.contains("transform="),
        "rotation should stay a transform array:\n{text}"
    );
    assert!(
        !text.contains("x={"),
        "rotation must not sugar to x/y:\n{text}"
    );

    assert_same_nodes(&nodes, &round_trip(&nodes));
}

#[test]
fn prototype_reaction_and_constraints_roundtrip_via_fnx() {
    // Ensures declarative format (.fnx) covers key Figma features: prototype
    // (reactions/triggers/actions) + resize (constraints) + they survive
    // the element <-> doc projection losslessly.
    use fanta_doc::id::{AnimationClipId, ReactionId, VariableId};
    use fanta_doc::node::{
        Action, ConstraintH, ConstraintV, Constraints, Direction, Easing, PrototypeAnimation,
        Reaction, Transition, TransitionStyle, Trigger,
    };
    use fanta_doc::value::VarValue;
    use fanta_doc::{CanvasNode, GroupNode, NodeData, Operation, Transform2D};

    let mut doc = fanta_doc::Doc::new();
    let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([200.0, 200.0]),
        ..Default::default()
    }));
    frame.transform = Transform2D::translation(10.0, 20.0);
    frame.constraints = Some(Constraints {
        horizontal: ConstraintH::LeftRight,
        vertical: ConstraintV::TopBottom,
    });
    let animation_clip = AnimationClipId::from_u128(88);
    frame.reactions.push(Reaction {
        id: ReactionId::from_u128(1),
        trigger: Trigger::Click,
        action: Action::Navigate {
            to: fanta_doc::id::NodeId::from_u128(999),
        },
        extra_actions: Vec::new(),
        transition: Some(Transition {
            style: TransitionStyle::SlideIn {
                direction: Direction::Left,
            },
            duration_ms: 250,
            easing: Easing::EaseOut,
        }),
        animation: Some(PrototypeAnimation {
            clip: animation_clip,
            delay_ms: 75,
        }),
    });
    // additional prototype trigger for fidelity
    frame.reactions.push(Reaction {
        id: ReactionId::from_u128(2),
        trigger: Trigger::WhilePressing,
        action: Action::SetVariable {
            variable: VariableId::from_u128(42),
            value: VarValue::Boolean { value: true },
        },
        extra_actions: Vec::new(),
        transition: None,
        animation: None,
    });
    // UpdateVariant for component switching in prototype (Figma fidelity)
    frame.reactions.push(Reaction {
        id: ReactionId::from_u128(3),
        trigger: Trigger::Click,
        action: Action::UpdateVariant {
            component: fanta_doc::id::ComponentId::from_u128(7),
            variant: "Large".to_string(),
        },
        extra_actions: Vec::new(),
        transition: None,
        animation: None,
    });
    // meta for comments, annotations, measurements, custom data (Figma etc. extensibility)
    frame.meta = serde_json::json!({"comment": "review me", "measurement": {"dist": 42}, "annotation": "note"});
    doc.apply(Operation::create_node(frame)).unwrap();

    // Extract subtree nodes as Value for fnx roundtrip.
    let nodes: Vec<serde_json::Value> = doc
        .scene
        .roots()
        .iter()
        .flat_map(|&r| doc.scene.descendants_of(r))
        .filter_map(|id| doc.scene.get(id))
        .map(|n| serde_json::to_value(n).unwrap())
        .collect();

    let tree = tree_from_nodes(&nodes).expect("decompose prototype node");
    let source = print_doc("Prototype", &tree.root);
    assert!(
        source.contains("\"animation\": {\"clip\":"),
        "prototype animation binding should be projected into FNX source:\n{source}"
    );
    assert!(
        source.contains("\"delay_ms\": 75"),
        "prototype animation delay should be projected into FNX source:\n{source}"
    );

    let back = round_trip(&nodes);
    assert_same_nodes(&nodes, &back);

    // Spot-check key fields survived.
    let restored = &back[0];
    assert!(
        restored.get("reactions").is_some(),
        "reactions roundtripped"
    );
    assert!(
        restored.get("constraints").is_some(),
        "constraints roundtripped"
    );
    assert!(
        restored.get("meta").is_some() && restored["meta"]["comment"] == "review me",
        "meta (comments/annotations/measurements) roundtripped"
    );
    // UpdateVariant action (component prototype) should survive declarative format
    let rx = restored
        .get("reactions")
        .and_then(|v| v.as_array())
        .expect("reactions array");
    assert!(
        rx.iter()
            .any(|r| r["action"]["kind"] == "update_variant" && r["action"]["variant"] == "Large"),
        "UpdateVariant roundtripped in fnx"
    );
    let expected_clip = serde_json::to_value(animation_clip).expect("serialize animation clip id");
    assert!(
        rx.iter().any(|reaction| {
            reaction["animation"]["clip"] == expected_clip
                && reaction["animation"]["delay_ms"] == 75
        }),
        "prototype animation binding roundtripped in fnx"
    );
}

#[test]
fn video_timeline_with_prototype_reaction_roundtrips_in_fnx() {
    // Ensures declarative format supports timeline (Video) + prototype (reaction) + meta (annotations).
    let video = json!({
        "type": "video",
        "id": "VID00000000000000000000000",
        "parent": "ROOT000000000000000000000",
        "index": 0.0,
        "name": "Clip",
        "asset": "AST00000000000000000000000",
        "natural_size": [640, 360],
        "local_size": [320.0, 180.0],
        "time_range_us": [0, 10000000],
        "reactions": [{
            "id": "R0000000000000000000000000001",
            "trigger": {"kind": "click"},
            "action": {"kind": "navigate", "to": "FRM00000000000000000000000"}
        }],
        "meta": {"annotation": "timeline note", "measurement": 100}
    });
    let nodes = vec![video];
    let back = round_trip(&nodes);
    assert_same_nodes(&nodes, &back);
    assert!(
        back[0].get("reactions").is_some(),
        "video with prototype reaction"
    );
    assert!(back[0].get("time_range_us").is_some(), "timeline preserved");
}

/// Hand-/agent-authored sources reach for the JSX idiom `width`/`height`
/// instead of the doc model's `clip_size`/`local_size`. The parser folds them
/// so a `<Text width={…} height={…}>` reassembles instead of failing with
/// "missing field `local_size`", and a `<Frame width={…} height={…}>` keeps
/// its box as `clip_size` instead of silently dropping it.
#[test]
fn width_height_sugar_folds_into_canonical_size_fields() {
    let source = r#"
export default function Card() {
  return (
    <Frame name="Box" width={640.0} height={480.0}>
      <Text content="Hi" width={120.0} height={32.0} x={8.0} y={8.0} />
      <Text content="Explicit wins" local_size={[50.0, 20.0]} width={999.0} height={999.0} />
    </Frame>
  );
}
"#;
    let root = parse_doc(source).expect("parse");
    assert_eq!(root.attrs.get("clip_size"), Some(&json!([640.0, 480.0])));
    assert!(!root.attrs.contains_key("width"));
    assert!(!root.attrs.contains_key("height"));
    let text = &root.children[0];
    assert_eq!(text.attrs.get("local_size"), Some(&json!([120.0, 32.0])));
    assert!(!text.attrs.contains_key("width"));
    let explicit = &root.children[1];
    assert_eq!(explicit.attrs.get("local_size"), Some(&json!([50.0, 20.0])));
    assert!(!explicit.attrs.contains_key("width"));
}

// ---------------------------------------------------------------------------
// Parse errors: line/column anchoring + excerpt + caret
// ---------------------------------------------------------------------------

#[test]
fn parse_errors_carry_line_and_column() {
    let source = r#"export default function Home() {
  return (
    <Frame name="Root">
      <Text content="Hi" style=oops />
    </Frame>
  );
}
"#;
    let error = parse_doc(source).expect_err("style value is not quoted or braced");
    let message = error.to_string();
    // `style=` sits on line 4; the caret lands on the bad value.
    assert!(
        message.contains("line 4, column 32: expected attribute value"),
        "got: {message}"
    );
    assert!(message.contains('^'), "caret missing: {message}");
    assert!(
        message.contains("style=oops"),
        "excerpt should show the offending span: {message}"
    );
}

#[test]
fn duplicate_attribute_error_points_at_the_second_key() {
    let source = "<Frame name=\"A\" name=\"B\" />";
    let error = parse_doc(source).expect_err("duplicate attribute");
    let message = error.to_string();
    assert!(
        message.contains("line 1, column 17: duplicate attribute name"),
        "got: {message}"
    );
}

#[test]
fn bad_json_value_error_maps_to_absolute_position() {
    // The unquoted key inside the braced value is invalid strict JSON. serde
    // reports it value-relative; the parser maps it back to source line/col.
    let source = "<Frame name=\"A\"\n       auto_layout={{mode: \"horizontal\"}} />";
    let error = parse_doc(source).expect_err("unquoted JSON key");
    let message = error.to_string();
    assert!(
        message.contains("line 2, column"),
        "should anchor on the attribute's line: {message}"
    );
    assert!(
        message.contains("bad value: key must be a string"),
        "serde's diagnosis survives: {message}"
    );
    assert!(
        !message.contains(" at line "),
        "value-relative serde coordinates must be stripped: {message}"
    );
}

// ---------------------------------------------------------------------------
// Shape sugar: <Rect> / <Ellipse> at the text boundary
// ---------------------------------------------------------------------------

/// THE seam drift guard for rectangles: the sugar's generated `path` must be
/// `Value`-equal to the serde JSON of the REAL `fanta_doc::PathData::rect` —
/// fanta-fnx mirrors the constructor without depending on fanta-doc, and this
/// dev-dependency test is what keeps the two from drifting.
#[test]
fn rect_tag_desugars_to_canonical_vector_path() {
    let source =
        r#"<Rect name="Box" width={100.0} height={40.0} corner_radius={4.0} opacity={0.5} />"#;
    let el = parse_doc(source).expect("parse rect sugar");
    assert_eq!(el.tag, "Vector", "sugar must desugar to a canonical Vector");
    assert_eq!(
        el.attrs.get("path"),
        Some(&serde_json::to_value(fanta_doc::PathData::rect(0.0, 0.0, 100.0, 40.0)).unwrap()),
        "generated path must equal the doc constructor's serde JSON exactly"
    );
    assert!(!el.attrs.contains_key("width"), "width must be consumed");
    assert!(!el.attrs.contains_key("height"), "height must be consumed");
    assert!(
        !el.attrs.contains_key("local_size"),
        "shape sugar must not set local_size (on a Vector it is the viewport clip)"
    );
    // Every other attribute rides along untouched.
    assert_eq!(el.attrs.get("corner_radius"), Some(&json!(4.0)));
    assert_eq!(el.attrs.get("opacity"), Some(&json!(0.5)));
    assert_eq!(el.attrs.get("name"), Some(&json!("Box")));
}

/// Same drift guard for ellipses: four kappa cubics, pinned against the real
/// `fanta_doc::PathData::ellipse(w/2, h/2, w/2, h/2)`.
#[test]
fn ellipse_tag_desugars_to_canonical_vector_path() {
    let source = r#"<Ellipse name="Dot" width={100.0} height={40.0} />"#;
    let el = parse_doc(source).expect("parse ellipse sugar");
    assert_eq!(el.tag, "Vector");
    let (w, h) = (100.0f64, 40.0f64);
    assert_eq!(
        el.attrs.get("path"),
        Some(
            &serde_json::to_value(fanta_doc::PathData::ellipse(
                w / 2.0,
                h / 2.0,
                w / 2.0,
                h / 2.0
            ))
            .unwrap()
        ),
        "generated ellipse path must equal the doc constructor's serde JSON exactly"
    );
    assert!(!el.attrs.contains_key("width"));
    assert!(!el.attrs.contains_key("height"));
}

/// Every entry of the sugar-tag table must actually desugar — pins the
/// `model::SUGAR_TAGS` list to the `sugar::ShapeKind` behavior.
#[test]
fn every_sugar_tag_desugars_to_a_vector() {
    for tag in crate::model::SUGAR_TAGS {
        let source = format!("<{tag} width={{10.0}} height={{10.0}} />");
        let el = parse_doc(&source).expect("sugar tag parses");
        assert_eq!(el.tag, "Vector", "sugar tag {tag} must desugar");
        assert!(el.attrs.contains_key("path"), "{tag} must generate a path");
    }
}

/// Print→parse→print is a fixpoint for sugar spellings by construction: the
/// print-side recognizer regenerates the path and demands `Value` equality,
/// so whatever prints as `<Rect>`/`<Ellipse>` parses back to the identical
/// canonical tree and prints identically again. No float tolerance anywhere.
#[test]
fn print_parse_print_is_stable_for_sugar_tags() {
    let source = r#"
export default function Shapes() {
  return (
    <Frame name="Shapes">
      <Rect name="Box" width={120.0} height={80.0} x={4.0} y={6.0} />
      <Ellipse name="Dot" width={30.0} height={30.0} />
    </Frame>
  );
}
"#;
    let root = parse_doc(source).expect("parse authored sugar");
    let printed = print_doc("Shapes", &root);
    assert!(
        printed.contains("<Rect "),
        "rect re-sugars on print:\n{printed}"
    );
    assert!(
        printed.contains("<Ellipse "),
        "ellipse re-sugars on print:\n{printed}"
    );
    assert!(
        !printed.contains("path="),
        "sugar shapes must not print their generated path:\n{printed}"
    );

    let reparsed = parse_doc(&printed).expect("reparse printed sugar");
    assert_eq!(
        root, reparsed,
        "sugar round-trip changed the canonical tree"
    );
    assert_eq!(
        printed,
        print_doc("Shapes", &reparsed),
        "print must be a fixpoint over parse"
    );
}

/// A canonical vector node whose path exactly matches the rect generator
/// re-prints as `<Rect width height>` — and stays lossless through the full
/// node round-trip.
#[test]
fn vector_matching_rect_shape_resugars_on_print() {
    let mut node = json!({
        "type": "vector", "id": "VEC00000000000000000000000", "parent": null, "index": 1.0,
        "name": "Box",
        "fills": [{ "kind": "solid", "color": { "r": 0, "g": 0, "b": 0, "a": 255 } }]
    });
    node["path"] = serde_json::to_value(fanta_doc::PathData::rect(0.0, 0.0, 120.0, 80.0)).unwrap();
    let nodes = vec![node];

    let tree = tree_from_nodes(&nodes).unwrap();
    let text = print_doc("Box", &tree.root);
    assert!(
        text.contains("<Rect "),
        "rect shape should re-sugar:\n{text}"
    );
    assert!(text.contains("width={120.0}"), "implied width:\n{text}");
    assert!(text.contains("height={80.0}"), "implied height:\n{text}");
    assert!(!text.contains("path="), "path must not leak:\n{text}");
    assert!(
        !text.contains("<Vector"),
        "no canonical spelling left:\n{text}"
    );

    assert_same_nodes(&nodes, &round_trip(&nodes));
}

/// A `.fig`-imported rectangle — and any rectangle whose viewport the doc
/// backfilled on load — carries a `local_size` equal to its own extent. That
/// viewport crops nothing, so it re-sugars and rides along as its own
/// attribute. Without this, a `<Rect>` degraded to raw path data on the first
/// reload and every rectangle in the file diffed on the next save.
#[test]
fn vector_with_viewport_equal_to_its_extent_resugars() {
    let mut node = json!({
        "type": "vector", "id": "VEC00000000000000000000000", "parent": null, "index": 1.0,
        "name": "Imported", "local_size": [120.0, 80.0]
    });
    node["path"] = serde_json::to_value(fanta_doc::PathData::rect(0.0, 0.0, 120.0, 80.0)).unwrap();
    let nodes = vec![node];

    let tree = tree_from_nodes(&nodes).unwrap();
    let text = print_doc("Imported", &tree.root);
    assert!(text.contains("<Rect "), "must re-sugar:\n{text}");
    assert!(!text.contains("path="), "path must not leak:\n{text}");
    assert!(
        text.contains("local_size={[120.0, 80.0]}"),
        "the viewport must ride along verbatim:\n{text}"
    );

    assert_same_nodes(&nodes, &round_trip(&nodes));
    let reparsed = parse_doc(&text).expect("reparse");
    assert_eq!(tree.root, reparsed, "sugar round-trip changed the tree");
}

/// A viewport that is NOT the shape's own extent genuinely crops it (a stroke
/// thickened past the authored box is the usual case). No `<Rect>` spelling
/// can express that, so the vector must stay verbatim rather than silently
/// lose its clip.
#[test]
fn vector_with_cropping_viewport_does_not_resugar() {
    let mut node = json!({
        "type": "vector", "id": "VEC00000000000000000000000", "parent": null, "index": 1.0,
        "name": "Clipped", "local_size": [60.0, 80.0]
    });
    node["path"] = serde_json::to_value(fanta_doc::PathData::rect(0.0, 0.0, 120.0, 80.0)).unwrap();
    let nodes = vec![node];

    let tree = tree_from_nodes(&nodes).unwrap();
    let text = print_doc("Clipped", &tree.root);
    assert!(
        text.contains("<Vector") && text.contains("path="),
        "a cropping viewport must stay canonical:\n{text}"
    );
    assert!(!text.contains("<Rect"), "must not re-sugar:\n{text}");

    assert_same_nodes(&nodes, &round_trip(&nodes));
}

/// The two spellings of the same shape are the same node: a sidecar built
/// against `<Vector path={…rect…}>` reconciles against the `<Rect width
/// height>` spelling without minting or retiring a single id, because
/// fingerprints are taken POST-parse where both spell tag `Vector`.
#[test]
fn rect_spelling_flip_keeps_identity() {
    let rect_path = serde_json::to_value(fanta_doc::PathData::rect(0.0, 0.0, 100.0, 40.0)).unwrap();
    let vector_source = format!(
        "export default function Card() {{\n  return (\n    <Frame name=\"Card\">\n      <Vector name=\"Box\" path={{{path}}} />\n    </Frame>\n  );\n}}\n",
        path = serde_json::to_string(&rect_path).unwrap()
    );
    let empty = FnxSidecar {
        root_parent: None,
        ids: Vec::new(),
    };
    let sidecar = reconcile_sidecar(&vector_source, &empty, sequential_minter()).unwrap();
    assert_eq!(sidecar.ids.len(), 2);
    assert_eq!(sidecar.ids[1].tag.as_deref(), Some("Vector"));

    let rect_source = "export default function Card() {\n  return (\n    <Frame name=\"Card\">\n      <Rect name=\"Box\" width={100.0} height={40.0} />\n    </Frame>\n  );\n}\n";
    let reconciled = reconcile_sidecar(rect_source, &sidecar, || {
        unreachable!("a spelling flip must not mint ids")
    })
    .unwrap();
    assert_eq!(
        reconciled, sidecar,
        "sugar spelling must be invisible to identity"
    );

    // Both spellings decode to the identical node set.
    assert_same_nodes(
        &decode_subtree(&vector_source, &sidecar).unwrap(),
        &decode_subtree(rect_source, &reconciled).unwrap(),
    );
}

#[test]
fn explicit_path_on_rect_is_an_error() {
    let source = r#"<Rect name="Box" width={10.0} height={10.0} path={{"segments": []}} />"#;
    let error = parse_doc(source).expect_err("explicit path on sugar tag");
    let message = error.to_string();
    assert!(
        message.contains("`<Rect name=\"Box\">` generates its own path"),
        "message must name the element: {message}"
    );
    assert!(
        message.contains("use `<Vector>` for explicit path data"),
        "message must point at the canonical escape hatch: {message}"
    );
}

#[test]
fn rect_missing_size_is_an_error() {
    let source = r#"<Rect name="Box" width={10.0} />"#;
    let error = parse_doc(source).expect_err("height missing");
    let message = error.to_string();
    assert!(
        message.contains("`<Rect name=\"Box\">` requires numeric `width` and `height`"),
        "message must name the element and the missing attributes: {message}"
    );

    // Non-numeric size is just as hard an error as a missing one.
    let source = r#"<Ellipse width="wide" height={10.0} />"#;
    let error = parse_doc(source).expect_err("non-numeric width");
    assert!(
        error
            .to_string()
            .contains("`<Ellipse>` requires numeric `width` and `height`"),
        "got: {error}"
    );
}

/// B5: `x`/`y` sugar used to silently DROP an explicit `transform`, losing
/// rotation. The combination is now a hard error that names the element.
#[test]
fn x_y_with_explicit_transform_is_an_error() {
    let source =
        r#"<Vector name="Spun" x={5.0} y={6.0} transform={[0.0, -1.0, 1.0, 0.0, 5.0, 7.0]} />"#;
    let error = parse_doc(source).expect_err("x/y alongside transform");
    let message = error.to_string();
    assert!(
        message.contains("`<Vector name=\"Spun\">`"),
        "message must name the element: {message}"
    );
    assert!(
        message.contains("`x`/`y` conflict with an explicit `transform`"),
        "got: {message}"
    );
}

/// Source-mirror fixture: a wrapper comment, a Frame parent, and one authored
/// `<Rect>` child — with the sidecar spelling the POST-parse tags.
fn authored_rect_fixture() -> (&'static str, FnxSidecar) {
    let source = "// author wrapper stays\nexport default function Card() {\n  return (\n    <Frame name=\"Card\">\n      <Rect name=\"Box\" width={100.0} height={40.0} />\n    </Frame>\n  );\n}\n";
    let sidecar = FnxSidecar {
        root_parent: None,
        ids: vec![
            IdEntry {
                id: "root".into(),
                index: json!(1.0),
                tag: Some("Frame".into()),
                name: Some("Card".into()),
                parent_index: None,
            },
            IdEntry {
                id: "box".into(),
                index: json!(1),
                tag: Some("Vector".into()),
                name: Some("Box".into()),
                parent_index: Some(0),
            },
        ],
    };
    (source, sidecar)
}

/// A canvas resize reaches the mirror as a canonical Vector whose path is the
/// regenerated rect at the new size. The patch must land in sugar space:
/// `width` updated in place, the author's `<Rect>` spelling (and everything
/// around it) untouched.
#[test]
fn canvas_resize_patches_width_on_authored_rect() {
    let (source, sidecar) = authored_rect_fixture();
    let mut mirror = FnxSourceMirror::from_source(source, &sidecar).unwrap();

    let root = parse_doc(source).unwrap();
    let previous = root.children[0].clone();
    let mut next = previous.clone();
    next.attrs.insert(
        "path".into(),
        serde_json::to_value(fanta_doc::PathData::rect(0.0, 0.0, 150.0, 40.0)).unwrap(),
    );
    let changed = mirror.patch_element_delta("box", &previous, &next).unwrap();
    assert!(changed);

    let patched = mirror.render();
    assert!(
        patched.contains("<Rect name=\"Box\" width={150.0} height={40.0} />"),
        "resize must patch width in sugar space, keeping author order:\n{patched}"
    );
    assert!(
        !patched.contains("<Vector"),
        "the sugar spelling must survive a resize:\n{patched}"
    );
    assert!(patched.starts_with("// author wrapper stays\n"));
    assert!(patched.contains("<Frame name=\"Card\">"));
}

/// The moment the shape stops matching its generator (a free-path edit), the
/// sugar spelling can no longer express the node: that ONE tag reprints as a
/// full canonical `<Vector path={…}/>`, mirroring the tag-change fallback.
#[test]
fn path_edit_on_authored_rect_falls_back_to_canonical_vector() {
    let (source, sidecar) = authored_rect_fixture();
    let mut mirror = FnxSourceMirror::from_source(source, &sidecar).unwrap();

    let root = parse_doc(source).unwrap();
    let previous = root.children[0].clone();
    let mut next = previous.clone();
    next.attrs.insert(
        "path".into(),
        json!({ "segments": [
            { "op": "move", "to": [0.0, 0.0] },
            { "op": "line", "to": [10.0, 50.0] },
            { "op": "close" }
        ]}),
    );
    let changed = mirror.patch_element_delta("box", &previous, &next).unwrap();
    assert!(changed);

    let patched = mirror.render();
    assert!(
        patched.contains("<Vector name=\"Box\" path={"),
        "free path must reprint canonically:\n{patched}"
    );
    assert!(
        !patched.contains("<Rect"),
        "sugar spelling retired:\n{patched}"
    );
    // Only that one tag was touched.
    assert!(patched.starts_with("// author wrapper stays\n"));
    assert!(patched.contains("<Frame name=\"Card\">"));
}

/// Legacy generated imports upgrade one generation at a time, keeping newly
/// persisted tags available to TypeScript tooling without duplicating imports.
#[test]
fn canonicalizer_upgrades_the_legacy_import_line_in_place() {
    for legacy_import in [
        "import { AiArtifact, Audio, Boolean, Ellipse, Embed, Frame, Image, Instance, Model3D, NodeGraph, Rect, Text, Vector, Video } from \"../../fnx\";",
        "import { AiArtifact, Audio, Boolean, Embed, Frame, Image, Instance, Model3D, NodeGraph, Text, Vector, Video } from \"../../fnx\";",
    ] {
        let legacy_source = format!(
            "{}\n{}\n{legacy_import}\n// @generated fanta source\nexport default () => <Frame name=\"Card\" />;\n",
            crate::canonicalize::JSX_RUNTIME_PRAGMA,
            crate::canonicalize::JSX_FACTORY_PRAGMA,
        );

        let upgraded =
            canonicalize_legacy_source(&legacy_source).expect("legacy import line should upgrade");
        assert!(upgraded.contains(crate::canonicalize::FNX_TAG_IMPORT));
        assert!(
            !upgraded.contains(legacy_import),
            "old import replaced, not duplicated:\n{upgraded}"
        );
        assert_eq!(upgraded.matches("from \"../../fnx\"").count(), 1);
        assert_eq!(canonicalize_legacy_source(&upgraded), None, "idempotent");
    }
}

#[test]
fn long_single_line_error_excerpt_is_windowed() {
    // Printed .fnx puts a whole node on one line; the excerpt must window
    // around the caret instead of echoing thousands of characters.
    let mut source = String::from("<Frame name=\"Root\" ");
    for index in 0..80 {
        source.push_str(&format!("attr_{index}={{{index}}} "));
    }
    source.push_str("bad=oops />");
    let error = parse_doc(&source).expect_err("bad attribute value");
    let message = error.to_string();
    let excerpt_line = message.lines().nth(1).expect("excerpt line");
    assert!(
        excerpt_line.chars().count() < 160,
        "excerpt must be windowed, got {} chars",
        excerpt_line.chars().count()
    );
    assert!(
        excerpt_line.starts_with("  … "),
        "leading ellipsis: {excerpt_line:?}"
    );
}

// ---------------------------------------------------------------------------
// Name-based references: component="Button" (B3) + $Collection/Name paths (B4)
// ---------------------------------------------------------------------------

const BUTTON_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const CHIP_ID: &str = "01BX5ZZKBKACTAV9WEVGEMMVRZ";
const VAR_BG: &str = "01BX5ZZKBKZAAAAAAAAAAAAAA0";
const VAR_BORDER: &str = "01BX5ZZKBKZAAAAAAAAAAAAAA1";
const VAR_HEADING: &str = "01BX5ZZKBKZAAAAAAAAAAAAAA2";

fn ref_table(emit_names: bool) -> RefTable {
    let mut table = RefTable::new(emit_names);
    table.insert_component("Button", BUTTON_ID);
    table.insert_component("Chip", CHIP_ID);
    table.insert_variable("Theme/bg", VAR_BG);
    table.insert_variable("Theme/border", VAR_BORDER);
    table.insert_variable("Type/Heading", VAR_HEADING);
    table
}

fn instance_source(component: &str, bindings: &str) -> String {
    format!(
        "export default function Home() {{\n  return (\n    <Frame name=\"Home\">\n      \
         <Instance name=\"Btn\" component={component:?} local_size={{[120.0, 40.0]}} \
         bindings={{{bindings}}} />\n    </Frame>\n  );\n}}\n"
    )
}

#[test]
fn component_name_resolves_to_ulid() {
    let src = instance_source("Button", "[]");
    let root = parse_doc_with(&src, &ref_table(false)).unwrap();
    assert_eq!(root.children[0].attrs["component"], json!(BUTTON_ID));
}

#[test]
fn component_ulid_passthrough() {
    // A raw ULID is the forever escape hatch — even one the table has never
    // heard of (and even spelled lowercase, which the ulid decoder accepts).
    let unknown = "01hqqqqqqqqqqqqqqqqqqqqqqq";
    let src = instance_source(unknown, "[]");
    let root = parse_doc_with(&src, &ref_table(false)).unwrap();
    assert_eq!(root.children[0].attrs["component"], json!(unknown));
}

#[test]
fn unresolved_component_name_errors_with_suggestion() {
    let src = instance_source("Buton", "[]");
    let error = parse_doc_with(&src, &ref_table(false)).unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("unknown component \"Buton\"")
            && message.contains("<Instance name=\"Btn\">")
            && message.contains("did you mean \"Button\"?"),
        "unhelpful error: {message}"
    );
}

#[test]
fn ambiguous_component_name_errors_on_parse() {
    let mut table = ref_table(false);
    table.insert_component("Button", CHIP_ID); // second id claims "Button"
    let src = instance_source("Button", "[]");
    let message = parse_doc_with(&src, &table).unwrap_err().to_string();
    assert!(
        message.contains("ambiguous component name \"Button\"")
            && message.contains("use the component id"),
        "unhelpful error: {message}"
    );
}

#[test]
fn ambiguous_component_name_never_prints() {
    let mut table = ref_table(true);
    table.insert_component("Button", CHIP_ID); // "Button" now names two ids
    let nodes = vec![
        json!({
            "type": "group", "id": "ROOT0000000000000000000000", "parent": null, "index": 1.0,
            "name": "Home",
        }),
        json!({
            "type": "instance", "id": "INST0000000000000000000000",
            "parent": "ROOT0000000000000000000000", "index": 1.0, "name": "Btn",
            "component": BUTTON_ID, "local_size": [120.0, 40.0],
        }),
    ];
    let (text, _) = encode_subtree_with(&nodes, "Home", &table).unwrap();
    assert!(
        text.contains(BUTTON_ID) && !text.contains("component=\"Button\""),
        "an ambiguous name leaked into print: {text}"
    );
}

/// Drift pin for the B4 map sugar: the desugared pair array must be VALUE-
/// identical to what a real `fanta_doc::CanvasNode` with the same bindings
/// serializes — key names, `index` presence (including index 0), pair order.
#[test]
fn bindings_map_sugar_round_trips_to_pair_array() {
    use fanta_doc::{BoundProp, CanvasNode, Color, NodeData, VectorNode};

    let mut node = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        10.0,
        10.0,
        Color::WHITE,
    )));
    let var = |id: &str| serde_json::from_value::<fanta_doc::id::VariableId>(json!(id)).unwrap();
    node.bindings
        .insert(BoundProp::FillColor { index: 0 }, var(VAR_BG));
    node.bindings
        .insert(BoundProp::StrokeColor { index: 1 }, var(VAR_BORDER));
    node.bindings.insert(BoundProp::TextStyle, var(VAR_HEADING));
    let expected = serde_json::to_value(&node).unwrap()["bindings"].clone();

    let src = instance_source(
        BUTTON_ID,
        r#"{"text_style": "$Type/Heading", "stroke_color:1": "$Theme/border", "fill_color": "$Theme/bg"}"#,
    );
    let root = parse_doc_with(&src, &ref_table(false)).unwrap();
    assert_eq!(
        root.children[0].attrs["bindings"], expected,
        "map sugar must desugar to the doc's exact pair-array serialization"
    );
}

#[test]
fn bindings_array_with_dollar_values_resolves() {
    // Hand-edited pair array: only the `$…` value strings resolve; everything
    // else stays verbatim.
    let src = instance_source(
        BUTTON_ID,
        r#"[[{"prop": "fill_color", "index": 0}, "$Theme/bg"]]"#,
    );
    let root = parse_doc_with(&src, &ref_table(false)).unwrap();
    assert_eq!(
        root.children[0].attrs["bindings"],
        json!([[{"prop": "fill_color", "index": 0}, VAR_BG]]),
    );
}

#[test]
fn unresolved_variable_path_errors_with_suggestion() {
    let src = instance_source(BUTTON_ID, r#"{"fill_color": "$Theme/bgg"}"#);
    let message = parse_doc_with(&src, &ref_table(false))
        .unwrap_err()
        .to_string();
    assert!(
        message.contains("unknown variable \"$Theme/bgg\"")
            && message.contains("did you mean \"$Theme/bg\"?"),
        "unhelpful error: {message}"
    );
}

#[test]
fn unknown_binding_property_errors_with_suggestion() {
    let src = instance_source(BUTTON_ID, r#"{"fil_color": "$Theme/bg"}"#);
    let message = parse_doc_with(&src, &ref_table(false))
        .unwrap_err()
        .to_string();
    assert!(
        message.contains("unknown binding property \"fil_color\"")
            && message.contains("did you mean \"fill_color\"?"),
        "unhelpful error: {message}"
    );
    // An index on a non-indexed property is a distinct, clear error.
    let src = instance_source(BUTTON_ID, r#"{"opacity:1": "$Theme/bg"}"#);
    let message = parse_doc_with(&src, &ref_table(false))
        .unwrap_err()
        .to_string();
    assert!(
        message.contains("\"opacity\"") && message.contains("does not take an index"),
        "unhelpful error: {message}"
    );
    // A bare name and an explicit `:0` collide on the same slot.
    let src = instance_source(
        BUTTON_ID,
        r#"{"fill_color": "$Theme/bg", "fill_color:0": "$Theme/border"}"#,
    );
    let message = parse_doc_with(&src, &ref_table(false))
        .unwrap_err()
        .to_string();
    assert!(
        message.contains("duplicate binding property \"fill_color\""),
        "unhelpful error: {message}"
    );
}

fn named_reference_nodes() -> Vec<Value> {
    vec![
        json!({
            "type": "group", "id": "ROOT0000000000000000000000", "parent": null, "index": 1.0,
            "name": "Home",
        }),
        json!({
            "type": "instance", "id": "INST0000000000000000000000",
            "parent": "ROOT0000000000000000000000", "index": 1.0, "name": "Btn",
            "component": BUTTON_ID, "local_size": [120.0, 40.0],
            "bindings": [[{"prop": "fill_color", "index": 0}, VAR_BG]],
        }),
    ]
}

#[test]
fn print_emits_names_only_when_emit_names() {
    let nodes = named_reference_nodes();
    let (quiet, _) = encode_subtree_with(&nodes, "Home", &ref_table(false)).unwrap();
    assert!(
        quiet.contains(BUTTON_ID) && quiet.contains(VAR_BG) && !quiet.contains('$'),
        "emit_names=false must keep raw ULIDs: {quiet}"
    );

    let (named, _) = encode_subtree_with(&nodes, "Home", &ref_table(true)).unwrap();
    assert!(
        named.contains("component=\"Button\"")
            && named.contains("bindings={{\"fill_color\": \"$Theme/bg\"}}")
            && !named.contains(BUTTON_ID)
            && !named.contains(VAR_BG),
        "emit_names=true must spell references readably: {named}"
    );
}

#[test]
fn bindings_array_stays_verbatim_when_any_entry_cannot_sugar() {
    // The second entry's variable has no path in the table → the WHOLE array
    // stays raw (all-or-nothing; no half-named arrays).
    let orphan = "01BX5ZZKBKZAAAAAAAAAAAAAA9";
    let mut nodes = named_reference_nodes();
    nodes[1]["bindings"] = json!([
        [{"prop": "fill_color", "index": 0}, VAR_BG],
        [{"prop": "opacity"}, orphan],
    ]);
    let (text, _) = encode_subtree_with(&nodes, "Home", &ref_table(true)).unwrap();
    assert!(
        text.contains(VAR_BG) && text.contains(orphan) && !text.contains("$Theme/bg"),
        "partial sugar leaked into a pair array: {text}"
    );
    // The component name still sugars independently of the bindings attr.
    assert!(text.contains("component=\"Button\""), "{text}");
}

#[test]
fn named_print_parse_print_is_a_fixpoint() {
    let nodes = named_reference_nodes();
    let table = ref_table(true);
    let (first, sidecar) = encode_subtree_with(&nodes, "Home", &table).unwrap();
    let decoded = decode_subtree_with(&first, &sidecar, &table).unwrap();
    assert_same_nodes(&nodes, &decoded);
    let root = parse_doc_with(&first, &table).unwrap();
    let second = print_doc_with("Home", &root, &table);
    assert_eq!(
        first, second,
        "print∘parse must be a fixpoint with names on"
    );
}

/// Regression pin: with no table (or the inert default), the codec's output
/// and acceptance are byte-identical to the pre-refs behavior — including for
/// sources whose `component` value is a non-ULID string it cannot resolve.
#[test]
fn default_table_behaves_exactly_like_no_table() {
    let nodes = sample();
    let tree = tree_from_nodes(&nodes).unwrap();
    let plain = print_doc("Card", &tree.root);
    let with_default = print_doc_with("Card", &tree.root, &RefTable::default());
    let with_quiet_table = print_doc_with("Card", &tree.root, &ref_table(false));
    assert_eq!(plain, with_default);
    assert_eq!(plain, with_quiet_table, "emit_names=false never rewrites");

    assert_eq!(
        parse_doc(&plain).unwrap(),
        parse_doc_with(&plain, &RefTable::default()).unwrap()
    );

    // The historical passthrough: a name-shaped component value survives a
    // default-table parse untouched (resolution needs an actual table).
    let src = instance_source("NotAUlid", "[]");
    let root = parse_doc(&src).unwrap();
    assert_eq!(root.children[0].attrs["component"], json!("NotAUlid"));
}

/// Drift pin against the REAL `fanta_doc::BoundProp`: variant serde tags,
/// which variants carry `index` (and that index 0 IS serialized), and the
/// declaration/Ord order the pair array is sorted by. The exhaustive `match`
/// makes adding a `BoundProp` variant a compile error here, forcing the
/// `BOUND_PROPS` mirror in refs.rs to be updated in the same change.
#[test]
fn bound_props_mirror_matches_fanta_doc() {
    use fanta_doc::BoundProp;

    fn _exhaustive_over_variants(prop: BoundProp) {
        match prop {
            BoundProp::FillColor { .. }
            | BoundProp::StrokeColor { .. }
            | BoundProp::StrokeWidth { .. }
            | BoundProp::CornerRadius
            | BoundProp::Opacity
            | BoundProp::Visible
            | BoundProp::TextContent
            | BoundProp::TextStyle
            | BoundProp::ClipWidth
            | BoundProp::ClipHeight => {}
        }
    }

    // One variant per mirror row, in the mirror's order.
    let variants = [
        BoundProp::FillColor { index: 0 },
        BoundProp::StrokeColor { index: 0 },
        BoundProp::StrokeWidth { index: 0 },
        BoundProp::CornerRadius,
        BoundProp::Opacity,
        BoundProp::Visible,
        BoundProp::TextContent,
        BoundProp::TextStyle,
        BoundProp::ClipWidth,
        BoundProp::ClipHeight,
    ];
    assert_eq!(variants.len(), crate::refs::BOUND_PROPS.len());
    let mut sorted = variants;
    sorted.sort();
    assert_eq!(
        sorted, variants,
        "mirror order must equal BoundProp's Ord (BTreeMap serialization) order"
    );
    for (variant, (name, indexed)) in variants.iter().zip(crate::refs::BOUND_PROPS) {
        let expected = if *indexed {
            json!({ "prop": name, "index": 0 })
        } else {
            json!({ "prop": name })
        };
        assert_eq!(
            serde_json::to_value(variant).unwrap(),
            expected,
            "serde shape drifted for {name}"
        );
    }
}

#[test]
fn source_mirror_patches_keep_reference_spellings() {
    use std::sync::Arc;

    let source = instance_source("Button", r#"{"fill_color": "$Theme/bg"}"#);
    let sidecar = FnxSidecar {
        root_parent: None,
        ids: vec![
            IdEntry {
                id: "ROOT0000000000000000000000".into(),
                index: json!(1.0),
                tag: Some("Frame".into()),
                name: Some("Home".into()),
                parent_index: None,
            },
            IdEntry {
                id: "INST0000000000000000000000".into(),
                index: json!(1.0),
                tag: Some("Instance".into()),
                name: Some("Btn".into()),
                parent_index: Some(0),
            },
        ],
    };
    let table = Arc::new(ref_table(true));
    let mut mirror =
        FnxSourceMirror::from_source_with(&source, &sidecar, Arc::clone(&table)).unwrap();
    let root = parse_doc_with(&source, &table).unwrap();

    // A geometry-only canvas patch leaves both reference spellings alone.
    let previous = root.children[0].clone();
    let mut next = previous.clone();
    next.attrs.insert("local_size".into(), json!([200.0, 40.0]));
    mirror
        .patch_element_delta("INST0000000000000000000000", &previous, &next)
        .unwrap();
    let patched = mirror.render();
    assert!(
        patched.contains("component=\"Button\"")
            && patched.contains("bindings={{\"fill_color\": \"$Theme/bg\"}}")
            && patched.contains("local_size={[200.0, 40.0]}"),
        "geometry patch disturbed reference spellings: {patched}"
    );

    // A component swap re-sugars to the NEW component's name (the id came
    // from the canvas as a raw ULID).
    let previous = next.clone();
    let mut next = previous.clone();
    next.attrs
        .insert("component".into(), Value::String(CHIP_ID.into()));
    mirror
        .patch_element_delta("INST0000000000000000000000", &previous, &next)
        .unwrap();
    let patched = mirror.render();
    assert!(
        patched.contains("component=\"Chip\"") && !patched.contains(CHIP_ID),
        "component swap should print the new name: {patched}"
    );

    // Re-binding to a new variable prints its `$path`, still in object form.
    let previous = next.clone();
    let mut next = previous.clone();
    next.attrs.insert(
        "bindings".into(),
        json!([[{"prop": "fill_color", "index": 0}, VAR_BORDER]]),
    );
    mirror
        .patch_element_delta("INST0000000000000000000000", &previous, &next)
        .unwrap();
    let patched = mirror.render();
    assert!(
        patched.contains("bindings={{\"fill_color\": \"$Theme/border\"}}"),
        "rebinding should print the new $path: {patched}"
    );
}

// ---------------------------------------------------------------------------
// Float fidelity across the text boundary
// ---------------------------------------------------------------------------

/// Floats that need all 17 significant digits — every `.fig`-imported
/// coordinate is one, since an `f32` widened to `f64` rarely has a shorter
/// exact spelling — must survive print → parse → print byte-identically.
///
/// They only do because `serde_json` is built with `float_roundtrip` here: its
/// default parser is accurate to within 1 ULP, so `21.762165069580078` used to
/// come back as `21.76216506958008` and the first save after an import
/// rewrote every line of the page with a value one ULP away from the one it
/// had just written.
#[test]
fn seventeen_digit_floats_survive_the_text_boundary() {
    let source = "export default function P() {\n  return (\n    \
                  <Frame name=\"P\" x={21.762165069580078} y={1234.5678901234567} \
                  clip_size={[935.6572265625001, 0.30000000000000004]} />\n  );\n}\n";
    let root = parse_doc(source).expect("parse");
    let printed = print_doc("P", &root);
    for spelling in [
        "x={21.762165069580078}",
        "y={1234.5678901234567}",
        "clip_size={[935.6572265625001, 0.30000000000000004]}",
    ] {
        assert!(
            printed.contains(spelling),
            "{spelling} must print back verbatim:\n{printed}"
        );
    }
    let reparsed = parse_doc(&printed).expect("reparse");
    assert_eq!(root, reparsed, "reparse moved a float");
    assert_eq!(
        printed,
        print_doc("P", &reparsed),
        "print must be a fixpoint over parse"
    );
}
