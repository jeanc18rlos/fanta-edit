//! Baked derivedSymbolData + auto-width / line-height text behavior.

use super::*;

/// A `Matrix` value from the six affine components `[m00,m01,m02,m10,m11,m12]`.
#[test]
fn derived_symbol_data_applies_baked_geometry_size_transform_on_expand() {
    // A SYMBOL master with a single VECTOR child that is a small WHITE 10×10 rect
    // (the "light master"). An INSTANCE carries `derivedSymbolData`: one entry
    // addressing that child (guidPath → its guid) with Figma's RESOLVED render
    // data — a dark 40×8 baked path, a baked size, and a baked transform. After
    // expansion the descendant must render with the BAKED values (real path, new
    // size + position), not the light master's rect.
    let bar = {
        let mut b = Vec::new();
        b.extend(cmd(1, &[0.0, 0.0])); // Move
        b.extend(cmd(2, &[40.0, 0.0])); // Line
        b.extend(cmd(2, &[40.0, 8.0])); // Line
        b.extend(cmd(2, &[0.0, 8.0])); // Line
        b.extend(cmd(0, &[])); // Close
        b
    };
    let fig = doc_from_with_blobs(
        vec![
            o(
                "NodeChange",
                vec![
                    ("guid", guid(0, 1)),
                    ("type", KiwiValue::Enum("SYMBOL".to_owned())),
                    ("name", KiwiValue::String("Header".to_owned())),
                    ("size", vector(300.0, 80.0)),
                ],
            ),
            o(
                "NodeChange",
                vec![
                    ("guid", guid(0, 2)),
                    ("parentIndex", parent_index(0, 1)),
                    ("type", KiwiValue::Enum("RECTANGLE".to_owned())),
                    ("name", KiwiValue::String("Bar".to_owned())),
                    ("size", vector(10.0, 10.0)),
                    (
                        "fillPaints",
                        KiwiValue::Array(vec![solid_paint(1.0, 1.0, 1.0, 1.0)]),
                    ),
                ],
            ),
            o(
                "NodeChange",
                vec![
                    ("guid", guid(0, 3)),
                    ("type", KiwiValue::Enum("INSTANCE".to_owned())),
                    ("name", KiwiValue::String("Header instance".to_owned())),
                    ("size", vector(300.0, 80.0)),
                    (
                        "symbolData",
                        o("SymbolData", vec![("symbolID", guid(0, 1))]),
                    ),
                    (
                        "derivedSymbolData",
                        KiwiValue::Array(vec![o(
                            "NodeChange",
                            vec![
                                ("guidPath", guid_path(0, 2)),
                                ("size", vector(40.0, 8.0)),
                                // Resolved transform: place the bar at (12, 34).
                                ("transform", matrix([1.0, 0.0, 12.0, 0.0, 1.0, 34.0])),
                                // Resolved fill geometry: the dark bar's real path,
                                // decoded from blob index 0.
                                (
                                    "fillGeometry",
                                    KiwiValue::Array(vec![fig_path(0, "NONZERO")]),
                                ),
                            ],
                        )]),
                    ),
                ],
            ),
        ],
        vec![bar],
    );

    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(
        report.instances_with_derived, 1,
        "the instance carries baked derivedSymbolData"
    );
    assert_eq!(
        report.derived_overrides_applied, 1,
        "one resolved derived entry"
    );
    assert_eq!(
        report.derived_geometry_decoded, 1,
        "fillGeometry decoded to a real path"
    );
    assert_eq!(report.derived_with_size, 1);
    assert_eq!(report.derived_with_transform, 1);

    let expanded = expand_only_instance(&doc);
    // The descendant clone (the master rect) must now carry the baked path,
    // size, and transform — Figma's resolved values, not the light master.
    let bar_node = expanded
        .iter()
        .find(|e| !e.def_path.is_empty())
        .expect("the bar descendant is present");
    assert_eq!(
        bar_node.node.transform,
        fanta_doc::transform::Transform2D::translation(12.0, 34.0),
        "baked transform overwrites the clone's transform"
    );
    match &bar_node.node.data {
        NodeData::Vector(v) => {
            assert_eq!(
                v.path.segments,
                vec![
                    PathSegment::Move { to: [0.0, 0.0] },
                    PathSegment::Line { to: [40.0, 0.0] },
                    PathSegment::Line { to: [40.0, 8.0] },
                    PathSegment::Line { to: [0.0, 8.0] },
                    PathSegment::Close,
                ],
                "baked fillGeometry replaces the master rect path"
            );
        }
        other => panic!("expected vector, got {other:?}"),
    }
}

