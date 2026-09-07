//! Shared fixtures + builders for the mapping test suite. Pulled together
//! when `mapping_tests.rs` (one giant `include!`d block) was split into
//! per-concern test modules; every test file reaches these through
//! `use super::*;` (re-exported by `tests/mod.rs`).

#![allow(dead_code)]

use super::*;
pub use crate::kiwi::{Def, DefKind, Field, KiwiType, Schema};
pub use fanta_doc::path::{FillRule, PathSegment};
pub use fanta_doc::resolve::expand_instance;

// A Figma-shaped schema covering every field the mapper reads, including the
// component / variant / variable / prototype / vector-geometry fields. Field
// ids mirror the real `fig.kiwi` schema so synthetic values encode/decode the
// same way a real file's do.
pub(crate) fn schema() -> Schema {
    Schema::new(vec![
        // 0: NodeType
        Def::new(
            "NodeType",
            DefKind::Enum,
            vec![
                Field::new("DOCUMENT", KiwiType(0), 1),
                Field::new("CANVAS", KiwiType(0), 2),
                Field::new("GROUP", KiwiType(0), 3),
                Field::new("FRAME", KiwiType(0), 4),
                Field::new("BOOLEAN_OPERATION", KiwiType(0), 5),
                Field::new("VECTOR", KiwiType(0), 6),
                Field::new("STAR", KiwiType(0), 7),
                Field::new("LINE", KiwiType(0), 8),
                Field::new("ELLIPSE", KiwiType(0), 9),
                Field::new("RECTANGLE", KiwiType(0), 10),
                Field::new("ROUNDED_RECTANGLE", KiwiType(0), 12),
                Field::new("TEXT", KiwiType(0), 13),
                Field::new("SYMBOL", KiwiType(0), 15),
                Field::new("INSTANCE", KiwiType(0), 16),
                Field::new("COMPONENT", KiwiType(0), 17),
                Field::new("COMPONENT_SET", KiwiType(0), 18),
                Field::new("VARIABLE", KiwiType(0), 28),
                Field::new("VARIABLE_SET", KiwiType(0), 31),
            ],
        ),
        // 1: GUID
        Def::new(
            "GUID",
            DefKind::Struct,
            vec![
                Field::new("sessionID", KiwiType::UINT, 0),
                Field::new("localID", KiwiType::UINT, 0),
            ],
        ),
        // 2: ParentIndex
        Def::new(
            "ParentIndex",
            DefKind::Struct,
            vec![
                Field::new("guid", KiwiType::user(1), 0),
                Field::new("position", KiwiType::STRING, 0),
            ],
        ),
        // 3: Vector
        Def::new(
            "Vector",
            DefKind::Struct,
            vec![
                Field::new("x", KiwiType::FLOAT, 0),
                Field::new("y", KiwiType::FLOAT, 0),
            ],
        ),
        // 4: Matrix
        Def::new(
            "Matrix",
            DefKind::Struct,
            vec![
                Field::new("m00", KiwiType::FLOAT, 0),
                Field::new("m01", KiwiType::FLOAT, 0),
                Field::new("m02", KiwiType::FLOAT, 0),
                Field::new("m10", KiwiType::FLOAT, 0),
                Field::new("m11", KiwiType::FLOAT, 0),
                Field::new("m12", KiwiType::FLOAT, 0),
            ],
        ),
        // 5: Color
        Def::new(
            "Color",
            DefKind::Struct,
            vec![
                Field::new("r", KiwiType::FLOAT, 0),
                Field::new("g", KiwiType::FLOAT, 0),
                Field::new("b", KiwiType::FLOAT, 0),
                Field::new("a", KiwiType::FLOAT, 0),
            ],
        ),
        // 6: PaintType
        Def::new(
            "PaintType",
            DefKind::Enum,
            vec![
                Field::new("SOLID", KiwiType(0), 0),
                Field::new("GRADIENT_LINEAR", KiwiType(0), 1),
                Field::new("IMAGE", KiwiType(0), 4),
            ],
        ),
        // 7: Paint — field ids (image=8, imageScaleMode=10) mirror real fig.kiwi.
        Def::new(
            "Paint",
            DefKind::Message,
            vec![
                Field::new("type", KiwiType::user(6), 1),
                Field::new("color", KiwiType::user(5), 2),
                Field::new("opacity", KiwiType::FLOAT, 3),
                Field::new("visible", KiwiType::BOOL, 4),
                Field::new("image", KiwiType::user(40), 8),
                Field::new("imageScaleMode", KiwiType::user(41), 10),
            ],
        ),
        // 8: NodeChange — ids mirror the real fig.kiwi schema.
        Def::new(
            "NodeChange",
            DefKind::Message,
            vec![
                Field::new("guid", KiwiType::user(1), 1),
                Field::new("parentIndex", KiwiType::user(2), 3),
                Field::new("type", KiwiType::user(0), 4),
                Field::new("name", KiwiType::STRING, 5),
                Field::new("size", KiwiType::user(3), 11),
                Field::new("transform", KiwiType::user(4), 12),
                Field::new("cornerRadius", KiwiType::FLOAT, 20),
                Field::new("fontSize", KiwiType::FLOAT, 21),
                Field::new("textAlignHorizontal", KiwiType::user(10), 32),
                Field::array("fillPaints", KiwiType::user(7), 38),
                Field::new("lineHeight", KiwiType::user(13), 40),
                Field::new("fontName", KiwiType::user(11), 41),
                Field::new("textData", KiwiType::user(14), 42),
                Field::new("textDecoration", KiwiType::STRING, 45),
                Field::new("letterSpacing", KiwiType::user(13), 165),
                // component/variant/instance fields
                Field::new("symbolData", KiwiType::user(15), 113),
                Field::new("prototypeStartNodeID", KiwiType::user(1), 140),
                Field::new("isStateGroup", KiwiType::BOOL, 225),
                Field::array("prototypeInteractions", KiwiType::user(16), 226),
                Field::array("componentPropDefs", KiwiType::user(20), 266),
                Field::array("componentPropAssignments", KiwiType::user(21), 268),
                Field::array("variantPropSpecs", KiwiType::user(22), 483),
                // override-bearing fields (def-local indices appended below)
                Field::array("componentPropRefs", KiwiType::user(36), 270),
                Field::new("textAutoResize", KiwiType::user(38), 47),
                Field::new("visible", KiwiType::BOOL, 10),
                Field::new("guidPath", KiwiType::user(37), 250),
                // variable fields
                Field::new("variableData", KiwiType::user(23), 306),
                Field::new("variableConsumptionMap", KiwiType::user(28), 307),
                Field::array("variableSetModes", KiwiType::user(30), 312),
                Field::new("variableSetID", KiwiType::user(32), 313),
                Field::new("variableResolvedType", KiwiType::user(33), 314),
                Field::new("variableDataValues", KiwiType::user(34), 315),
            ],
        ),
        // 9: Message (root)
        Def::new(
            "Message",
            DefKind::Message,
            vec![Field::array("nodeChanges", KiwiType::user(8), 4)],
        ),
        // 10: TextAlignHorizontal
        Def::new(
            "TextAlignHorizontal",
            DefKind::Enum,
            vec![
                Field::new("LEFT", KiwiType(0), 0),
                Field::new("CENTER", KiwiType(0), 1),
                Field::new("RIGHT", KiwiType(0), 2),
                Field::new("JUSTIFIED", KiwiType(0), 3),
            ],
        ),
        // 11: FontName
        Def::new(
            "FontName",
            DefKind::Struct,
            vec![
                Field::new("family", KiwiType::STRING, 1),
                Field::new("style", KiwiType::STRING, 2),
                Field::new("postscript", KiwiType::STRING, 3),
            ],
        ),
        // 12: NumberUnits
        Def::new(
            "NumberUnits",
            DefKind::Enum,
            vec![
                Field::new("RAW", KiwiType(0), 0),
                Field::new("PIXELS", KiwiType(0), 1),
                Field::new("PERCENT", KiwiType(0), 2),
            ],
        ),
        // 13: Number
        Def::new(
            "Number",
            DefKind::Struct,
            vec![
                Field::new("value", KiwiType::FLOAT, 1),
                Field::new("units", KiwiType::user(12), 2),
            ],
        ),
        // 14: TextData
        Def::new(
            "TextData",
            DefKind::Message,
            vec![
                Field::new("characters", KiwiType::STRING, 1),
                Field::array("characterStyleIDs", KiwiType::UINT, 2),
                Field::array("styleOverrideTable", KiwiType::user(8), 3),
            ],
        ),
        // 15: SymbolData
        Def::new(
            "SymbolData",
            DefKind::Message,
            vec![
                Field::new("symbolID", KiwiType::user(1), 1),
                // Each override is a full NodeChange (guidPath + overridden fields).
                Field::array("symbolOverrides", KiwiType::user(8), 2),
            ],
        ),
        // 16: PrototypeInteraction
        Def::new(
            "PrototypeInteraction",
            DefKind::Message,
            vec![
                Field::new("id", KiwiType::user(1), 1),
                Field::new("event", KiwiType::user(17), 2),
                Field::array("actions", KiwiType::user(18), 3),
                Field::new("isDeleted", KiwiType::BOOL, 4),
            ],
        ),
        // 17: PrototypeEvent
        Def::new(
            "PrototypeEvent",
            DefKind::Message,
            vec![
                Field::new("interactionType", KiwiType::user(19), 1),
                Field::new("interactionDuration", KiwiType::FLOAT, 3),
            ],
        ),
        // 18: PrototypeAction
        Def::new(
            "PrototypeAction",
            DefKind::Message,
            vec![
                Field::new("transitionNodeID", KiwiType::user(1), 1),
                Field::new("transitionType", KiwiType::user(25), 2),
                Field::new("transitionDuration", KiwiType::FLOAT, 3),
                Field::new("easingType", KiwiType::user(26), 4),
            ],
        ),
        // 19: InteractionType
        Def::new(
            "InteractionType",
            DefKind::Enum,
            vec![
                Field::new("ON_CLICK", KiwiType(0), 0),
                Field::new("AFTER_TIMEOUT", KiwiType(0), 1),
                Field::new("ON_HOVER", KiwiType(0), 4),
                Field::new("DRAG", KiwiType(0), 9),
            ],
        ),
        // 20: ComponentPropDef — an exposed component property's schema: its id
        // (a GUID), name, type (TEXT/BOOL/INSTANCE_SWAP/VARIANT), and default
        // value (`varValue`, a VariableData).
        Def::new(
            "ComponentPropDef",
            DefKind::Message,
            vec![
                Field::new("id", KiwiType::user(1), 1),
                Field::new("name", KiwiType::STRING, 2),
                Field::new("type", KiwiType::user(42), 3),
                Field::new("varValue", KiwiType::user(23), 4),
                // A variant member's thin prop def carries only `id` +
                // `parentPropDefId`; the name/type/default live on the set-level
                // parent it inherits from. (Field id 5 is arbitrary for the test
                // schema — it only needs to round-trip through our own writer.)
                Field::new("parentPropDefId", KiwiType::user(1), 5),
            ],
        ),
        // 21: ComponentPropAssignment
        Def::new(
            "ComponentPropAssignment",
            DefKind::Message,
            vec![
                Field::new("defID", KiwiType::user(1), 1),
                Field::new("value", KiwiType::user(39), 2),
            ],
        ),
        // 22: VariantPropSpec
        Def::new(
            "VariantPropSpec",
            DefKind::Message,
            vec![
                Field::new("propDefId", KiwiType::user(1), 1),
                Field::new("value", KiwiType::STRING, 2),
            ],
        ),
        // 23: VariableData
        Def::new(
            "VariableData",
            DefKind::Message,
            vec![Field::new("value", KiwiType::user(24), 1)],
        ),
        // 24: VariableAnyValue
        Def::new(
            "VariableAnyValue",
            DefKind::Message,
            vec![
                Field::new("boolValue", KiwiType::BOOL, 1),
                Field::new("textValue", KiwiType::STRING, 2),
                Field::new("floatValue", KiwiType::FLOAT, 3),
                Field::new("alias", KiwiType::user(32), 4),
                Field::new("colorValue", KiwiType::user(5), 5),
            ],
        ),
        // 25: TransitionType
        Def::new(
            "TransitionType",
            DefKind::Enum,
            vec![
                Field::new("INSTANT_TRANSITION", KiwiType(0), 0),
                Field::new("DISSOLVE", KiwiType(0), 1),
                Field::new("SLIDE_FROM_LEFT", KiwiType(0), 3),
            ],
        ),
        // 26: EasingType
        Def::new(
            "EasingType",
            DefKind::Enum,
            vec![
                Field::new("IN_CUBIC", KiwiType(0), 0),
                Field::new("OUT_CUBIC", KiwiType(0), 1),
                Field::new("LINEAR", KiwiType(0), 3),
            ],
        ),
        // 27: VariableField
        Def::new(
            "VariableField",
            DefKind::Enum,
            vec![
                Field::new("MISSING", KiwiType(0), 0),
                Field::new("CORNER_RADIUS", KiwiType(0), 1),
                Field::new("OPACITY", KiwiType(0), 31),
                Field::new("TEXT_DATA", KiwiType(0), 11),
            ],
        ),
        // 28: VariableDataMap (variableConsumptionMap)
        Def::new(
            "VariableDataMap",
            DefKind::Message,
            vec![Field::array("entries", KiwiType::user(29), 1)],
        ),
        // 29: VariableDataMapEntry
        Def::new(
            "VariableDataMapEntry",
            DefKind::Message,
            vec![
                Field::new("variableData", KiwiType::user(23), 2),
                Field::new("variableField", KiwiType::user(27), 3),
            ],
        ),
        // 30: VariableSetMode
        Def::new(
            "VariableSetMode",
            DefKind::Message,
            vec![
                Field::new("id", KiwiType::user(1), 1),
                Field::new("name", KiwiType::STRING, 2),
            ],
        ),
        // 31: VariableSetMode array element placeholder is user(30); but the
        // NodeChange.variableSetModes field references type 31 — alias it to the
        // same message so the schema is self-consistent.
        Def::new(
            "VariableSetModeAlias",
            DefKind::Message,
            vec![
                Field::new("id", KiwiType::user(1), 1),
                Field::new("name", KiwiType::STRING, 2),
            ],
        ),
        // 32: VariableID
        Def::new(
            "VariableID",
            DefKind::Message,
            vec![Field::new("guid", KiwiType::user(1), 1)],
        ),
        // 33: VariableResolvedDataType
        Def::new(
            "VariableResolvedDataType",
            DefKind::Enum,
            vec![
                Field::new("BOOLEAN", KiwiType(0), 0),
                Field::new("FLOAT", KiwiType(0), 1),
                Field::new("STRING", KiwiType(0), 2),
                Field::new("COLOR", KiwiType(0), 4),
            ],
        ),
        // 34: VariableDataValues
        Def::new(
            "VariableDataValues",
            DefKind::Message,
            vec![Field::array("entries", KiwiType::user(35), 1)],
        ),
        // 35: VariableDataValuesEntry
        Def::new(
            "VariableDataValuesEntry",
            DefKind::Message,
            vec![
                Field::new("modeID", KiwiType::user(1), 1),
                Field::new("variableData", KiwiType::user(23), 2),
            ],
        ),
        // 36: ComponentPropRef — binds a node field to a prop-def.
        Def::new(
            "ComponentPropRef",
            DefKind::Message,
            vec![
                Field::new("defID", KiwiType::user(1), 2),
                Field::new("componentPropNodeField", KiwiType::user(27), 4),
            ],
        ),
        // 37: GUIDPath — a path of master-descendant guids addressing an override
        // target.
        Def::new(
            "GUIDPath",
            DefKind::Message,
            vec![Field::array("guids", KiwiType::user(1), 1)],
        ),
        // 38: TextAutoResize
        Def::new(
            "TextAutoResize",
            DefKind::Enum,
            vec![
                Field::new("NONE", KiwiType(0), 0),
                Field::new("HEIGHT", KiwiType(0), 1),
                Field::new("WIDTH_AND_HEIGHT", KiwiType(0), 2),
            ],
        ),
        // 39: ComponentPropValue — the value a ComponentPropAssignment sets.
        Def::new(
            "ComponentPropValue",
            DefKind::Message,
            vec![Field::new("textValue", KiwiType::user(14), 2)],
        ),
        // 40: Image — an IMAGE paint's bitmap reference (hash + name). The hash
        // is the lowercase-hex sha1 the `.fig` ZIP stores under `images/<hash>`.
        Def::new(
            "Image",
            DefKind::Message,
            vec![
                Field::array("hash", KiwiType::BYTE, 1),
                Field::new("name", KiwiType::STRING, 2),
            ],
        ),
        // 41: ImageScaleMode — how an IMAGE paint sizes into the node bounds.
        Def::new(
            "ImageScaleMode",
            DefKind::Enum,
            vec![
                Field::new("FILL", KiwiType(0), 0),
                Field::new("FIT", KiwiType(0), 1),
                Field::new("CROP", KiwiType(0), 2),
                Field::new("TILE", KiwiType(0), 3),
                Field::new("STRETCH", KiwiType(0), 4),
            ],
        ),
        // 42: ComponentPropType — the kind of an exposed component property.
        Def::new(
            "ComponentPropType",
            DefKind::Enum,
            vec![
                Field::new("BOOL", KiwiType(0), 0),
                Field::new("TEXT", KiwiType(0), 1),
                Field::new("INSTANCE_SWAP", KiwiType(0), 2),
                Field::new("VARIANT", KiwiType(0), 3),
            ],
        ),
    ])
}

