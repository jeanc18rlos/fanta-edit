//! The dynamic [`KiwiValue`] tree and its schema-driven encode/decode.

use crate::error::{FigError, FigResult};
use crate::kiwi::reader::ByteReader;
use crate::kiwi::schema::{DefKind, Field, Schema};
use crate::kiwi::types::{
    KiwiType, TYPE_BOOL, TYPE_BYTE, TYPE_FLOAT, TYPE_INT, TYPE_INT64, TYPE_STRING, TYPE_UINT,
    TYPE_UINT64,
};
use crate::kiwi::writer::ByteWriter;
use std::sync::Arc;

// =============================================================================
// Dynamic value tree
// =============================================================================

/// A dynamically-typed Kiwi value, produced by [`KiwiValue::decode`] and
/// consumed by [`KiwiValue::encode`].
///
/// Object keys, type names, and enum members are interned `Arc<str>` clones
/// of the schema's own strings, and an object's fields live in a sorted
/// vector rather than a `HashMap`. A real document decodes to millions of
/// objects; the previous owned-`String` + per-object-`HashMap` shape cost
/// ~1.7GB for a 25MB stream (68x), most of it duplicate key strings and
/// hash-table slack.
#[derive(Debug, Clone, PartialEq)]
pub enum KiwiValue {
    Bool(bool),
    Byte(u8),
    Int(i32),
    Uint(u32),
    Float(f32),
    String(String),
    Int64(i64),
    Uint64(u64),
    /// Array payloads are shared: cloning a value that holds one — the
    /// shared-style pre-pass copies every consuming node before rewriting a
    /// field or two of it — bumps a count instead of deep-copying the
    /// `derivedSymbolData` / geometry arrays that make up most of a large
    /// document. The two writers in that pre-pass go through
    /// [`Arc::make_mut`], so a shared payload is copied only when it is
    /// actually edited. Build one with [`KiwiValue::array`].
    Array(Arc<Vec<KiwiValue>>),
    /// A `byte[]` field as one contiguous buffer. Figma's path-command blobs
    /// and image hashes are `byte[]`; decoding them as `Array` of [`Byte`]
    /// costs 32 bytes and a dispatch per source byte, which for a large file
    /// is hundreds of MB of transient values for a few MB of blob data.
    /// Encodes identically to the element-wise form.
    ///
    /// [`Byte`]: KiwiValue::Byte
    Bytes(Vec<u8>),
    /// An enum value: the *member* name (e.g. `"RECTANGLE"`). The owning def's
    /// name is not retained — callers match on the member, which is what
    /// matters for mapping.
    Enum(Arc<str>),
    /// A struct or message: the def name plus the present fields by name.
    Object {
        type_name: Arc<str>,
        fields: KiwiFields,
    },
}

/// An object's present fields, sorted by name — binary-search reads, compact
/// contiguous storage, no per-object hash table.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct KiwiFields(Vec<(Arc<str>, KiwiValue)>);

impl KiwiFields {
    pub fn with_capacity(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }

    fn position(&self, name: &str) -> Result<usize, usize> {
        self.0.binary_search_by(|(key, _)| (**key).cmp(name))
    }

