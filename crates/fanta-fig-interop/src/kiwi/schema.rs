//! The Kiwi schema model — [`DefKind`], [`Field`], [`Def`], [`Schema`] — and the
//! self-describing binary-schema codec.

use crate::error::{FigError, FigResult};
use crate::kiwi::reader::ByteReader;
use crate::kiwi::types::{KiwiType, TYPE_MIN};
use crate::kiwi::writer::ByteWriter;
use std::collections::HashMap;

// =============================================================================
// Schema model
// =============================================================================

/// The kind of a Kiwi definition. Drives how its fields are encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefKind {
    /// A set of named integer constants, encoded as a single `uint`.
    Enum,
    /// A fixed tuple of fields, all present, encoded in declaration order.
    Struct,
    /// An open set of optional fields, each prefixed by a nonzero id, ended by
    /// a `0x00` id. Forward-compatible: unknown ids are skipped.
    Message,
}

/// The byte used for [`DefKind::Enum`] in the binary schema.
const DEF_ENUM: u8 = 0;
/// The byte used for [`DefKind::Struct`] in the binary schema.
const DEF_STRUCT: u8 = 1;
/// The byte used for [`DefKind::Message`] in the binary schema.
const DEF_MESSAGE: u8 = 2;

/// One field of a [`Def`].
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// Field name (the map key in a decoded [`KiwiValue::Object`]).
    ///
    /// [`KiwiValue::Object`]: crate::kiwi::KiwiValue::Object
    pub name: std::sync::Arc<str>,
    /// The field's type. For enum members this is unused (the reference writes
    /// `0`); the constant lives in [`Field::value`].
    pub ty: KiwiType,
    /// Whether the field is an array (`T[]`), encoded length-prefixed.
    pub is_array: bool,
    /// Dual-purpose per the def kind: the *enum constant* for enum members, the
    /// *field id* for message fields, and meaningless (conventionally `0`) for
    /// struct fields. Named `value` to match the reference and the wire schema.
    pub value: u32,
}

impl Field {
    /// A non-array message/struct field of the given builtin or user type.
    pub fn new(name: impl Into<String>, ty: KiwiType, value: u32) -> Self {
        Self {
            name: std::sync::Arc::from(name.into()),
            ty,
            is_array: false,
            value,
        }
    }

    /// An array field (`ty[]`).
    pub fn array(name: impl Into<String>, ty: KiwiType, value: u32) -> Self {
        Self {
            name: std::sync::Arc::from(name.into()),
            ty,
            is_array: true,
            value,
        }
    }
}

/// One definition (enum, struct, or message) in a [`Schema`].
#[derive(Debug, Clone, PartialEq)]
pub struct Def {
    pub name: std::sync::Arc<str>,
    pub kind: DefKind,
    pub fields: Vec<Field>,
    /// `value` (enum constant / message field-id) -> index into `fields`.
    /// Built on construction so decode can resolve a wire id to a field in O(1)
    /// without scanning, exactly like the reference's `field_value_to_index`.
    value_to_index: HashMap<u32, usize>,
    /// field name -> index into `fields`, for encode-side lookups.
    name_to_index: HashMap<String, usize>,
}

impl Def {
    pub fn new(name: impl Into<String>, kind: DefKind, fields: Vec<Field>) -> Self {
        let mut value_to_index = HashMap::with_capacity(fields.len());
        let mut name_to_index = HashMap::with_capacity(fields.len());
        for (i, f) in fields.iter().enumerate() {
            value_to_index.insert(f.value, i);
            name_to_index.insert(f.name.to_string(), i);
        }
        Self {
            name: std::sync::Arc::from(name.into()),
            kind,
            fields,
            value_to_index,
            name_to_index,
        }
    }

    /// The field carrying message/enum `value`, if any.
    pub(crate) fn field_by_value(&self, value: u32) -> Option<&Field> {
        self.value_to_index.get(&value).map(|&i| &self.fields[i])
    }

    /// Look up a field by name.
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.name_to_index.get(name).map(|&i| &self.fields[i])
    }
}

/// A complete Kiwi schema: an ordered list of definitions.
///
/// The order is load-bearing — a field's user type is encoded as the index of
/// its definition in this list, so re-ordering changes the wire meaning. The
/// `name_to_index` map supports encode-side lookups by name.
#[derive(Debug, Clone, PartialEq)]
pub struct Schema {
    pub defs: Vec<Def>,
    name_to_index: HashMap<String, usize>,
}

impl Schema {
    pub fn new(defs: Vec<Def>) -> Self {
        let mut name_to_index = HashMap::with_capacity(defs.len());
        for (i, d) in defs.iter().enumerate() {
            name_to_index.insert(d.name.to_string(), i);
        }
        Self {
            defs,
            name_to_index,
        }
    }

    /// The definition with the given name, if present.
    pub fn def(&self, name: &str) -> Option<&Def> {
        self.name_to_index.get(name).map(|&i| &self.defs[i])
    }

    /// The index of the definition with the given name, if present.
    pub fn def_index(&self, name: &str) -> Option<usize> {
        self.name_to_index.get(name).copied()
    }

    // ---- binary schema codec ------------------------------------------------
    //
    // Kiwi schemas serialize themselves through a fixed meta-layout. This is
    // what lets a `.fig` embed its own schema: chunk 1 of the file is exactly
    // the output of `encode_binary`, and we feed it to `decode_binary` to learn
    // how to read chunk 2. The layout (per the reference `binary.ts` /
    // `lib.rs`):
    //
    //   uint   definition_count
    //   repeat definition_count times:
    //     string name
    //     byte   kind            (0=enum, 1=struct, 2=message)
    //     uint   field_count
    //     repeat field_count times:
    //       string name
    //       int    type_id       (zig-zag; negative=builtin, >=0=def index)
    //       byte   is_array       (low bit)
    //       uint   value          (enum constant / message field-id)