pub(crate) fn o(type_name: &str, fields: Vec<(&str, KiwiValue)>) -> KiwiValue {
    KiwiValue::Object {
        type_name: type_name.into(),
        fields: fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(),
    }
}

pub(crate) fn guid(sid: u32, lid: u32) -> KiwiValue {
    o(
        "GUID",
        vec![
            ("sessionID", KiwiValue::Uint(sid)),
            ("localID", KiwiValue::Uint(lid)),
        ],
    )
}

/// A `ParentIndex` with the conventional `"!"` position. Most tests don't
/// exercise sibling ordering, so they use this constant-position form.
pub(crate) fn parent_index(sid: u32, lid: u32) -> KiwiValue {
    parent_index_pos(sid, lid, "!")
}

/// A `ParentIndex` carrying an explicit `position` — Figma's fractional-index
/// sibling-order string (compared LEXICOGRAPHICALLY). Lets a test set a
/// per-child z-order distinct from the NodeChange stream order so the
/// position-driven sort in [`fig_to_doc`]'s attach pass is exercised.
pub(crate) fn parent_index_pos(sid: u32, lid: u32, position: &str) -> KiwiValue {
    o(
        "ParentIndex",
        vec![
            ("guid", guid(sid, lid)),
            ("position", KiwiValue::String(position.to_owned())),
        ],
    )
}

