//! Kiwi type ids and the [`KiwiType`] newtype.

// =============================================================================
// Type ids
// =============================================================================
//
// On the wire (in the binary schema) a field's type is a single zig-zag varint.
// Builtins are encoded as small *negative* numbers; user-defined types as the
// non-negative index of their definition in the schema. We keep the raw `i32`
// representation (matching the reference's `type_id`) because the binary schema
// round-trip has to reproduce these exact integers, and because it lets a field
// reference a definition purely by index without a name lookup on the hot path.

/// `bool` builtin type id.
pub const TYPE_BOOL: i32 = -1;
/// `byte` builtin type id.
pub const TYPE_BYTE: i32 = -2;
/// `int` (32-bit zig-zag) builtin type id.
pub const TYPE_INT: i32 = -3;
/// `uint` (32-bit varint) builtin type id.
pub const TYPE_UINT: i32 = -4;
/// `float` (compact 32-bit) builtin type id.
pub const TYPE_FLOAT: i32 = -5;
/// `string` (UTF-8, null-terminated) builtin type id.
pub const TYPE_STRING: i32 = -6;
/// `int64` (64-bit zig-zag) builtin type id.
pub const TYPE_INT64: i32 = -7;
/// `uint64` (64-bit varint) builtin type id.
pub const TYPE_UINT64: i32 = -8;

/// The most negative builtin type id; anything below this is invalid.
pub(crate) const TYPE_MIN: i32 = TYPE_UINT64;

/// A field's declared type, kept as the raw wire `i32`.
///
/// This is a thin newtype over the type-id integer rather than an enum so that
/// the binary-schema codec can reproduce the exact bytes (a `KiwiType` is
/// `Copy` and `==`-comparable, which is all the codec needs). Use the
/// constructors and predicates to avoid sprinkling magic numbers around.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KiwiType(pub i32);

impl KiwiType {
    pub const BOOL: Self = Self(TYPE_BOOL);
    pub const BYTE: Self = Self(TYPE_BYTE);
    pub const INT: Self = Self(TYPE_INT);
    pub const UINT: Self = Self(TYPE_UINT);
    pub const FLOAT: Self = Self(TYPE_FLOAT);
    pub const STRING: Self = Self(TYPE_STRING);
    pub const INT64: Self = Self(TYPE_INT64);
    pub const UINT64: Self = Self(TYPE_UINT64);

    /// A reference to the user-defined definition at `index` in the schema.
    pub fn user(index: usize) -> Self {
        Self(index as i32)
    }

    /// Whether this id names a user-defined type (a definition index), as
    /// opposed to a builtin. Builtins are negative; definition indices are `>= 0`.
    pub fn is_user(self) -> bool {
        self.0 >= 0
    }

    /// The definition index if this is a user-defined type.
    pub fn def_index(self) -> Option<usize> {
        if self.is_user() {
            Some(self.0 as usize)
        } else {
            None
        }
    }
}
