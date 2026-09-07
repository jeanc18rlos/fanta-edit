//! Shared parsing for host-supplied hexadecimal UI colors.

pub(crate) fn parse_hex_rgba(value: &str) -> Option<u32> {
    let trimmed = value.trim();
    let value = trimmed.strip_prefix('#').unwrap_or(trimmed);
    // `from_str_radix` accepts a sign prefix, which is not valid CSS hex.
    if !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let parsed = u32::from_str_radix(value, 16).ok()?;
    match value.len() {
        3 => {
            let red = (parsed >> 8) & 0xf;
            let green = (parsed >> 4) & 0xf;
            let blue = parsed & 0xf;
            Some(((red * 0x11) << 24) | ((green * 0x11) << 16) | ((blue * 0x11) << 8) | 0xff)
        }
        4 => {
            let red = (parsed >> 12) & 0xf;
            let green = (parsed >> 8) & 0xf;
            let blue = (parsed >> 4) & 0xf;
            let alpha = parsed & 0xf;
            Some(
                ((red * 0x11) << 24)
                    | ((green * 0x11) << 16)
                    | ((blue * 0x11) << 8)
                    | (alpha * 0x11),
            )
        }
        6 => Some((parsed << 8) | 0xff),
        8 => Some(parsed),
        _ => None,
    }
}

/// Splits a packed `0xRRGGBBAA` value into `[red, green, blue, alpha]`.
pub(crate) fn rgba_channels(value: u32) -> [u8; 4] {
    [
        (value >> 24) as u8,
        (value >> 16) as u8,
        (value >> 8) as u8,
        value as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::{parse_hex_rgba, rgba_channels};

    #[test]
    fn parses_css_hex_lengths_and_optional_hashes() {
        assert_eq!(parse_hex_rgba("AbC"), Some(0xaabbccff));
        assert_eq!(parse_hex_rgba("#abcd"), Some(0xaabbccdd));
        assert_eq!(parse_hex_rgba("336699"), Some(0x336699ff));
        assert_eq!(parse_hex_rgba("#33669980"), Some(0x33669980));
    }

    #[test]
    fn rejects_invalid_or_unsupported_hex_values() {
        assert_eq!(parse_hex_rgba(""), None);
        assert_eq!(parse_hex_rgba("not-a-color"), None);
        assert_eq!(parse_hex_rgba("#12345"), None);
        assert_eq!(parse_hex_rgba("+0F0"), None);
        assert_eq!(parse_hex_rgba("#+f00"), None);
    }

    #[test]
    fn splits_packed_rgba_into_channels() {
        assert_eq!(rgba_channels(0x4c86f780), [0x4c, 0x86, 0xf7, 0x80]);
        assert_eq!(rgba_channels(0x00000000), [0, 0, 0, 0]);
    }
}