/// A `ParentIndex` with NO `position` field at all — exercises the
/// missing-position fallback (stable stream order).
pub(crate) fn parent_index_no_pos(sid: u32, lid: u32) -> KiwiValue {
    o("ParentIndex", vec![("guid", guid(sid, lid))])
}

pub(crate) fn vector(x: f64, y: f64) -> KiwiValue {
    o(
        "Vector",
        vec![
            ("x", KiwiValue::Float(x as f32)),
            ("y", KiwiValue::Float(y as f32)),
        ],
    )
}

pub(crate) fn color(r: f32, g: f32, b: f32, a: f32) -> KiwiValue {
    o(
        "Color",
        vec![
            ("r", KiwiValue::Float(r)),
            ("g", KiwiValue::Float(g)),
            ("b", KiwiValue::Float(b)),
            ("a", KiwiValue::Float(a)),
        ],
    )
}

pub(crate) fn solid_paint(r: f32, g: f32, b: f32, a: f32) -> KiwiValue {
    o(
        "Paint",
        vec![
            ("type", KiwiValue::Enum("SOLID".into())),
            ("color", color(r, g, b, a)),
        ],
    )
}

pub(crate) fn text_data(characters: &str) -> KiwiValue {
    o(
        "TextData",
        vec![("characters", KiwiValue::String(characters.to_owned()))],
    )
}