#[test]
fn derived_text_color_and_weight_themes_an_instance_label() {
    // A SYMBOL master with a TEXT child styled near-BLACK at weight 400 (the
    // "light master"). An INSTANCE on a dark page carries `derivedSymbolData`:
    // one entry addressing that text child whose RESOLVED `fillPaints` is a light
    // (white) color and whose `fontName` is SemiBold. After expansion the label
    // clone must render WHITE at weight 600 — the themed-text fix (a dark-page
    // label reads in its real light color, not the master near-black).
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".to_owned())),
                ("name", KiwiValue::String("Button".to_owned())),
                ("size", vector(120.0, 40.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("TEXT".to_owned())),
                ("name", KiwiValue::String("Label".to_owned())),
                ("size", vector(100.0, 20.0)),
                ("textData", text_data("Submit")),
                ("fontSize", KiwiValue::Float(16.0)),
                ("fontName", font_name("Helvetica", "Regular")),
                // Master glyph color: near-black.
                (
                    "fillPaints",
                    KiwiValue::Array(vec![solid_paint(0.05, 0.05, 0.05, 1.0)]),
                ),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3)),
                ("type", KiwiValue::Enum("INSTANCE".to_owned())),
                ("name", KiwiValue::String("Button instance".to_owned())),
                ("size", vector(120.0, 40.0)),
                (
                    "symbolData",
                    o("SymbolData", vec![("symbolID", guid(0, 1))]),
                ),
                (
                    "derivedSymbolData",
                    KiwiValue::Array(vec![o(
                        "NodeChange",
                        vec![
                            ("guidPath", guid_path(0, 2)),
                            // Resolved (theme) glyph color: white.
                            (
                                "fillPaints",
                                KiwiValue::Array(vec![solid_paint(1.0, 1.0, 1.0, 1.0)]),
                            ),
                            // Resolved font: SemiBold (weight 600).
                            (
                                "derivedTextData",
                                o(
                                    "TextData",
                                    vec![
                                        ("characters", KiwiValue::String("Submit".to_owned())),
                                        ("fontName", font_name("Inter", "SemiBold")),
                                    ],
                                ),
                            ),
                        ],
                    )]),
                ),
            ],
        ),
    ]);

    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(
        report.derived_overrides_applied, 1,
        "one resolved derived entry"
    );
    assert_eq!(
        report.derived_text_color, 1,
        "the entry carried a resolved glyph color"
    );
    assert_eq!(
        report.derived_text_weight, 1,
        "the entry carried a resolved weight"
    );

    let expanded = expand_only_instance(&doc);
    let label = expanded
        .iter()
        .find(|e| !e.def_path.is_empty())
        .expect("the label descendant is present");
    match &label.node.data {
        NodeData::Text(t) => {
            assert_eq!(
                t.style.color,
                Color::rgba(255, 255, 255, 255),
                "derived white color overwrites the master near-black"
            );
            assert_eq!(
                t.style.weight, 600,
                "derived SemiBold overwrites master 400"
            );
            assert_eq!(
                t.style.font_family, "Inter",
                "derived family overwrites master"
            );
        }
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn auto_width_text_does_not_wrap_to_its_hugged_box() {
    // Regression for the card title/subtitle overlap: a Figma auto-width
    // (WIDTH_AND_HEIGHT) TEXT node hugs its content, so its stored width is the
    // already-shrink-wrapped box. We must NOT wrap to it (that re-breaks the line
    // and collides with the sibling below). The renderer keys off auto_resize to
    // shape at no-wrap width, while the doc keeps Figma's real hugged box.
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("TEXT".to_owned())),
            ("name", KiwiValue::String("Link Prompt".to_owned())),
            ("size", vector(108.0, 20.0)),
            ("textData", text_data("View guidelines")),
            ("fontSize", KiwiValue::Float(15.0)),
            (
                "textAutoResize",
                KiwiValue::Enum("WIDTH_AND_HEIGHT".to_owned()),
            ),
        ],
    )]);
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    match &doc.scene.get(doc.scene.roots()[0]).unwrap().data {
        NodeData::Text(t) => {
            assert_eq!(t.auto_resize, TextAutoResize::WidthAndHeight);
            assert_eq!(t.local_size, [108.0, 20.0]);
        }
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn text_decoration_imports_to_text_style() {
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("TEXT".to_owned())),
            ("size", vector(120.0, 32.0)),
            ("textData", text_data("Underlined")),
            ("textDecoration", KiwiValue::String("UNDERLINE".to_owned())),
        ],
    )]);
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    match &doc.scene.get(doc.scene.roots()[0]).unwrap().data {
        NodeData::Text(t) => {
            assert!(t.style.underline);
            assert!(!t.style.strikethrough);
        }
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn text_style_override_table_preserves_metric_changing_runs() {
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("TEXT".to_owned())),
            ("size", vector(160.0, 40.0)),
            (
                "textData",
                o(
                    "TextData",
                    vec![
                        ("characters", KiwiValue::String("Hello".to_owned())),
                        (
                            "characterStyleIDs",
                            KiwiValue::Array(vec![
                                KiwiValue::Uint(0),
                                KiwiValue::Uint(1),
                                KiwiValue::Uint(1),
                                KiwiValue::Uint(0),
                                KiwiValue::Uint(0),
                            ]),
                        ),
                        (
                            "styleOverrideTable",
                            KiwiValue::Array(vec![o(
                                "NodeChange",
                                vec![
                                    ("fontSize", KiwiValue::Float(24.0)),
                                    ("fontName", font_name("Inter", "Semi Bold Italic")),
                                    (
                                        "fillPaints",
                                        KiwiValue::Array(vec![solid_paint(1.0, 0.0, 0.0, 1.0)]),
                                    ),
                                    (
                                        "textDecoration",
                                        KiwiValue::String("STRIKETHROUGH".to_owned()),
                                    ),
                                ],
                            )]),
                        ),
                    ],
                ),
            ),
            ("fontSize", KiwiValue::Float(14.0)),
            ("fontName", font_name("Helvetica", "Regular")),
        ],
    )]);
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    match &doc.scene.get(doc.scene.roots()[0]).unwrap().data {
        NodeData::Text(t) => {
            assert_eq!(t.content, "Hello");
            assert_eq!(t.style_runs.len(), 1);
            assert_eq!(t.style_runs[0].start, 1);
            assert_eq!(t.style_runs[0].end, 3);
            assert_eq!(t.style_runs[0].style.size_px, 24.0);
            assert_eq!(t.style_runs[0].style.weight, 600);
            assert!(t.style_runs[0].style.italic);
            assert!(t.style_runs[0].style.strikethrough);
        }
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn text_style_override_table_uses_sparse_style_ids() {
    // Real Figma files store styleOverrideTable as a compact list whose entries
    // carry their authored styleID. The characterStyleIDs are not dense array
    // indices (Spectrum uses ids like 9, 10, 11 in a 3-entry table).
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("TEXT".to_owned())),
            ("size", vector(160.0, 40.0)),
            (
                "textData",
                o(
                    "TextData",
                    vec![
                        ("characters", KiwiValue::String("Hello".to_owned())),
                        (
                            "characterStyleIDs",
                            KiwiValue::Array(vec![
                                KiwiValue::Uint(0),
                                KiwiValue::Uint(20),
                                KiwiValue::Uint(20),
                                KiwiValue::Uint(0),
                                KiwiValue::Uint(0),
                            ]),
                        ),
                        (
                            "styleOverrideTable",
                            KiwiValue::Array(vec![o(
                                "NodeChange",
                                vec![
                                    ("styleID", KiwiValue::Uint(20)),
                                    ("fontName", font_name("Source Sans Pro", "Bold")),
                                ],
                            )]),
                        ),
                    ],
                ),
            ),
            ("fontSize", KiwiValue::Float(18.0)),
            ("fontName", font_name("Source Sans Pro", "Regular")),
        ],
    )]);
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    match &doc.scene.get(doc.scene.roots()[0]).unwrap().data {
        NodeData::Text(t) => {
            assert_eq!(t.content, "Hello");
            assert_eq!(t.style_runs.len(), 1);
            assert_eq!(t.style_runs[0].start, 1);
            assert_eq!(t.style_runs[0].end, 3);
            assert_eq!(t.style_runs[0].style.weight, 700);
        }
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn text_style_override_table_resolves_fill_style_refs() {
    let style_ref = |sid: u32, lid: u32| o("StyleId", vec![("guid", guid(sid, lid))]);
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(10, 95)),
                ("type", KiwiValue::Enum("VECTOR".to_owned())),
                ("styleType", KiwiValue::Enum("FILL".to_owned())),
                (
                    "fillPaints",
                    KiwiValue::Array(vec![solid_paint(
                        20.0 / 255.0,
                        115.0 / 255.0,
                        230.0 / 255.0,
                        1.0,
                    )]),
                ),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("TEXT".to_owned())),
                ("size", vector(160.0, 40.0)),
                (
                    "textData",
                    o(
                        "TextData",
                        vec![
                            ("characters", KiwiValue::String("Hello".to_owned())),
                            (
                                "characterStyleIDs",
                                KiwiValue::Array(vec![
                                    KiwiValue::Uint(0),
                                    KiwiValue::Uint(20),
                                    KiwiValue::Uint(20),
                                    KiwiValue::Uint(0),
                                    KiwiValue::Uint(0),
                                ]),
                            ),
                            (
                                "styleOverrideTable",
                                KiwiValue::Array(vec![o(
                                    "NodeChange",
                                    vec![
                                        ("styleID", KiwiValue::Uint(20)),
                                        ("styleIdForFill", style_ref(10, 95)),
                                    ],
                                )]),
                            ),
                        ],
                    ),
                ),
                (
                    "fillPaints",
                    KiwiValue::Array(vec![solid_paint(
                        34.0 / 255.0,
                        34.0 / 255.0,
                        34.0 / 255.0,
                        1.0,
                    )]),
                ),
                ("fontSize", KiwiValue::Float(18.0)),
                ("fontName", font_name("Source Sans Pro", "Regular")),
            ],
        ),
    ]);
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    let text = doc
        .scene
        .roots()
        .iter()
        .flat_map(|&root| doc.scene.descendants_of(root))
        .find_map(|id| match &doc.scene.get(id).unwrap().data {
            NodeData::Text(t) => Some(t),
            _ => None,
        })
        .expect("text node");
    assert_eq!(text.style_runs.len(), 1);
    assert_eq!(
        text.style_runs[0].style.color,
        Color::rgba(20, 115, 230, 255)
    );
}

