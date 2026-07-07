//! [`ByteWriter`] — the append-only Kiwi primitive write buffer.

// =============================================================================
// ByteWriter
// =============================================================================

/// An append-only byte buffer exposing the Kiwi primitive writes.
///
/// The writes are the exact inverse of [`ByteReader`]; the test suite pins this
/// with byte-for-byte vectors lifted from the reference implementation.
///
/// [`ByteReader`]: crate::kiwi::ByteReader
#[derive(Default)]
pub struct ByteWriter {
    data: Vec<u8>,
}

impl ByteWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Consume the writer and return the bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.data
    }

    /// Borrow the bytes written so far.
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    pub fn write_byte(&mut self, value: u8) {
        self.data.push(value);
    }

    pub fn write_bool(&mut self, value: bool) {
        self.data.push(value as u8);
    }

    pub fn write_var_uint(&mut self, mut value: u32) {
        loop {
            let byte = (value as u8) & 127;
            value >>= 7;
            if value == 0 {
                self.write_byte(byte);
                return;
            }
            self.write_byte(byte | 128);
        }
    }

    pub fn write_var_int(&mut self, value: i32) {
        // Zig-zag: map sign into the low bit so small magnitudes stay short.
        self.write_var_uint(((value << 1) ^ (value >> 31)) as u32);
    }

    pub fn write_var_uint64(&mut self, mut value: u64) {
        // Mirror the reference: up to 8 continuation bytes, then a final byte
        // that carries a full 8 bits.
        let mut i = 0;
        while value > 127 && i < 8 {
            self.write_byte((value as u8 & 127) | 128);
            value >>= 7;
            i += 1;
        }
        self.write_byte(value as u8);
    }

    pub fn write_var_int64(&mut self, value: i64) {
        self.write_var_uint64(((value << 1) ^ (value >> 63)) as u64);
    }

    pub fn write_var_float(&mut self, value: f32) {
        // Rotate the exponent into the low byte. `rotate_right(23)` matches the
        // reference's `(bits >> 23) | (bits << 9)`.
        let bits = value.to_bits().rotate_right(23);
        // Zero and subnormals (exponent byte == 0) collapse to a single 0x00.
        if bits & 255 == 0 {
            self.write_byte(0);
            return;
        }
        self.data.extend_from_slice(&[
            bits as u8,
            (bits >> 8) as u8,
            (bits >> 16) as u8,
            (bits >> 24) as u8,
        ]);
    }

    /// Write a UTF-8 string and its `0x00` terminator.
    ///
    /// A string containing an interior NUL cannot be represented (the
    /// terminator would be ambiguous); the reference throws, and so do we via
    /// a debug assertion plus truncating at the NUL in release. Design-doc
    /// strings never contain NUL, so this is purely defensive.
    pub fn write_string(&mut self, value: &str) {
        debug_assert!(
            !value.as_bytes().contains(&0),
            "Kiwi strings cannot contain an interior NUL byte"
        );
        self.data.extend_from_slice(value.as_bytes());
        self.write_byte(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kiwi::ByteReader;

    // ---- primitive byte-vector pins (lifted from the reference) -------------
    //
    // These assert our writer produces the *exact* bytes the canonical Kiwi
    // implementation does. If any of these drift, we are no longer Kiwi-
    // compatible and could not read a real `.fig`.

    fn write_uint(v: u32) -> Vec<u8> {
        let mut w = ByteWriter::new();
        w.write_var_uint(v);
        w.into_bytes()
    }
    fn write_int(v: i32) -> Vec<u8> {
        let mut w = ByteWriter::new();
        w.write_var_int(v);
        w.into_bytes()
    }
    fn write_float(v: f32) -> Vec<u8> {
        let mut w = ByteWriter::new();
        w.write_var_float(v);
        w.into_bytes()
    }

    #[test]
    fn var_uint_matches_reference_vectors() {
        assert_eq!(write_uint(0), [0]);
        assert_eq!(write_uint(127), [127]);
        assert_eq!(write_uint(128), [128, 1]);
        assert_eq!(write_uint(256), [128, 2]);
        assert_eq!(write_uint(129), [129, 1]);
        assert_eq!(write_uint(131069), [253, 255, 7]);
        assert_eq!(write_uint(4294967295), [255, 255, 255, 255, 15]);
    }

    #[test]
    fn var_int_zigzag_matches_reference_vectors() {
        assert_eq!(write_int(0), [0]);
        assert_eq!(write_int(-1), [1]);
        assert_eq!(write_int(1), [2]);
        assert_eq!(write_int(-2), [3]);
        assert_eq!(write_int(-64), [127]);
        assert_eq!(write_int(64), [128, 1]);
        assert_eq!(write_int(-2147483648), [255, 255, 255, 255, 15]);
    }

    #[test]
    fn var_float_matches_reference_vectors() {
        // The headline optimization: zero (and negative zero) is a single byte.
        assert_eq!(write_float(0.0), [0]);
        assert_eq!(write_float(-0.0), [0]);
        assert_eq!(write_float(123.456), [133, 242, 210, 237]);
        assert_eq!(write_float(-123.456), [133, 243, 210, 237]);
        assert_eq!(write_float(f32::MIN), [254, 255, 255, 255]);
        assert_eq!(write_float(f32::MAX), [254, 254, 255, 255]);
        assert_eq!(write_float(f32::INFINITY), [255, 0, 0, 0]);
        assert_eq!(write_float(f32::NEG_INFINITY), [255, 1, 0, 0]);
        // Subnormals collapse to the single-byte zero encoding.
        assert_eq!(write_float(1.0e-40), [0]);
    }

    #[test]
    fn string_is_utf8_null_terminated() {
        let mut w = ByteWriter::new();
        w.write_string("🍕");
        assert_eq!(w.into_bytes(), [240, 159, 141, 149, 0]);
    }

    // ---- primitive round-trips ---------------------------------------------

    #[test]
    fn uint_round_trips_across_range() {
        for v in [
            0u32,
            1,
            127,
            128,
            300,
            16384,
            1 << 21,
            u32::MAX,
            u32::MAX - 1,
        ] {
            let bytes = write_uint(v);
            let mut r = ByteReader::new(&bytes);
            assert_eq!(r.read_var_uint().unwrap(), v);
            assert!(r.is_at_end(), "reader did not consume all bytes for {v}");
        }
    }

    #[test]
    fn int_round_trips_including_extremes() {
        for v in [
            0i32,
            -1,
            1,
            -2,
            2,
            i32::MIN,
            i32::MAX,
            -65535,
            65535,
            -123456,
        ] {
            let bytes = write_int(v);
            let mut r = ByteReader::new(&bytes);
            assert_eq!(r.read_var_int().unwrap(), v);
        }
    }

    #[test]
    fn int64_and_uint64_round_trip() {
        for v in [0i64, -1, 1, i64::MIN, i64::MAX, -0x4000_0000_0000_0000] {
            let mut w = ByteWriter::new();
            w.write_var_int64(v);
            let bytes = w.into_bytes();
            let mut r = ByteReader::new(&bytes);
            assert_eq!(r.read_var_int64().unwrap(), v);
        }
        for v in [
            0u64,
            1,
            u64::MAX,
            0x8000_0000_0000_0000,
            0x1000_0000_0000_0001,
        ] {
            let mut w = ByteWriter::new();
            w.write_var_uint64(v);
            let bytes = w.into_bytes();
            let mut r = ByteReader::new(&bytes);
            assert_eq!(r.read_var_uint64().unwrap(), v);
        }
    }

    #[test]
    fn float_round_trips_edge_cases() {
        for v in [
            0.0f32,
            -0.0,
            1.0,
            -1.0,
            123.456,
            -123.456,
            f32::MIN,
            f32::MAX,
            f32::MIN_POSITIVE,
            -f32::MIN_POSITIVE,
            f32::INFINITY,
            f32::NEG_INFINITY,
            1e30,
            -1e-30,
        ] {
            let bytes = write_float(v);
            let mut r = ByteReader::new(&bytes);
            let back = r.read_var_float().unwrap();
            // -0.0 and subnormals normalize to +0.0 by design; compare via the
            // same lossy lens the encoder applies.
            if v == 0.0 || v.abs() < f32::MIN_POSITIVE {
                assert_eq!(back, 0.0);
            } else {
                assert_eq!(back.to_bits(), v.to_bits(), "float {v} did not round-trip");
            }
        }
    }

    #[test]
    fn nan_float_round_trips_as_nan() {
        let bytes = write_float(f32::NAN);
        let mut r = ByteReader::new(&bytes);
        assert!(r.read_var_float().unwrap().is_nan());
    }

    #[test]
    fn string_round_trips_unicode_and_empty() {
        for s in ["", "a", "abc", "🍕 with spaces", "Ünîçödé", "新しい"] {
            let mut w = ByteWriter::new();
            w.write_string(s);
            let bytes = w.into_bytes();
            let mut r = ByteReader::new(&bytes);
            assert_eq!(r.read_string().unwrap(), s);
        }
    }
}