pub(crate) fn font_name(family: &str, style: &str) -> KiwiValue {
    o(
        "FontName",
        vec![
            ("family", KiwiValue::String(family.to_owned())),
            ("style", KiwiValue::String(style.to_owned())),
            ("postscript", KiwiValue::String(String::new())),
        ],
    )
}

pub(crate) fn number(value: f32, units: &str) -> KiwiValue {
    o(
        "Number",
        vec![
            ("value", KiwiValue::Float(value)),
            ("units", KiwiValue::Enum(units.into())),
        ],
    )
}

pub(crate) fn doc_from(changes: Vec<KiwiValue>) -> FigDocument {
    doc_from_with_blobs(changes, Vec::new())
}

/// Build a doc with an explicit `blobs` table (raw command byte streams) so the
/// geometry-decode tests can index `commandsBlob` into real bytes.
pub(crate) fn doc_from_with_blobs(changes: Vec<KiwiValue>, blobs: Vec<Vec<u8>>) -> FigDocument {
    FigDocument {
        version: 0,
        schema: schema(),
        root: o("Message", vec![("nodeChanges", KiwiValue::Array(changes))]),
        root_type_name: "Message".into(),
        blobs,
        images: std::collections::HashMap::new(),
    }
}

/// Build a doc with an explicit embedded-images table (hash → raw bytes) so the
/// image-fill mapping tests can prove an IMAGE paint resolves to the right
/// `AssetId` and the asset bytes round-trip out of `fig_to_doc`.
pub(crate) fn doc_from_with_images(
    changes: Vec<KiwiValue>,
    images: std::collections::HashMap<String, Vec<u8>>,
) -> FigDocument {
    FigDocument {
        version: 0,
        schema: schema(),
        root: o("Message", vec![("nodeChanges", KiwiValue::Array(changes))]),
        root_type_name: "Message".into(),
        blobs: Vec::new(),
        images,
    }
}

