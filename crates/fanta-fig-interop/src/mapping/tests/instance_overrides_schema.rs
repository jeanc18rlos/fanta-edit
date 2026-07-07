//! Instance overrides: symbolOverrides, nested routing, swaps, prop assignments.

use super::*;

// =============================================================================
// Component property SCHEMA import + DEFAULTS (componentPropDefs)
// =============================================================================

#[test]
fn component_prop_defs_parse_onto_component_def() {
    // A SYMBOL exposing a TEXT prop ("Label", default "Click") and a BOOL prop
    // ("Show Icon", default true). Both must land on ComponentDef.props with the
    // right kind + default.
    let label_prop = guid(8, 1);
    let icon_prop = guid(8, 2);
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("SYMBOL".to_owned())),
            ("name", KiwiValue::String("Button".to_owned())),
            ("size", vector(120.0, 40.0)),
            (
                "componentPropDefs",
                KiwiValue::Array(vec![
                    prop_def(label_prop, "Label", "TEXT", var_value_text("Click")),
                    prop_def(icon_prop, "Show Icon", "BOOL", var_value_bool(true)),
                ]),
            ),
        ],
    )]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.component_props, 2, "two props parsed onto the def");
    let def = doc.components.defs.values().next().unwrap();
    let label = def.props.iter().find(|p| p.name == "Label").unwrap();
    assert!(matches!(
        label.kind,
        fanta_doc::component::ComponentPropKind::Text
    ));
    assert_eq!(
        label.default,
        VarValue::String {
            value: "Click".into()
        }
    );
    let icon = def.props.iter().find(|p| p.name == "Show Icon").unwrap();
    assert!(matches!(
        icon.kind,
        fanta_doc::component::ComponentPropKind::Bool
    ));
    assert_eq!(icon.default, VarValue::Boolean { value: true });
}

#[test]
fn unset_prop_default_drives_bound_text_on_expand() {
    // A SYMBOL exposes a TEXT prop ("Label", default "Default Label"); its TEXT
    // child binds TEXT_DATA to that prop. An INSTANCE that assigns NOTHING must
    // still render the prop DEFAULT, not the master's authored "Placeholder".
    let label_prop = guid(9, 1);
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".to_owned())),
                ("name", KiwiValue::String("Button".to_owned())),
                ("size", vector(120.0, 40.0)),
                (
                    "componentPropDefs",
                    KiwiValue::Array(vec![prop_def(
                        label_prop.clone(),
                        "Label",
                        "TEXT",
                        var_value_text("Default Label"),
                    )]),
                ),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("TEXT".to_owned())),
                ("name", KiwiValue::String("Label".to_owned())),
                ("size", vector(120.0, 40.0)),
                ("textData", text_data("Placeholder")),
                ("fontSize", KiwiValue::Float(16.0)),
                (
                    "componentPropRefs",
                    KiwiValue::Array(vec![o(
                        "ComponentPropRef",
                        vec![
                            ("defID", label_prop),
                            (
                                "componentPropNodeField",
                                KiwiValue::Enum("TEXT_DATA".to_owned()),
                            ),
                        ],
                    )]),
                ),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3)),
                ("type", KiwiValue::Enum("INSTANCE".to_owned())),
                ("name", KiwiValue::String("Btn instance".to_owned())),
                ("size", vector(120.0, 40.0)),
                (
                    "symbolData",
                    o("SymbolData", vec![("symbolID", guid(0, 1))]),
                ),
                // NO componentPropAssignments — the instance relies on the default.
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert!(
        report.prop_defaults_applied >= 1,
        "the unset prop's default should be applied to the bound text"
    );
    let expanded = expand_only_instance(&doc);
    let label = expanded.iter().find_map(|e| match &e.node.data {
        NodeData::Text(t) => Some(t.content.clone()),
        _ => None,
    });
    assert_eq!(
        label.as_deref(),
        Some("Default Label"),
        "the prop DEFAULT (not the master placeholder) renders when the instance \
         leaves the prop unset"
    );
}

#[test]
fn assigned_prop_overrides_its_default() {
    // Same master as above, but the instance DOES assign "Label" = "Explicit".
    // The assignment must win; the default must not also fire (no double-apply).
    let label_prop = guid(10, 1);
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".to_owned())),
                ("name", KiwiValue::String("Button".to_owned())),
                ("size", vector(120.0, 40.0)),
                (
                    "componentPropDefs",
                    KiwiValue::Array(vec![prop_def(
                        label_prop.clone(),
                        "Label",
                        "TEXT",
                        var_value_text("Default Label"),
                    )]),
                ),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("TEXT".to_owned())),
                ("name", KiwiValue::String("Label".to_owned())),
                ("size", vector(120.0, 40.0)),
                ("textData", text_data("Placeholder")),
                ("fontSize", KiwiValue::Float(16.0)),
                (
                    "componentPropRefs",
                    KiwiValue::Array(vec![o(
                        "ComponentPropRef",
                        vec![
                            ("defID", label_prop.clone()),
                            (
                                "componentPropNodeField",
                                KiwiValue::Enum("TEXT_DATA".to_owned()),
                            ),
                        ],
                    )]),
                ),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3)),
                ("type", KiwiValue::Enum("INSTANCE".to_owned())),
                ("name", KiwiValue::String("Btn instance".to_owned())),
                ("size", vector(120.0, 40.0)),
                (
                    "symbolData",
                    o("SymbolData", vec![("symbolID", guid(0, 1))]),
                ),
                (
                    "componentPropAssignments",
                    KiwiValue::Array(vec![o(
                        "ComponentPropAssignment",
                        vec![
                            ("defID", label_prop),
                            ("value", prop_value_text("Explicit")),
                        ],
                    )]),
                ),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(
        report.prop_defaults_applied, 0,
        "an assigned prop must not also apply its default"
    );
    assert_eq!(
        report.instances_with_prop_values, 1,
        "the assignment populated typed prop_values"
    );
    let expanded = expand_only_instance(&doc);
    let label = expanded.iter().find_map(|e| match &e.node.data {
        NodeData::Text(t) => Some(t.content.clone()),
        _ => None,
    });
    assert_eq!(label.as_deref(), Some("Explicit"));
}