    /// Decode a binary-encoded Kiwi schema.
    pub fn decode_binary(bytes: &[u8]) -> FigResult<Schema> {
        let mut r = ByteReader::new(bytes);
        let def_count = r.read_var_uint()?;
        // Each definition takes at least three bytes; a larger count is corrupt
        // and must not size the allocation.
        if def_count as usize > r.remaining() {
            return Err(FigError::Truncated);
        }
        let mut defs = Vec::with_capacity(def_count as usize);

        for _ in 0..def_count {
            let name = r.read_string()?;
            let kind = match r.read_byte()? {
                DEF_ENUM => DefKind::Enum,
                DEF_STRUCT => DefKind::Struct,
                DEF_MESSAGE => DefKind::Message,
                other => {
                    return Err(FigError::Schema(format!(
                        "unknown definition kind byte {other}"
                    )));
                }
            };
            let field_count = r.read_var_uint()?;
            if field_count as usize > r.remaining() {
                return Err(FigError::Truncated);
            }
            let mut fields = Vec::with_capacity(field_count as usize);
            for _ in 0..field_count {
                let fname = r.read_string()?;
                let type_id = r.read_var_int()?;
                let is_array = r.read_byte()? & 1 != 0;
                let value = r.read_var_uint()?;
                // Validate the type id is in range now, so later decoding can
                // trust it. Mirrors the reference's bounds check.
                if type_id < TYPE_MIN || type_id >= def_count as i32 {
                    return Err(FigError::Schema(format!(
                        "field '{fname}' references out-of-range type id {type_id}"
                    )));
                }
                fields.push(Field {
                    name: std::sync::Arc::from(fname),
                    ty: KiwiType(type_id),
                    is_array,
                    value,
                });
            }
            defs.push(Def::new(name, kind, fields));
        }

        Ok(Schema::new(defs))
    }

    /// Encode this schema to the binary schema format (inverse of
    /// [`Schema::decode_binary`]).
    pub fn encode_binary(&self) -> Vec<u8> {
        let mut w = ByteWriter::new();
        w.write_var_uint(self.defs.len() as u32);
        for def in &self.defs {
            w.write_string(&def.name);
            w.write_byte(match def.kind {
                DefKind::Enum => DEF_ENUM,
                DefKind::Struct => DEF_STRUCT,
                DefKind::Message => DEF_MESSAGE,
            });
            w.write_var_uint(def.fields.len() as u32);
            for f in &def.fields {
                w.write_string(&f.name);
                w.write_var_int(f.ty.0);
                w.write_bool(f.is_array);
                w.write_var_uint(f.value);
            }
        }
        w.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kiwi::test_support::sample_schema;

    // ---- binary schema round-trip ------------------------------------------

    #[test]
    fn binary_schema_matches_reference_abc_vector() {
        // The reference's documented encoding of "message ABC { int[] xyz = 1; }".
        let schema_bytes = [1, 65, 66, 67, 0, 2, 1, 120, 121, 122, 0, 5, 1, 1];
        let schema = Schema::decode_binary(&schema_bytes).unwrap();
        assert_eq!(schema.defs.len(), 1);
        let def = schema.def("ABC").unwrap();
        assert_eq!(def.kind, DefKind::Message);
        let f = def.field("xyz").unwrap();
        assert_eq!(f.ty, KiwiType::INT);
        assert!(f.is_array);
        assert_eq!(f.value, 1);
        // And re-encoding reproduces the exact bytes.
        assert_eq!(schema.encode_binary(), schema_bytes);
    }

    #[test]
    fn binary_schema_round_trips_all_kinds() {
        let schema = sample_schema();
        let bytes = schema.encode_binary();
        let back = Schema::decode_binary(&bytes).unwrap();
        assert_eq!(schema, back);
    }

    #[test]
    fn binary_schema_rejects_bad_kind_byte() {
        // def_count=1, name="X\0", kind=9 (invalid), ...
        let bytes = [1, b'X', 0, 9, 0];
        assert!(matches!(
            Schema::decode_binary(&bytes),
            Err(FigError::Schema(_))
        ));
    }

    #[test]
    fn binary_schema_counts_larger_than_the_data_are_refused_before_allocating() {
        // def_count = u32::MAX in five bytes.
        let bytes = [0xFF, 0xFF, 0xFF, 0xFF, 0x0F];
        assert!(matches!(
            Schema::decode_binary(&bytes),
            Err(FigError::Truncated)
        ));
        // One message, "M", claiming u32::MAX fields.
        let bytes = [1, b'M', 0, 2, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F];
        assert!(matches!(
            Schema::decode_binary(&bytes),
            Err(FigError::Truncated)
        ));
    }

    #[test]
    fn binary_schema_rejects_out_of_range_type() {
        // def_count=1, "M\0", kind=2 (message), field_count=1, "f\0",
        // type_id = zig-zag(50) = 100 (way past def_count), is_array=0, value=1.
        let bytes = [1, b'M', 0, 2, 1, b'f', 0, 100, 0, 1];
        assert!(matches!(
            Schema::decode_binary(&bytes),
            Err(FigError::Schema(_))
        ));
    }
}