// ---- cross-section builders (shared by ≥2 test modules) ----

pub(crate) fn symbol_data(sid: u32, lid: u32) -> KiwiValue {
    o("SymbolData", vec![("symbolID", guid(sid, lid))])
}

pub(crate) fn guid_path(sid: u32, lid: u32) -> KiwiValue {
    o(
        "GUIDPath",
        vec![("guids", KiwiValue::Array(vec![guid(sid, lid)]))],
    )
}

/// A `GUIDPath` of two guids: a nested-instance crossing then the target.
pub(crate) fn guid_path2(a: (u32, u32), b: (u32, u32)) -> KiwiValue {
    o(
        "GUIDPath",
        vec![(
            "guids",
            KiwiValue::Array(vec![guid(a.0, a.1), guid(b.0, b.1)]),
        )],
    )
}

/// A `ComponentPropValue` carrying overridden text.
pub(crate) fn prop_value_text(text: &str) -> KiwiValue {
    o("ComponentPropValue", vec![("textValue", text_data(text))])
}

/// A `VariableData` (a component prop-def's `varValue`) carrying a String default.
pub(crate) fn var_value_text(text: &str) -> KiwiValue {
    o(
        "VariableData",
        vec![(
            "value",
            o(
                "VariableAnyValue",
                vec![("textValue", KiwiValue::String(text.to_owned()))],
            ),
        )],
    )
}