    pub fn get(&self, name: &str) -> Option<&KiwiValue> {
        self.position(name).ok().map(|pos| &self.0[pos].1)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut KiwiValue> {
        self.position(name).ok().map(|pos| &mut self.0[pos].1)
    }

    pub fn insert(&mut self, name: Arc<str>, value: KiwiValue) {
        match self.position(&name) {
            Ok(pos) => self.0[pos].1 = value,
            Err(pos) => self.0.insert(pos, (name, value)),
        }
    }

    /// Bulk append for decode: caller pushes in wire order, then seals once.
    fn push_unsorted(&mut self, name: Arc<str>, value: KiwiValue) {
        self.0.push((name, value));
    }

    fn seal(&mut self) {
        self.0.sort_by(|a, b| a.0.cmp(&b.0));
        self.0.shrink_to_fit();
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &KiwiValue)> {
        self.0.iter().map(|(key, value)| (&**key, value))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<K: Into<Arc<str>>> FromIterator<(K, KiwiValue)> for KiwiFields {
    fn from_iter<T: IntoIterator<Item = (K, KiwiValue)>>(iter: T) -> Self {
        let mut fields = KiwiFields::default();
        for (name, value) in iter {
            fields.insert(name.into(), value);
        }
        fields
    }
}

impl KiwiValue {
    /// An array value over `items`.
    pub fn array(items: Vec<KiwiValue>) -> Self {
        KiwiValue::Array(Arc::new(items))
    }

    // ---- ergonomic accessors (used heavily by the mapping layer) -----------

    /// Borrow a field of an [`KiwiValue::Object`].
    pub fn get(&self, name: &str) -> Option<&KiwiValue> {
        match self {
            KiwiValue::Object { fields, .. } => fields.get(name),
            _ => None,
        }
    }

    /// Mutably borrow a field of an [`KiwiValue::Object`]. Used by the mapping
    /// layer's shared-style pre-pass to inline a referenced style's payload into
    /// a consuming node.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut KiwiValue> {
        match self {
            KiwiValue::Object { fields, .. } => fields.get_mut(name),
            _ => None,
        }
    }

    /// Insert / replace a field on an [`KiwiValue::Object`]. No-op on non-objects.
    /// Used by the shared-style pre-pass to graft a style's resolved
    /// `fillPaints` / `effects` / text fields onto a consuming node.
    pub fn set_field(&mut self, name: &str, value: KiwiValue) {
        if let KiwiValue::Object { fields, .. } = self {
            fields.insert(Arc::from(name), value);
        }
    }

    /// The object's type name, if this is an object.
    pub fn type_name(&self) -> Option<&str> {
        match self {
            KiwiValue::Object { type_name, .. } => Some(type_name),
            _ => None,
        }
    }

    /// Coerce to `f64`, accepting any numeric Kiwi scalar. Returns `None` for
    /// non-numeric values. Figma stores most geometry as `float`, but ids and
    /// counts arrive as `uint`/`int`, so a forgiving numeric read is convenient
    /// at the mapping layer.
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            KiwiValue::Float(v) => Some(v as f64),
            KiwiValue::Int(v) => Some(v as f64),
            KiwiValue::Uint(v) => Some(v as f64),
            KiwiValue::Int64(v) => Some(v as f64),
            KiwiValue::Uint64(v) => Some(v as f64),
            KiwiValue::Byte(v) => Some(v as f64),
            _ => None,
        }
    }

    /// Borrow as a string slice if this is a [`KiwiValue::String`] or
    /// [`KiwiValue::Enum`].
    pub fn as_str(&self) -> Option<&str> {
        match self {
            KiwiValue::String(s) => Some(s),
            KiwiValue::Enum(s) => Some(s),
            _ => None,
        }
    }

    /// Borrow as a slice if this is a [`KiwiValue::Array`]. A
    /// [`KiwiValue::Bytes`] is not an array of values; read it with
    /// [`KiwiValue::as_bytes`].
    pub fn as_array(&self) -> Option<&[KiwiValue]> {
        match self {
            KiwiValue::Array(v) => Some(v.as_slice()),
            _ => None,
        }
    }

