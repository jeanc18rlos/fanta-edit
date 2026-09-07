//! Shared compact numeric display formatting for the Design surfaces.
//!
//! Passive inspector readouts, the paint picker, and the typography style
//! picker all display the same host-provided values; routing them through one
//! formatter keeps a value like `12.75` identical across surfaces.

/// Values closer than this to an integer display without a fractional part.
const NEAR_INTEGER_TOLERANCE: f32 = 0.001;

/// Formats a value with up to two decimals, trimming trailing zeros.
pub(super) fn format_compact_number(value: f32) -> String {
    if (value - value.round()).abs() < NEAR_INTEGER_TOLERANCE {
        format!("{}", value.round() as i64)
    } else {
        format!("{value:.2}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::format_compact_number;

    #[test]
    fn compact_formatting_trims_trailing_zeros_at_two_decimals() {
        assert_eq!(format_compact_number(0.), "0");
        assert_eq!(format_compact_number(16.), "16");
        assert_eq!(format_compact_number(12.5), "12.5");
        assert_eq!(format_compact_number(12.75), "12.75");
        assert_eq!(format_compact_number(-12.75), "-12.75");
        assert_eq!(format_compact_number(12.999_9), "13");
    }
}