/// A `VariableData` carrying a Boolean default.
pub(crate) fn var_value_bool(value: bool) -> KiwiValue {
    o(
        "VariableData",
        vec![(
            "value",
            o(
                "VariableAnyValue",
                vec![("boolValue", KiwiValue::Bool(value))],
            ),
        )],
    )
}

/// A `ComponentPropDef` of `ty` (TEXT/BOOL/INSTANCE_SWAP/VARIANT) with `id`,
/// `name`, and a `varValue` default.
pub(crate) fn prop_def(id: KiwiValue, name: &str, ty: &str, default: KiwiValue) -> KiwiValue {
    o(
        "ComponentPropDef",
        vec![
            ("id", id),
            ("name", KiwiValue::String(name.to_owned())),
            ("type", KiwiValue::Enum(ty.into())),
            ("varValue", default),
        ],
    )
}

/// A thin (variant-member) `ComponentPropDef` carrying only an `id` and a
/// `parentPropDefId` — the shape a component-set member's local prop def takes
/// (its name/type/default live on the set-level parent it inherits from).
pub(crate) fn thin_prop_def(id: KiwiValue, parent: KiwiValue) -> KiwiValue {
    o(
        "ComponentPropDef",
        vec![("id", id), ("parentPropDefId", parent)],
    )
}

