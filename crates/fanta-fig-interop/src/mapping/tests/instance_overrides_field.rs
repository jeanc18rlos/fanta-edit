//! Generic Field overrides — OV-1..OV-8 (§3.1 / §5 F1 of
//! docs/research/figma-parity-divergence.md): the renderable fields of a
//! `symbolOverrides` entry that the typed swap/text/fill/stroke/visible arms
//! don't consume are folded into ONE `OverrideValue::Field` per guidPath and
//! serde-merged onto the clone at expansion. One pinned test per divergence
//! id, each proving both the import (the Field object carries the expected
//! key) and the end-to-end apply (`expand_instance` lands it on the clone),
//! plus the snapshot-redundancy drop.

use super::*;

/// A SYMBOL master ("Card") with one RECTANGLE child ("Body", `guid (0,2)`,
/// extra fields appended), and an INSTANCE carrying one symbolOverride entry
/// addressing that child with the given override fields.
fn rect_master_instance(
    master_child_extra: Vec<(&str, KiwiValue)>,
    override_fields: Vec<(&str, KiwiValue)>,
) -> FigDocument {
    let mut child_fields = vec![
        ("guid", guid(0, 2)),
        ("parentIndex", parent_index(0, 1)),
        ("type", KiwiValue::Enum("RECTANGLE".into())),
        ("name", KiwiValue::String("Body".to_owned())),
        ("size", vector(100.0, 40.0)),
        (
            "fillPaints",
            KiwiValue::Array(vec![solid_paint(0.5, 0.5, 0.5, 1.0)]),
        ),
    ];
    child_fields.extend(master_child_extra);
    let mut ov_fields = vec![("guidPath", guid_path(0, 2))];
    ov_fields.extend(override_fields);
    doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("Card".to_owned())),
                ("size", vector(100.0, 40.0)),
            ],
        ),
        o("NodeChange", child_fields),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3)),
                ("type", KiwiValue::Enum("INSTANCE".into())),
                ("name", KiwiValue::String("Card instance".to_owned())),
                ("size", vector(100.0, 40.0)),
                (
                    "symbolData",
                    o(
                        "SymbolData",
                        vec![
                            ("symbolID", guid(0, 1)),
                            (
                                "symbolOverrides",
                                KiwiValue::Array(vec![o("NodeChange", ov_fields)]),
                            ),
                        ],
                    ),
                ),
            ],
        ),
    ])
}

/// The instance's Field-override JSON object, if one was imported.
fn field_override_of(doc: &Doc) -> Option<serde_json::Value> {
    let inst = doc
        .scene
        .roots()
        .iter()
        .flat_map(|r| doc.scene.descendants_of(*r))
        .find_map(|id| match doc.scene.get(id).map(|n| &n.data) {
            Some(NodeData::Instance(i)) => Some(i.clone()),
            _ => None,
        })?;
    inst.overrides.iter().find_map(|ov| match &ov.value {
        fanta_doc::node::OverrideValue::Field { value } => Some(value.clone()),
        _ => None,
    })
}

/// The expanded instance's vector descendant (the "Body" clone).
fn expanded_vector_node(doc: &Doc) -> CanvasNode {
    expand_only_instance(doc)
        .into_iter()
        .find(|e| matches!(e.node.data, NodeData::Vector(_)))
        .expect("the master's vector child expands")
        .node
}

