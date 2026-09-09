use super::*;

// =============================================================================
// In-place variant-master placeholder hiding (the spurious-▪/▾ fix)
// =============================================================================

/// Build the Action-Button shape: a documentation FRAME holding a variant-member
/// SYMBOL master (name carries `Label ?=True, Icon ?=False`) whose children bind
/// `VISIBLE` to three props via THIN member prop defs that inherit name+default
/// from the set-level parent defs:
///   - `Hold Icon` (the 5×5 ▪): bound to a NON-axis bool prop `"Hold Icon ?"`.
///   - `Label` (TEXT):           bound to the variant AXIS `"Label ?"` (=True).
///   - `Icon` (VECTOR):          bound to the variant AXIS `"Icon ?"` (=False).
///
/// The master renders in place (nested in design content). The placeholder pass
/// must HIDE the Hold Icon (non-axis placeholder, no per-variant supplier) and
/// the Icon (axis False), while KEEPING the Label (axis True).
fn action_button_variant_master_doc() -> FigDocument {
    // Set-level parent prop defs (carry name + type + default).
    let hold_parent = guid(7, 0); // "Hold Icon ?" BOOL default true (NOT an axis)
    let label_parent = guid(7, 100); // "Label ?" BOOL (a variant axis)
    let icon_parent = guid(7, 200); // "Icon ?" BOOL (a variant axis)
    // Member-level thin prop defs the children actually ref (inherit from above).
    let hold_member = guid(8, 1);
    let label_member = guid(8, 2);
    let icon_member = guid(8, 3);

    doc_from(vec![
        // Document + page so the master is real design content.
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
        // The component-set documentation FRAME (carries the named parent defs).
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("FRAME".into())),
                (
                    "name",
                    KiwiValue::String("Action Button - Desktop".to_owned()),
                ),
                ("size", vector(400.0, 200.0)),
                ("isStateGroup", KiwiValue::Bool(true)),
                (
                    "componentPropDefs",
                    KiwiValue::array(vec![
                        prop_def(hold_parent, "Hold Icon ?", "BOOL", var_value_bool(true)),
                        prop_def(label_parent, "Label ?", "BOOL", var_value_bool(true)),
                        prop_def(icon_parent, "Icon ?", "BOOL", var_value_bool(true)),
                    ]),
                ),
            ],
        ),
        // The variant-member SYMBOL master, nested inside the FRAME so it renders
        // IN PLACE (not relocated). Its name carries the axis selections.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 10)),
                ("parentIndex", parent_index(0, 2)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                (
                    "name",
                    KiwiValue::String(
                        "Color=Standard, State=Default, Label ?=True, Icon ?=False".to_owned(),
                    ),
                ),
                ("size", vector(88.0, 32.0)),
                (
                    "componentPropDefs",
                    KiwiValue::array(vec![
                        thin_prop_def(guid(8, 1), guid(7, 0)),
                        thin_prop_def(guid(8, 2), guid(7, 100)),
                        thin_prop_def(guid(8, 3), guid(7, 200)),
                    ]),
                ),
            ],
        ),
        // Hold Icon — the 5×5 placeholder ▪, VISIBLE→non-axis bool "Hold Icon ?".
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 11)),
                ("parentIndex", parent_index(0, 10)),
                ("type", KiwiValue::Enum("VECTOR".into())),
                ("name", KiwiValue::String("Hold Icon".to_owned())),
                ("size", vector(5.0, 5.0)),
                ("visible", KiwiValue::Bool(true)),
                (
                    "componentPropRefs",
                    KiwiValue::array(vec![prop_ref(hold_member, "VISIBLE")]),
                ),
            ],
        ),
        // Label — TEXT, VISIBLE→axis "Label ?" (this member: True ⇒ shown).
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 12)),
                ("parentIndex", parent_index(0, 10)),
                ("type", KiwiValue::Enum("TEXT".into())),
                ("name", KiwiValue::String("Label".to_owned())),
                ("size", vector(40.0, 18.0)),
                ("textData", text_data("Action")),
                ("fontSize", KiwiValue::Float(14.0)),
                ("visible", KiwiValue::Bool(true)),
                (
                    "componentPropRefs",
                    KiwiValue::array(vec![prop_ref(label_member, "VISIBLE")]),
                ),
            ],
        ),
        // Icon — VECTOR, VISIBLE→axis "Icon ?" (this member: False ⇒ hidden).
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 13)),
                ("parentIndex", parent_index(0, 10)),
                ("type", KiwiValue::Enum("VECTOR".into())),
                ("name", KiwiValue::String("Icon".to_owned())),
                ("size", vector(18.0, 18.0)),
                ("visible", KiwiValue::Bool(true)),
                (
                    "componentPropRefs",
                    KiwiValue::array(vec![prop_ref(icon_member, "VISIBLE")]),
                ),
            ],
        ),
    ])
}

#[test]
fn in_place_variant_master_resolves_axis_and_non_axis_visible_defaults() {
    let fig = action_button_variant_master_doc();
    let (doc, report, _) = fig_to_doc(&fig).unwrap();

    // Three VISIBLE-bound children: Hold Icon uses its non-axis BOOL default
    // (true), Label uses axis=True, Icon uses axis=False; only Icon is hidden.
    assert_eq!(
        report.master_placeholders_hidden, 1,
        "only the false-axis child is hidden"
    );

    // Non-axis BOOL props use their component default. This mirrors real
    // Spectrum content such as Coach Mark card images (`Image ?=true`).
    assert_eq!(
        node_hidden(&doc, "Hold Icon"),
        Some(false),
        "a non-axis VISIBLE prop with default true must remain visible"
    );
    // The Icon child binds the variant axis "Icon ?" which this member sets False,
    // so it is hidden.
    assert_eq!(
        node_hidden(&doc, "Icon"),
        Some(true),
        "an Icon bound to a variant axis that is False must be hidden"
    );
    // The Label binds the axis "Label ?" which this member sets True — it MUST
    // stay visible (real content, not a placeholder).
    assert_eq!(
        node_hidden(&doc, "Label"),
        Some(false),
        "a Label bound to a variant axis that is True must remain visible"
    );
}

#[test]
fn plain_non_variant_master_placeholders_are_untouched() {
    // A NON-variant SYMBOL master (no `Axis=Value` name) is not a component-set
    // member, so the placeholder pass must leave its VISIBLE-bound children alone
    // — we only resolve placeholders for in-place variant members.
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
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 10)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("Plain Button".to_owned())),
                ("size", vector(88.0, 32.0)),
                (
                    "componentPropDefs",
                    KiwiValue::array(vec![prop_def(
                        guid(7, 0),
                        "Hold Icon ?",
                        "BOOL",
                        var_value_bool(true),
                    )]),
                ),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 11)),
                ("parentIndex", parent_index(0, 10)),
                ("type", KiwiValue::Enum("VECTOR".into())),
                ("name", KiwiValue::String("Hold Icon".to_owned())),
                ("size", vector(5.0, 5.0)),
                ("visible", KiwiValue::Bool(true)),
                (
                    "componentPropRefs",
                    KiwiValue::array(vec![prop_ref(guid(7, 0), "VISIBLE")]),
                ),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(
        report.master_placeholders_hidden, 0,
        "a non-variant master is not touched by the placeholder pass"
    );
    assert_eq!(
        node_hidden(&doc, "Hold Icon"),
        Some(false),
        "the placeholder pass only applies to component-set variant members"
    );
}