/// A `ComponentPropRef` binding a node field (e.g. `"VISIBLE"`) to a prop-def.
pub(crate) fn prop_ref(def_id: KiwiValue, node_field: &str) -> KiwiValue {
    o(
        "ComponentPropRef",
        vec![
            ("defID", def_id),
            ("componentPropNodeField", KiwiValue::Enum(node_field.into())),
        ],
    )
}

/// Find a node by name in the scene and return whether it is HIDDEN.
pub(crate) fn node_hidden(doc: &Doc, name: &str) -> Option<bool> {
    doc.scene
        .roots()
        .iter()
        .flat_map(|r| doc.scene.descendants_of(*r))
        .find_map(|id| {
            let n = doc.scene.get(id)?;
            (n.name == name).then(|| n.flags.contains(fanta_doc::NodeFlags::HIDDEN))
        })
}

/// Find the instance node in the scene and expand it against the components.
pub(crate) fn expand_only_instance(doc: &Doc) -> Vec<fanta_doc::resolve::ExpandedNode> {
    let inst = doc
        .scene
        .roots()
        .iter()
        .flat_map(|r| doc.scene.descendants_of(*r))
        .find_map(|id| match doc.scene.get(id).map(|n| &n.data) {
            Some(NodeData::Instance(i)) => Some(i.clone()),
            _ => None,
        })
        .expect("an instance is present");
    expand_instance(&doc.scene, &doc.components, &inst)
}

pub(crate) fn nested_component_doc(card_instance_extra: Vec<(&str, KiwiValue)>) -> FigDocument {
    let mut card_instance_fields = vec![
        ("guid", guid(0, 20)),
        ("type", KiwiValue::Enum("INSTANCE".into())),
        ("name", KiwiValue::String("Card instance".to_owned())),
        ("size", vector(120.0, 40.0)),
        (
            "symbolData",
            o("SymbolData", vec![("symbolID", guid(0, 10))]),
        ),
    ];
    card_instance_fields.extend(card_instance_extra);

    doc_from(vec![
        // Inner master "Chip".
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("Chip".to_owned())),
                ("size", vector(60.0, 20.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("TEXT".into())),
                ("name", KiwiValue::String("Inner".to_owned())),
                ("size", vector(60.0, 16.0)),
                ("textData", text_data("Inner")),
                ("fontSize", KiwiValue::Float(12.0)),
            ],
        ),
        // Outer master "Card" holding an INSTANCE of Chip.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 10)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("Card".to_owned())),
                ("size", vector(120.0, 40.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 11)),
                ("parentIndex", parent_index(0, 10)),
                ("type", KiwiValue::Enum("INSTANCE".into())),
                ("name", KiwiValue::String("Nested chip".to_owned())),
                ("size", vector(60.0, 20.0)),
                (
                    "symbolData",
                    o("SymbolData", vec![("symbolID", guid(0, 1))]),
                ),
            ],
        ),
        // Top-level INSTANCE of Card.
        o("NodeChange", card_instance_fields),
    ])
}

pub(crate) fn prop_value_bool(value: bool) -> KiwiValue {
    o(
        "ComponentPropValue",
        vec![("boolValue", KiwiValue::Bool(value))],
    )
}

