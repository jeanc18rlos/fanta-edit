//! The Kiwi binary serialization codec.
//!
//! Kiwi (github.com/evanw/kiwi) is the wire format underneath Figma's `.fig`
//! files. This module is a faithful Rust port of the reference implementation's
//! wire format — encode and decode for every primitive, the schema-driven
//! compound model (enum / struct / message / array), the *self-describing*
//! binary schema (a Kiwi file embeds the schema that decodes its own body), and
//! a dynamic [`KiwiValue`] tree so a `.fig` can be parsed generically without
//! compiling its schema into Rust types.
//!
//! ## Why a dynamic value model rather than generated structs
//!
//! Kiwi's first-party tooling generates a struct per schema definition. We
//! cannot do that here: Figma's schema is enormous (hundreds of message types),
//! versioned per file, and *embedded in each document*. The only tractable way
//! to read an arbitrary `.fig` is to decode its embedded schema at runtime and
//! walk the bytes into a dynamic tree. [`KiwiValue::decode`] does exactly that.
//!
//! ## The wire format, precisely (ported from the reference)
//!
//! - `bool` — 1 byte, `0`/`1`.
//! - `byte` — 1 byte verbatim.
//! - `uint` — LEB128 varint: 7 payload bits per byte, low byte first, high bit
//!   (`0x80`) is the continuation flag. At most 5 bytes for a `u32`.
//! - `int` — zig-zag mapped (`(n << 1) ^ (n >> 31)`) then written as `uint`, so
//!   small-magnitude negatives stay short.
//! - `float` — the compact float: reinterpret the `f32` as bits, rotate the
//!   8-bit exponent down into the low byte (`bits.rotate_right(23)`); if the low
//!   byte is then `0` (zero and subnormals) emit a single `0x00`, otherwise emit
//!   the 4 rotated bytes little-endian. Decode reverses the rotation. This is
//!   why `0.0` costs one byte — the single most common float in a design doc.
//! - `string` — UTF-8 bytes followed by a `0x00` terminator.
//! - `enum` — the field's `uint` value (not its declaration index).
//! - `struct` — every field encoded in declaration order, no ids, all present.
//! - `message` — a sequence of (`uint` field-id, value) pairs ended by a `0x00`
//!   field-id. Fields are optional and may appear in any order; an unknown id is
//!   skipped *by consulting the schema for its type*. This is Kiwi's forward-
//!   compatibility story and the reason the schema must travel with the data.
//! - `array` — a `uint` length followed by that many elements of the element
//!   type.
//!
//! Round-trip equality (`decode(encode(v)) == v`) is the correctness anchor for
//! the test suite, since we have no guaranteed real `.fig` fixture to diff
//! against.

pub mod reader;
pub mod schema;
pub mod types;
pub mod value;
pub mod writer;

pub use reader::ByteReader;
pub use schema::{Def, DefKind, Field, Schema};
pub use types::{
    KiwiType, TYPE_BOOL, TYPE_BYTE, TYPE_FLOAT, TYPE_INT, TYPE_INT64, TYPE_STRING, TYPE_UINT,
    TYPE_UINT64,
};
pub use value::KiwiValue;
pub use writer::ByteWriter;

#[cfg(test)]
pub(crate) mod test_support {
    //! Shared fixtures for the codec tests, split across `schema.rs` and
    //! `value.rs` after the module was promoted to a folder.

    use super::{Def, DefKind, Field, KiwiType, KiwiValue, Schema};
    use std::collections::HashMap;

    /// A small schema exercising every def kind, builtin, array, and nesting.
    ///
    /// ```text
    /// enum Shape { RECT = 1; ELLIPSE = 2; }
    /// struct Vec2 { float x; float y; }
    /// message Node {
    ///   string name = 1;
    ///   Shape shape = 2;
    ///   Vec2 size = 3;
    ///   Node[] children = 4;
    ///   int z = 5;
    /// }
    /// ```
    pub(crate) fn sample_schema() -> Schema {
        Schema::new(vec![
            Def::new(
                "Shape",
                DefKind::Enum,
                vec![
                    Field::new("RECT", KiwiType(0), 1),
                    Field::new("ELLIPSE", KiwiType(0), 2),
                ],
            ),
            Def::new(
                "Vec2",
                DefKind::Struct,
                vec![
                    Field::new("x", KiwiType::FLOAT, 0),
                    Field::new("y", KiwiType::FLOAT, 0),
                ],
            ),
            Def::new(
                "Node",
                DefKind::Message,
                vec![
                    Field::new("name", KiwiType::STRING, 1),
                    Field::new("shape", KiwiType::user(0), 2),
                    Field::new("size", KiwiType::user(1), 3),
                    Field::array("children", KiwiType::user(2), 4),
                    Field::new("z", KiwiType::INT, 5),
                ],
            ),
        ])
    }

    pub(crate) fn obj(type_name: &str, fields: Vec<(&str, KiwiValue)>) -> KiwiValue {
        KiwiValue::Object {
            type_name: type_name.to_owned(),
            fields: fields
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v))
                .collect::<HashMap<_, _>>(),
        }
    }
}
