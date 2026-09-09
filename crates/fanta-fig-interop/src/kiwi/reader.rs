//! [`ByteReader`] — the Kiwi primitive read cursor over a borrowed byte slice.

use crate::error::{FigError, FigResult};

// =============================================================================
// ByteReader
// =============================================================================

/// A cursor over a borrowed byte slice exposing the Kiwi primitive reads.
///
/// Every read is fallible and returns [`FigError::Truncated`] on EOF — there is
/// no panic path, because the input is untrusted (a real `.fig` from disk).
pub struct ByteReader<'a> {
    data: &'a [u8],
    index: usize,
}

impl<'a> ByteReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, index: 0 }
    }

    /// The current read offset. Useful for diagnostics and for asserting a
    /// decode consumed exactly the bytes it should.
    pub fn index(&self) -> usize {
        self.index
    }

    /// Whether the cursor has reached the end of the slice.
    pub fn is_at_end(&self) -> bool {
        self.index >= self.data.len()
    }

    pub fn read_byte(&mut self) -> FigResult<u8> {
        let b = *self.data.get(self.index).ok_or(FigError::Truncated)?;
        self.index += 1;
        Ok(b)
    }

    /// Borrow the next `len` bytes verbatim and advance past them. This is how
    /// a `byte[]` field is read: one bounds check for the whole run instead of
    /// one per element.
    pub fn read_bytes(&mut self, len: usize) -> FigResult<&'a [u8]> {
        let end = self.index.checked_add(len).ok_or(FigError::Truncated)?;
        let bytes = self.data.get(self.index..end).ok_or(FigError::Truncated)?;
        self.index = end;
        Ok(bytes)
    }

    pub fn read_bool(&mut self) -> FigResult<bool> {
        match self.read_byte()? {
            0 => Ok(false),
            1 => Ok(true),
            // A bool byte outside {0,1} is a corrupt stream. Treating it as
            // truncation keeps the error surface tiny and is what a malformed
            // file effectively is.
            _ => Err(FigError::Truncated),
        }
    }

    /// Read a variable-length unsigned 32-bit integer (LEB128).
    pub fn read_var_uint(&mut self) -> FigResult<u32> {
        let mut result: u32 = 0;
        let mut shift: u32 = 0;
        loop {
            let byte = self.read_byte()?;
            // Mask to 7 payload bits; OR into place at the running shift.
            result |= ((byte & 127) as u32) << shift;
            shift += 7;
            // Stop on a clear continuation bit, or once we have consumed the
            // 5 bytes that fully cover a u32 (the reference caps at shift 35).
            if byte & 128 == 0 || shift >= 35 {
                break;
            }
        }
        Ok(result)
    }

    /// Read a zig-zag-encoded signed 32-bit integer.
    pub fn read_var_int(&mut self) -> FigResult<i32> {
        let v = self.read_var_uint()?;
        // Undo zig-zag: even -> v/2, odd -> !(v/2).
        Ok(if v & 1 != 0 { !(v >> 1) } else { v >> 1 } as i32)
    }

    /// Read a variable-length unsigned 64-bit integer.
    ///
    /// The 64-bit varint differs subtly from the 32-bit one: after 8
    /// continuation bytes the 9th byte carries a full 8 payload bits (not 7),
    /// because 8*7 + 8 = 64. The reference encodes this by special-casing the
    /// final byte; we mirror it exactly.
    pub fn read_var_uint64(&mut self) -> FigResult<u64> {
        let mut result: u64 = 0;
        let mut shift: u32 = 0;
        loop {
            let byte = self.read_byte()?;
            if byte & 128 == 0 || shift >= 56 {
                result |= (byte as u64) << shift;
                break;
            }
            result |= ((byte & 127) as u64) << shift;
            shift += 7;
        }
        Ok(result)
    }

    /// Read a zig-zag-encoded signed 64-bit integer.
    pub fn read_var_int64(&mut self) -> FigResult<i64> {
        let v = self.read_var_uint64()?;
        Ok(if v & 1 != 0 { !(v >> 1) } else { v >> 1 } as i64)
    }

    /// Read a compact 32-bit float.
    pub fn read_var_float(&mut self) -> FigResult<f32> {
        let first = self.read_byte()?;
        // Single-byte zero (and subnormals encode here too).
        if first == 0 {
            return Ok(0.0);
        }
        let b1 = self.read_byte()?;
        let b2 = self.read_byte()?;
        let b3 = self.read_byte()?;
        let bits = first as u32 | (b1 as u32) << 8 | (b2 as u32) << 16 | (b3 as u32) << 24;
        // Reverse the encoder's exponent rotation: it did rotate_right(23) to
        // move the exponent into the low byte; we rotate_left(23) to restore.
        // (`(bits << 23) | (bits >> 9)` in the reference is exactly a 32-bit
        // left-rotate by 23.)
        let bits = bits.rotate_left(23);
        Ok(f32::from_bits(bits))
    }

    /// Read a UTF-8 string up to (and consuming) the `0x00` terminator.
    ///
    /// Invalid UTF-8 is replaced lossily rather than rejected — the reference
    /// does the same (`from_utf8_lossy`), and a single bad string should not
    /// abort the import of an otherwise-valid document.
    pub fn read_string(&mut self) -> FigResult<String> {
        let start = self.index;
        while self.index < self.data.len() {
            if self.data[self.index] == 0 {
                let s = String::from_utf8_lossy(&self.data[start..self.index]).into_owned();
                self.index += 1; // consume the terminator
                return Ok(s);
            }
            self.index += 1;
        }
        // Hit EOF without a terminator.
        Err(FigError::Truncated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- truncation handling ------------------------------------------------

    #[test]
    fn reads_past_end_return_truncated() {
        assert!(matches!(
            ByteReader::new(&[]).read_byte(),
            Err(FigError::Truncated)
        ));
        // A varint with the continuation bit set but no following byte.
        assert!(matches!(
            ByteReader::new(&[0x80]).read_var_uint(),
            Err(FigError::Truncated)
        ));
        // A string with no terminator.
        assert!(matches!(
            ByteReader::new(&[97, 98]).read_string(),
            Err(FigError::Truncated)
        ));
        // A float that announces 4 bytes but supplies one.
        assert!(matches!(
            ByteReader::new(&[133]).read_var_float(),
            Err(FigError::Truncated)
        ));
    }
}