/// A `ComponentPropValue` carrying a `guidValue` (an INSTANCE_SWAP prop — the
/// chosen swap target's symbol guid).
pub(crate) fn prop_value_guid(sid: u32, lid: u32) -> KiwiValue {
    o("ComponentPropValue", vec![("guidValue", guid(sid, lid))])
}

pub(crate) fn matrix(m: [f32; 6]) -> KiwiValue {
    o(
        "Matrix",
        vec![
            ("m00", KiwiValue::Float(m[0])),
            ("m01", KiwiValue::Float(m[1])),
            ("m02", KiwiValue::Float(m[2])),
            ("m10", KiwiValue::Float(m[3])),
            ("m11", KiwiValue::Float(m[4])),
            ("m12", KiwiValue::Float(m[5])),
        ],
    )
}

// ---- element-fidelity builders (shared across gradient/stroke/effect modules) ----

pub(crate) fn grad_stop(position: f32, r: f32, g: f32, b: f32, a: f32) -> KiwiValue {
    o(
        "ColorStop",
        vec![
            ("position", KiwiValue::Float(position)),
            ("color", color(r, g, b, a)),
        ],
    )
}

/// A gradient `Paint` of `kind` with the given 2x3 transform and stops.
pub(crate) fn gradient_paint(kind: &str, m: [f32; 6], stops: Vec<KiwiValue>) -> KiwiValue {
    let transform = o(
        "Matrix",
        vec![
            ("m00", KiwiValue::Float(m[0])),
            ("m01", KiwiValue::Float(m[1])),
            ("m02", KiwiValue::Float(m[2])),
            ("m10", KiwiValue::Float(m[3])),
            ("m11", KiwiValue::Float(m[4])),
            ("m12", KiwiValue::Float(m[5])),
        ],
    );
    o(
        "Paint",
        vec![
            ("type", KiwiValue::Enum(kind.into())),
            ("transform", transform),
            ("stops", KiwiValue::Array(stops)),
        ],
    )
}

/// Wrap one RECTANGLE-ish node change under a CANVAS so it lands in the scene.
pub(crate) fn doc_with_shape(shape: KiwiValue) -> FigDocument {
    doc_from(vec![
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
            ],
        ),
        shape,
    ])
}

pub(crate) fn first_vector(doc: &Doc) -> VectorNode {
    for root in doc.scene.roots().to_vec() {
        for id in doc.scene.descendants_of(root) {
            if let Some(NodeData::Vector(v)) = doc.scene.get(id).map(|n| &n.data) {
                return v.clone();
            }
        }
    }
    panic!("no vector node in scene");
}

pub(crate) fn first_node_with_vector(doc: &Doc) -> CanvasNode {
    for root in doc.scene.roots().to_vec() {
        for id in doc.scene.descendants_of(root) {
            if let Some(n) = doc.scene.get(id) {
                if matches!(n.data, NodeData::Vector(_)) {
                    return n.clone();
                }
            }
        }
    }
    panic!("no vector node in scene");
}

// ---- cross-section builders moved from per-section test files ----

pub(crate) fn fig_path(commands_blob: u32, winding: &str) -> KiwiValue {
    o(
        "Path",
        vec![
            ("windingRule", KiwiValue::Enum(winding.into())),
            ("commandsBlob", KiwiValue::Uint(commands_blob)),
            ("styleID", KiwiValue::Uint(0)),
        ],
    )
}
pub(crate) fn cmd(verb: u8, coords: &[f32]) -> Vec<u8> {
    let mut v = vec![verb];
    for c in coords {
        v.extend_from_slice(&c.to_le_bytes());
    }
    v
}
pub(crate) fn group_named(doc: &Doc, name: &str) -> fanta_doc::node::GroupNode {
    for root in doc.scene.roots().to_vec() {
        for id in doc.scene.descendants_of(root) {
            let Some(n) = doc.scene.get(id) else { continue };
            if n.name == name {
                if let NodeData::Group(g) = &n.data {
                    return g.clone();
                }
            }
        }
    }
    panic!("no group named {name}");
}
pub(crate) fn variable_id(sid: u32, lid: u32) -> KiwiValue {
    o("VariableID", vec![("guid", guid(sid, lid))])
}