#[test]
fn ov1_opacity_override_applies() {
    // A per-instance faded element: the override entry carries `opacity: 0.4`
    // while the master child is fully opaque. Previously dropped — the
    // instance rendered at master opacity.
    let fig = rect_master_instance(vec![], vec![("opacity", KiwiValue::Float(0.4))]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert!(report.override_fields_applied >= 1, "one Field key emitted");
    let field = field_override_of(&doc).expect("a Field override is imported");
    assert!(field.get("opacity").is_some(), "carries the opacity key");
    let body = expanded_vector_node(&doc);
    assert!(
        (body.opacity.get() - 0.4).abs() < 1e-4,
        "expansion applies the per-instance opacity, got {}",
        body.opacity.get()
    );
}

#[test]
fn ov2_effects_override_applies() {
    // A hover-card shadow change: the override entry carries an `effects` list
    // (a drop shadow + a background blur) the master child doesn't have. The
    // one Kiwi array splits into the doc's `effects` (shadows) + `blurs`.
    let shadow = o(
        "Effect",
        vec![
            ("type", KiwiValue::Enum("DROP_SHADOW".into())),
            ("color", color(0.0, 0.0, 0.0, 0.5)),
            ("radius", KiwiValue::Float(6.0)),
            ("spread", KiwiValue::Float(0.0)),
            ("offset", vector(0.0, 2.0)),
            ("visible", KiwiValue::Bool(true)),
        ],
    );
    let blur = o(
        "Effect",
        vec![
            ("type", KiwiValue::Enum("BACKGROUND_BLUR".into())),
            ("radius", KiwiValue::Float(10.0)),
            ("visible", KiwiValue::Bool(true)),
        ],
    );
    let fig = rect_master_instance(
        vec![],
        vec![("effects", KiwiValue::Array(vec![shadow, blur]))],
    );
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert!(report.override_fields_applied >= 2, "effects + blurs keys");
    let field = field_override_of(&doc).expect("a Field override is imported");
    assert!(field.get("effects").is_some(), "carries the effects key");
    assert!(field.get("blurs").is_some(), "carries the blurs key");
    let body = expanded_vector_node(&doc);
    assert_eq!(body.effects.len(), 1, "the drop shadow lands");
    assert_eq!(body.effects[0].blur, 6.0);
    assert_eq!(body.blurs.len(), 1, "the background blur lands");
    assert_eq!(body.blurs[0].kind, BlurKind::Background);
    assert_eq!(body.blurs[0].radius, 10.0);
}

#[test]
fn ov3_corner_radius_override_applies() {
    // A rounded-corner override on a square master: previously it snapped back
    // to the master's square corners.
    let fig = rect_master_instance(vec![], vec![("cornerRadius", KiwiValue::Float(8.0))]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert!(report.override_fields_applied >= 1);
    let field = field_override_of(&doc).expect("a Field override is imported");
    assert_eq!(
        field.get("corner_radius").and_then(|v| v.as_f64()),
        Some(8.0)
    );
    // The paired `corner_radii: null` restates the master's absent per-corner
    // field, so the snapshot rule drops it rather than pinning the instance.
    assert!(field.get("corner_radii").is_none());
    assert!(report.override_fields_dropped_snapshot >= 1);
    match &expanded_vector_node(&doc).data {
        NodeData::Vector(v) => assert_eq!(v.corner_radius, Some(8.0)),
        other => panic!("expected vector, got {other:?}"),
    }
}

#[test]
fn ov4_blend_mode_override_applies() {
    let fig = rect_master_instance(
        vec![],
        vec![("blendMode", KiwiValue::Enum("MULTIPLY".into()))],
    );
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert!(report.override_fields_applied >= 1);
    let field = field_override_of(&doc).expect("a Field override is imported");
    assert!(field.get("blend_mode").is_some(), "carries blend_mode");
    assert_eq!(expanded_vector_node(&doc).blend_mode, BlendMode::Multiply);
}

/// A SYMBOL master ("Header") with one TEXT child ("Title", `guid (0,2)`), and
/// an INSTANCE carrying one symbolOverride entry addressing that child.
fn text_master_instance(override_fields: Vec<(&str, KiwiValue)>) -> FigDocument {
    let mut ov_fields = vec![("guidPath", guid_path(0, 2))];
    ov_fields.extend(override_fields);
    doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("Header".to_owned())),
                ("size", vector(300.0, 80.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("TEXT".into())),
                ("name", KiwiValue::String("Title".to_owned())),
                ("size", vector(300.0, 40.0)),
                ("textData", text_data("Title")),
                ("fontSize", KiwiValue::Float(16.0)),
                ("fontName", font_name("Inter", "Regular")),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3)),
                ("type", KiwiValue::Enum("INSTANCE".into())),
                ("name", KiwiValue::String("Header instance".to_owned())),
                ("size", vector(300.0, 80.0)),
                (
                    "symbolData",
                    o(
                        "SymbolData",
                        vec![
                            ("symbolID", guid(0, 1)),
                            (
                                "symbolOverrides",
                                KiwiValue::Array(vec![o("NodeChange", ov_fields)]),
                            ),
                        ],
                    ),
                ),
            ],
        ),
    ])
}