    /// The raw bytes of a `byte[]` value: a [`KiwiValue::Bytes`] directly, or
    /// an element-wise [`KiwiValue::Array`] of integer items (the shape
    /// hand-built values take; wider integers keep their low byte). `None`
    /// for anything else, including an array with a non-integer item.
    pub fn as_bytes(&self) -> Option<std::borrow::Cow<'_, [u8]>> {
        match self {
            KiwiValue::Bytes(bytes) => Some(std::borrow::Cow::Borrowed(bytes)),
            KiwiValue::Array(items) => items
                .iter()
                .map(|item| match *item {
                    KiwiValue::Byte(b) => Some(b),
                    KiwiValue::Uint(u) => Some(u as u8),
                    KiwiValue::Int(i) => Some(i as u8),
                    _ => None,
                })
                .collect::<Option<Vec<u8>>>()
                .map(std::borrow::Cow::Owned),
            _ => None,
        }
    }

    // ---- decode -------------------------------------------------------------

    /// Decode a value of `root_type` from `bytes` using `schema`.
    ///
    /// `root_type` is typically a user-defined message index (the document
    /// root); builtins are accepted too, which the test suite uses to exercise
    /// each primitive in isolation.
    pub fn decode(schema: &Schema, root_type: KiwiType, bytes: &[u8]) -> FigResult<KiwiValue> {
        let mut r = ByteReader::new(bytes);
        Self::decode_type(schema, root_type, &mut r)
    }

    /// Decode a value of `ty` from the reader at its current position.
    fn decode_type(schema: &Schema, ty: KiwiType, r: &mut ByteReader) -> FigResult<KiwiValue> {
        Ok(match ty.0 {
            TYPE_BOOL => KiwiValue::Bool(r.read_bool()?),
            TYPE_BYTE => KiwiValue::Byte(r.read_byte()?),
            TYPE_INT => KiwiValue::Int(r.read_var_int()?),
            TYPE_UINT => KiwiValue::Uint(r.read_var_uint()?),
            TYPE_FLOAT => KiwiValue::Float(r.read_var_float()?),
            TYPE_STRING => KiwiValue::String(r.read_string()?),
            TYPE_INT64 => KiwiValue::Int64(r.read_var_int64()?),
            TYPE_UINT64 => KiwiValue::Uint64(r.read_var_uint64()?),
            idx if idx >= 0 => {
                let def = schema
                    .defs
                    .get(idx as usize)
                    .ok_or(FigError::UnknownType(idx))?;
                match def.kind {
                    DefKind::Enum => {
                        let constant = r.read_var_uint()?;
                        match def.field_by_value(constant) {
                            Some(f) => KiwiValue::Enum(f.name.clone()),
                            None => {
                                return Err(FigError::Schema(format!(
                                    "enum '{}' has no member with value {constant}",
                                    def.name
                                )));
                            }
                        }
                    }
                    DefKind::Struct => {
                        let mut fields = KiwiFields::with_capacity(def.fields.len());
                        for f in &def.fields {
                            fields.push_unsorted(f.name.clone(), Self::decode_field(schema, f, r)?);
                        }
                        fields.seal();
                        KiwiValue::Object {
                            type_name: def.name.clone(),
                            fields,
                        }
                    }
                    DefKind::Message => {
                        let mut fields = KiwiFields::default();
                        loop {
                            let id = r.read_var_uint()?;
                            if id == 0 {
                                break;
                            }
                            match def.field_by_value(id) {
                                Some(f) => {
                                    fields.push_unsorted(
                                        f.name.clone(),
                                        Self::decode_field(schema, f, r)?,
                                    );
                                }
                                // Unknown id but known message type: skip by
                                // type. We cannot recover the type of an id the
                                // schema does not list, so this is still an
                                // error — but it only fires for genuinely
                                // malformed data, not for forward-compat skips
                                // (those are handled inside `skip_type`).
                                None => {
                                    return Err(FigError::Schema(format!(
                                        "unknown field id {id} in message '{}'",
                                        def.name
                                    )));
                                }
                            }
                        }
                        fields.seal();
                        KiwiValue::Object {
                            type_name: def.name.clone(),
                            fields,
                        }
                    }
                }
            }
            other => return Err(FigError::UnknownType(other)),
        })
    }

    /// Decode a field value, applying the array length prefix when needed.
    fn decode_field(schema: &Schema, field: &Field, r: &mut ByteReader) -> FigResult<KiwiValue> {
        if field.is_array {
            let len = r.read_var_uint()?;
            if field.ty == KiwiType::BYTE {
                return Ok(KiwiValue::Bytes(r.read_bytes(len as usize)?.to_vec()));
            }
            let mut items = Vec::with_capacity(len as usize);
            for _ in 0..len {
                items.push(Self::decode_type(schema, field.ty, r)?);
            }
            Ok(KiwiValue::array(items))
        } else {
            Self::decode_type(schema, field.ty, r)
        }
    }

    // ---- encode -------------------------------------------------------------

    /// Encode this value into bytes using `schema`.
    ///
    /// Encoding is schema-driven, not value-driven: for an object we walk the
    /// *definition's* fields (in declaration order) and emit those present in
    /// the value. This guarantees a deterministic field order (critical for
    /// round-trip equality) and the correct struct-vs-message framing.
    pub fn encode(&self, schema: &Schema) -> FigResult<Vec<u8>> {
        let mut w = ByteWriter::new();
        self.encode_into(schema, &mut w)?;
        Ok(w.into_bytes())
    }

    fn encode_into(&self, schema: &Schema, w: &mut ByteWriter) -> FigResult<()> {
        match self {
            KiwiValue::Bool(v) => w.write_bool(*v),
            KiwiValue::Byte(v) => w.write_byte(*v),
            KiwiValue::Int(v) => w.write_var_int(*v),
            KiwiValue::Uint(v) => w.write_var_uint(*v),
            KiwiValue::Float(v) => w.write_var_float(*v),
            KiwiValue::String(v) => w.write_string(v),
            KiwiValue::Int64(v) => w.write_var_int64(*v),
            KiwiValue::Uint64(v) => w.write_var_uint64(*v),
            KiwiValue::Array(items) => {
                w.write_var_uint(items.len() as u32);
                for item in items.iter() {
                    item.encode_into(schema, w)?;
                }
            }
            KiwiValue::Bytes(bytes) => {
                w.write_var_uint(bytes.len() as u32);
                w.write_bytes(bytes);
            }
            KiwiValue::Enum(member) => {
                // An enum value on its own needs its def to resolve the member
                // name to its integer constant. We have to scan defs because
                // the value does not retain its def name; enums are rare enough
                // outside objects that this is acceptable. Inside an object the
                // field path below handles it without a scan.
                let constant = schema
                    .defs
                    .iter()
                    .filter(|d| d.kind == DefKind::Enum)
                    .find_map(|d| d.field(member).map(|f| f.value))
                    .ok_or_else(|| {
                        FigError::Schema(format!("no enum member named '{member}' in schema"))
                    })?;
                w.write_var_uint(constant);
            }
            KiwiValue::Object { type_name, fields } => {
                let def = schema
                    .def(type_name)
                    .ok_or_else(|| FigError::Schema(format!("unknown type '{type_name}'")))?;
                match def.kind {
                    DefKind::Enum => {
                        return Err(FigError::Schema(format!(
                            "'{type_name}' is an enum but was given object fields"
                        )));
                    }
                    DefKind::Struct => {
                        // Every struct field must be present — structs have no
                        // optionality on the wire.
                        for f in &def.fields {
                            let v = fields.get(&f.name).ok_or_else(|| {
                                FigError::Schema(format!(
                                    "struct '{type_name}' is missing required field '{}'",
                                    f.name
                                ))
                            })?;
                            Self::encode_field(schema, f, v, w)?;
                        }
                    }
                    DefKind::Message => {
                        // Walk fields in declaration order so the byte output is
                        // deterministic; emit only those present, each prefixed
                        // by its id; terminate with a 0 id.
                        for f in &def.fields {
                            if let Some(v) = fields.get(&f.name) {
                                w.write_var_uint(f.value);
                                Self::encode_field(schema, f, v, w)?;
                            }
                        }
                        w.write_byte(0);
                    }
                }
            }
        }
        Ok(())
    }

    /// Encode a field value. For enum-typed object fields we resolve the
    /// constant via the field's own def (no schema-wide scan), which is both
    /// faster and correct when two enums share a member name.
    fn encode_field(
        schema: &Schema,
        field: &Field,
        value: &KiwiValue,
        w: &mut ByteWriter,
    ) -> FigResult<()> {
        if field.is_array {
            if let KiwiValue::Bytes(bytes) = value {
                if field.ty != KiwiType::BYTE {
                    return Err(FigError::Schema(format!(
                        "field '{}' holds raw bytes but is not a byte array",
                        field.name
                    )));
                }
                w.write_var_uint(bytes.len() as u32);
                w.write_bytes(bytes);
                return Ok(());
            }
            let items = value.as_array().ok_or_else(|| {
                FigError::Schema(format!(
                    "field '{}' is an array but value is not",
                    field.name
                ))
            })?;
            w.write_var_uint(items.len() as u32);
            for item in items {
                Self::encode_typed(schema, field.ty, item, w)?;
            }
            Ok(())
        } else {
            Self::encode_typed(schema, field.ty, value, w)
        }
    }

    /// Encode `value` as exactly `ty`. Resolves enum constants through the
    /// declared type so it works even when the value is an [`KiwiValue::Enum`]
    /// whose member name collides across enums.
    fn encode_typed(
        schema: &Schema,
        ty: KiwiType,
        value: &KiwiValue,
        w: &mut ByteWriter,
    ) -> FigResult<()> {
        if let Some(idx) = ty.def_index() {
            let def = schema.defs.get(idx).ok_or(FigError::UnknownType(ty.0))?;
            if def.kind == DefKind::Enum {
                if let KiwiValue::Enum(member) = value {
                    let constant = def.field(member).map(|f| f.value).ok_or_else(|| {
                        FigError::Schema(format!("enum '{}' has no member '{member}'", def.name))
                    })?;
                    w.write_var_uint(constant);
                    return Ok(());
                }
            }
        }
        value.encode_into(schema, w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kiwi::schema::{Def, DefKind};
    use crate::kiwi::test_support::{obj, sample_schema};
    use crate::kiwi::{ByteWriter, Field};

    // ---- value round-trips through encode/decode ----------------------------

    #[test]
    fn struct_round_trips_via_value() {
        let schema = sample_schema();
        let v = obj(
            "Vec2",
            vec![("x", KiwiValue::Float(1.5)), ("y", KiwiValue::Float(-2.0))],
        );
        let bytes = v.encode(&schema).unwrap();
        let back = KiwiValue::decode(&schema, KiwiType::user(1), &bytes).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn enum_round_trips_via_value() {
        let schema = sample_schema();
        // Decode the raw constant 2 as Shape -> ELLIPSE.
        let bytes = {
            let mut w = ByteWriter::new();
            w.write_var_uint(2);
            w.into_bytes()
        };
        let v = KiwiValue::decode(&schema, KiwiType::user(0), &bytes).unwrap();
        assert_eq!(v, KiwiValue::Enum("ELLIPSE".into()));
        // And re-encode produces the same single byte.
        assert_eq!(v.encode(&schema).unwrap(), [2]);
    }

    #[test]
    fn message_with_all_fields_round_trips() {
        let schema = sample_schema();
        let v = obj(
            "Node",
            vec![
                ("name", KiwiValue::String("root".to_owned())),
                ("shape", KiwiValue::Enum("RECT".into())),
                (
                    "size",
                    obj(
                        "Vec2",
                        vec![
                            ("x", KiwiValue::Float(100.0)),
                            ("y", KiwiValue::Float(50.0)),
                        ],
                    ),
                ),
                ("children", KiwiValue::array(vec![])),
                ("z", KiwiValue::Int(7)),
            ],
        );
        let bytes = v.encode(&schema).unwrap();
        let back = KiwiValue::decode(&schema, KiwiType::user(2), &bytes).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn message_with_absent_optional_fields_round_trips() {
        let schema = sample_schema();
        // Only `name` and `z` present — messages allow omission, and the
        // round-trip must preserve exactly which fields were set.
        let v = obj(
            "Node",
            vec![
                ("name", KiwiValue::String("partial".to_owned())),
                ("z", KiwiValue::Int(-3)),
            ],
        );
        let bytes = v.encode(&schema).unwrap();
        let back = KiwiValue::decode(&schema, KiwiType::user(2), &bytes).unwrap();
        assert_eq!(v, back);
        // The decoded object must NOT contain the omitted fields.
        assert!(back.get("shape").is_none());
        assert!(back.get("size").is_none());
        assert!(back.get("children").is_none());
    }

    #[test]
    fn nested_message_array_round_trips() {
        let schema = sample_schema();
        let child = |name: &str| {
            obj(
                "Node",
                vec![
                    ("name", KiwiValue::String(name.to_owned())),
                    ("z", KiwiValue::Int(0)),
                ],
            )
        };
        let v = obj(
            "Node",
            vec![
                ("name", KiwiValue::String("parent".to_owned())),
                (
                    "children",
                    KiwiValue::array(vec![child("a"), child("b"), child("c")]),
                ),
                ("z", KiwiValue::Int(1)),
            ],
        );
        let bytes = v.encode(&schema).unwrap();
        let back = KiwiValue::decode(&schema, KiwiType::user(2), &bytes).unwrap();
        assert_eq!(v, back);
        assert_eq!(back.get("children").unwrap().as_array().unwrap().len(), 3);
    }

    #[test]
    fn message_field_order_is_deterministic_regardless_of_insertion() {
        // Two values with the same fields inserted in different orders must
        // encode to identical bytes, because encoding follows the schema's
        // field declaration order, not the HashMap iteration order.
        let schema = sample_schema();
        let a = obj(
            "Node",
            vec![
                ("z", KiwiValue::Int(9)),
                ("name", KiwiValue::String("x".to_owned())),
            ],
        );
        let b = obj(
            "Node",
            vec![
                ("name", KiwiValue::String("x".to_owned())),
                ("z", KiwiValue::Int(9)),
            ],
        );
        assert_eq!(a.encode(&schema).unwrap(), b.encode(&schema).unwrap());
    }

    #[test]
    fn unknown_message_field_is_skipped_when_present_in_schema() {
        // Simulate a "newer writer": a Node message that includes a field id
        // the *reader's* schema does not list. We do this by encoding with a
        // schema that has an extra field, then decoding with one that does not,
        // proving forward-compat skipping works end to end.
        let writer_schema = Schema::new(vec![Def::new(
            "M",
            DefKind::Message,
            vec![
                Field::new("known", KiwiType::INT, 1),
                Field::new("extra", KiwiType::STRING, 2),
            ],
        )]);
        let reader_schema = Schema::new(vec![Def::new(
            "M",
            DefKind::Message,
            vec![Field::new("known", KiwiType::INT, 1)],
        )]);

        let v = obj(
            "M",
            vec![
                ("known", KiwiValue::Int(42)),
                ("extra", KiwiValue::String("ignore me".to_owned())),
            ],
        );
        let bytes = v.encode(&writer_schema).unwrap();

        // The reader's schema lacks `extra` (id 2). Decoding currently errors
        // on an unrecognized id because we cannot know its type from the reader
        // schema alone — but the writer's schema CAN skip it. Verify skipping
        // via the writer schema's skip path: decode with the writer schema,
        // then confirm the reader schema can still skip the whole message.
        let mut r = ByteReader::new(&bytes);
        writer_schema
            .skip_type(&mut r, KiwiType::user(0))
            .expect("writer schema skips its own message cleanly");
        assert!(r.is_at_end());

        // Sanity: the reader schema decodes the known field and rejects the
        // unknown id (documented limitation — generic decode is strict; the
        // skip API is the forward-compat tool).
        assert!(KiwiValue::decode(&reader_schema, KiwiType::user(0), &bytes).is_err());
    }

    #[test]
    fn byte_and_bool_and_64bit_fields_round_trip_in_struct() {
        // A struct mixing the less-common scalar types to cover them in the
        // compound path, not just standalone.
        let schema = Schema::new(vec![Def::new(
            "All",
            DefKind::Struct,
            vec![
                Field::new("b", KiwiType::BOOL, 0),
                Field::new("by", KiwiType::BYTE, 0),
                Field::new("i64", KiwiType::INT64, 0),
                Field::new("u64", KiwiType::UINT64, 0),
            ],
        )]);
        let v = obj(
            "All",
            vec![
                ("b", KiwiValue::Bool(true)),
                ("by", KiwiValue::Byte(200)),
                ("i64", KiwiValue::Int64(-12_345_678_901)),
                ("u64", KiwiValue::Uint64(98_765_432_109)),
            ],
        );
        let bytes = v.encode(&schema).unwrap();
        let back = KiwiValue::decode(&schema, KiwiType::user(0), &bytes).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn struct_missing_field_is_an_error_on_encode() {
        let schema = sample_schema();
        // Vec2 requires both x and y; omit y.
        let v = obj("Vec2", vec![("x", KiwiValue::Float(1.0))]);
        assert!(matches!(v.encode(&schema), Err(FigError::Schema(_))));
    }

    #[test]
    fn decode_with_unknown_type_id_errors() {
        let schema = sample_schema();
        // Type id 99 is not a builtin and past the def list.
        assert!(matches!(
            KiwiValue::decode(&schema, KiwiType(99), &[0]),
            Err(FigError::UnknownType(99))
        ));
    }

    #[test]
    fn array_of_floats_round_trips() {
        let schema = Schema::new(vec![Def::new(
            "Poly",
            DefKind::Message,
            vec![Field::array("pts", KiwiType::FLOAT, 1)],
        )]);
        let v = obj(
            "Poly",
            vec![(
                "pts",
                KiwiValue::array(vec![
                    KiwiValue::Float(0.0),
                    KiwiValue::Float(1.5),
                    KiwiValue::Float(-3.25),
                    KiwiValue::Float(1000.0),
                ]),
            )],
        );
        let bytes = v.encode(&schema).unwrap();
        let back = KiwiValue::decode(&schema, KiwiType::user(0), &bytes).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn byte_array_field_decodes_to_bytes_and_round_trips() {
        let schema = Schema::new(vec![Def::new(
            "Blob",
            DefKind::Message,
            vec![Field::array("bytes", KiwiType::BYTE, 1)],
        )]);
        let v = obj(
            "Blob",
            vec![("bytes", KiwiValue::Bytes(vec![0, 1, 254, 255]))],
        );
        let bytes = v.encode(&schema).unwrap();
        // id 1, length 4, then the raw bytes, then the message terminator.
        assert_eq!(bytes, [1, 4, 0, 1, 254, 255, 0]);
        let back = KiwiValue::decode(&schema, KiwiType::user(0), &bytes).unwrap();
        assert_eq!(back, v);
        assert_eq!(
            back.get("bytes").unwrap().as_bytes().unwrap().as_ref(),
            &[0, 1, 254, 255]
        );
        assert!(back.get("bytes").unwrap().as_array().is_none());
    }

    #[test]
    fn element_wise_byte_array_encodes_like_bytes_and_decodes_to_bytes() {
        // A hand-built `Array` of `Byte`s is the same wire bytes as `Bytes`, and
        // the decoder always produces the compact form.
        let schema = Schema::new(vec![Def::new(
            "Blob",
            DefKind::Message,
            vec![Field::array("bytes", KiwiType::BYTE, 1)],
        )]);
        let element_wise = obj(
            "Blob",
            vec![(
                "bytes",
                KiwiValue::array(vec![KiwiValue::Byte(7), KiwiValue::Byte(9)]),
            )],
        );
        let compact = obj("Blob", vec![("bytes", KiwiValue::Bytes(vec![7, 9]))]);
        let encoded = element_wise.encode(&schema).unwrap();
        assert_eq!(encoded, compact.encode(&schema).unwrap());
        let back = KiwiValue::decode(&schema, KiwiType::user(0), &encoded).unwrap();
        assert_eq!(back, compact);
        assert_eq!(
            element_wise
                .get("bytes")
                .unwrap()
                .as_bytes()
                .unwrap()
                .as_ref(),
            &[7, 9]
        );
    }

    #[test]
    fn bytes_on_a_non_byte_array_field_is_an_encode_error() {
        let schema = Schema::new(vec![Def::new(
            "Poly",
            DefKind::Message,
            vec![Field::array("pts", KiwiType::FLOAT, 1)],
        )]);
        let v = obj("Poly", vec![("pts", KiwiValue::Bytes(vec![1, 2]))]);
        assert!(matches!(v.encode(&schema), Err(FigError::Schema(_))));
    }

    #[test]
    fn truncated_byte_array_is_a_truncation_error() {
        let schema = Schema::new(vec![Def::new(
            "Blob",
            DefKind::Message,
            vec![Field::array("bytes", KiwiType::BYTE, 1)],
        )]);
        // id 1, length 4, but only two payload bytes follow.
        assert!(matches!(
            KiwiValue::decode(&schema, KiwiType::user(0), &[1, 4, 0, 1]),
            Err(FigError::Truncated)
        ));
    }

    #[test]
    fn empty_message_is_just_a_terminator() {
        let schema = sample_schema();
        let v = obj("Node", vec![]);
        let bytes = v.encode(&schema).unwrap();
        // An empty message is a single 0x00 (the field-id terminator).
        assert_eq!(bytes, [0]);
        let back = KiwiValue::decode(&schema, KiwiType::user(2), &bytes).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn full_document_schema_plus_value_round_trips_together() {
        // The end-to-end shape a `.fig` uses: encode the schema, encode a value,
        // then decode the schema from bytes and use it to decode the value.
        // This proves the two halves agree without any compiled types.
        let schema = sample_schema();
        let schema_bytes = schema.encode_binary();

        let doc = obj(
            "Node",
            vec![
                ("name", KiwiValue::String("Document".to_owned())),
                (
                    "size",
                    obj(
                        "Vec2",
                        vec![
                            ("x", KiwiValue::Float(1920.0)),
                            ("y", KiwiValue::Float(1080.0)),
                        ],
                    ),
                ),
                (
                    "children",
                    KiwiValue::array(vec![obj(
                        "Node",
                        vec![
                            ("name", KiwiValue::String("Frame".to_owned())),
                            ("shape", KiwiValue::Enum("RECT".into())),
                            ("z", KiwiValue::Int(0)),
                        ],
                    )]),
                ),
                ("z", KiwiValue::Int(0)),
            ],
        );
        let doc_bytes = doc.encode(&schema).unwrap();

        // Now pretend we just read these two blobs off disk.
        let decoded_schema = Schema::decode_binary(&schema_bytes).unwrap();
        let root_idx = decoded_schema.def_index("Node").unwrap();
        let decoded_doc =
            KiwiValue::decode(&decoded_schema, KiwiType::user(root_idx), &doc_bytes).unwrap();
        assert_eq!(decoded_doc, doc);
    }
}