#[test]
fn fixed_width_text_keeps_its_box_width() {
    // A non-auto-width TEXT node (textAutoResize absent / HEIGHT) keeps its
    // authored width so it wraps normally.
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("TEXT".to_owned())),
            ("size", vector(200.0, 60.0)),
            ("textData", text_data("a long paragraph that should wrap")),
            ("fontSize", KiwiValue::Float(16.0)),
            ("textAutoResize", KiwiValue::Enum("HEIGHT".to_owned())),
        ],
    )]);
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    match &doc.scene.get(doc.scene.roots()[0]).unwrap().data {
        NodeData::Text(t) => assert_eq!(t.local_size, [200.0, 60.0], "auto-height keeps its box"),
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn raw_unit_line_height_is_a_passthrough_multiplier() {
    // RAW lineHeight is already a unitless multiple of the font size.
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("TEXT".to_owned())),
            ("size", vector(100.0, 40.0)),
            ("textData", text_data("hi")),
            ("fontSize", KiwiValue::Float(20.0)),
            ("lineHeight", number(1.4, "RAW")),
        ],
    )]);
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    match &doc.scene.get(doc.scene.roots()[0]).unwrap().data {
        // f32→f64 widening lands 1.4 at ~1.3999999; compare with tolerance.
        NodeData::Text(t) => assert!(
            (t.style.line_height - 1.4).abs() < 1e-5,
            "RAW 1.4 → line_height ~1.4, got {}",
            t.style.line_height
        ),
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn raw_one_line_height_uses_authored_figma_box_height() {
    // Real Figma exports often encode "auto"/normal leading as RAW 1.0 while
    // the TEXT box already carries the normal line box height. Use that box
    // height so vertically-centered labels do not sink inside their frames.
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("TEXT".to_owned())),
            ("size", vector(71.0, 15.0)),
            ("textData", text_data("Email address")),
            ("fontSize", KiwiValue::Float(12.0)),
            ("lineHeight", number(1.0, "RAW")),
        ],
    )]);
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    match &doc.scene.get(doc.scene.roots()[0]).unwrap().data {
        NodeData::Text(t) => assert!(
            (t.style.line_height - 1.25).abs() < 1e-6,
            "RAW 1.0 should use the 15/12 authored box ratio, got {}",
            t.style.line_height
        ),
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn raw_one_line_height_ignores_subnormal_compact_box_height() {
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("TEXT".to_owned())),
            ("size", vector(47.0, 11.0)),
            ("textData", text_data("#FF0000")),
            ("fontSize", KiwiValue::Float(13.0)),
            ("lineHeight", number(1.0, "RAW")),
        ],
    )]);
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    match &doc.scene.get(doc.scene.roots()[0]).unwrap().data {
        NodeData::Text(t) => assert!(
            (t.style.line_height - 1.25).abs() < 1e-6,
            "RAW 1.0 should keep normal leading for compact boxes, got {}",
            t.style.line_height
        ),
        other => panic!("expected text, got {other:?}"),
    }
}