fn expanded_text_node(doc: &Doc) -> TextNode {
    expand_only_instance(doc)
        .into_iter()
        .find_map(|e| match e.node.data {
            NodeData::Text(t) => Some(t),
            _ => None,
        })
        .expect("the master's text child expands")
}

#[test]
fn ov6_text_style_scalars_apply() {
    // Relabeled AND restyled text: the override entry carries `fontSize` /
    // `fontName` / `letterSpacing` alongside its characters. Previously only
    // `textData.characters` was read — the label changed but kept master
    // styling. The Field's `style` is the master style + the entry's scalars
    // (a FULL TextStyle: replacement, matching the shallow serde-merge).
    let fig = text_master_instance(vec![
        ("textData", text_data("Big Title")),
        ("fontSize", KiwiValue::Float(24.0)),
        ("fontName", font_name("Inter", "Bold")),
        ("letterSpacing", number(2.0, "PIXELS")),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert!(report.override_fields_applied >= 1);
    let field = field_override_of(&doc).expect("a Field override is imported");
    assert!(
        field.get("style").is_some(),
        "carries the full style object"
    );
    let title = expanded_text_node(&doc);
    assert_eq!(
        title.content, "Big Title",
        "the typed Text arm still applies"
    );
    assert_eq!(title.style.size_px, 24.0);
    assert_eq!(title.style.weight, 700);
    assert_eq!(title.style.letter_spacing, 2.0);
}

#[test]
fn ov7_style_runs_apply() {
    // Per-run styles inside overridden instance text: the override entry's
    // `textData` carries `characterStyleIDs` + `styleOverrideTable` (the first
    // two characters bolded). Decoded through the SAME
    // `build_content_and_style_runs` path master text uses, and emitted with
    // the exact `content` the run byte offsets index into.
    let run_style = o(
        "NodeChange",
        vec![
            ("styleID", KiwiValue::Uint(1)),
            ("fontName", font_name("Inter", "Bold")),
        ],
    );
    let override_text = o(
        "TextData",
        vec![
            ("characters", KiwiValue::String("Go bold".to_owned())),
            (
                "characterStyleIDs",
                KiwiValue::Array(vec![
                    KiwiValue::Uint(1),
                    KiwiValue::Uint(1),
                    KiwiValue::Uint(0),
                    KiwiValue::Uint(0),
                    KiwiValue::Uint(0),
                    KiwiValue::Uint(0),
                    KiwiValue::Uint(0),
                ]),
            ),
            ("styleOverrideTable", KiwiValue::Array(vec![run_style])),
        ],
    );
    let fig = text_master_instance(vec![("textData", override_text)]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert!(report.override_fields_applied >= 1);
    let field = field_override_of(&doc).expect("a Field override is imported");
    assert!(field.get("style_runs").is_some(), "carries the run table");
    assert!(
        field.get("content").is_some(),
        "carries the content the run offsets index into"
    );
    let title = expanded_text_node(&doc);
    assert_eq!(title.content, "Go bold");
    assert_eq!(title.style_runs.len(), 1, "one bolded run");
    assert_eq!(title.style_runs[0].start, 0);
    assert_eq!(title.style_runs[0].end, 2);
    assert_eq!(title.style_runs[0].style.weight, 700);
    assert_eq!(title.style.weight, 400, "the base style stays regular");
}

#[test]
fn snapshot_restating_master_emits_no_field_override() {
    // Figma bakes resolved state into override entries: an `opacity` that
    // merely restates the master child's own value is a snapshot, not an
    // authored delta. Emitting it would pin the instance against master edits
    // (the same rule the fills/strokes arms apply), so nothing is emitted.
    let fig = rect_master_instance(
        vec![("opacity", KiwiValue::Float(0.4))],
        vec![("opacity", KiwiValue::Float(0.4))],
    );
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert!(
        report.override_fields_dropped_snapshot >= 1,
        "the restated key is counted as a dropped snapshot"
    );
    assert_eq!(report.override_fields_applied, 0, "no key survived");
    assert!(
        field_override_of(&doc).is_none(),
        "no Field override is emitted when every key restates the master"
    );
    // The instance still renders at the (master's) 0.4 — via inheritance, so a
    // later master edit propagates instead of being pinned.
    let body = expanded_vector_node(&doc);
    assert!((body.opacity.get() - 0.4).abs() < 1e-4);
}
