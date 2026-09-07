//! Numeric value semantics shared by editable Design-panel fields.
//!
//! The panel remains controlled by its host: these helpers only interpret a
//! draft string relative to the host-provided value. They do not retain or
//! mutate document state.

use std::{error::Error, fmt};

/// A unit marker accepted after a numeric value.
///
/// Pixels and degrees are parse-only decoration. A percent suffix is also
/// decoration when using [`parse_decorated_number`], but
/// [`evaluate_numeric_expression`] gives it Figma-style relative semantics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NumericSuffix {
    Pixels,
    Percent,
    Degrees,
}

/// A parsed finite number and its optional display suffix.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DecoratedNumber {
    pub value: f64,
    pub suffix: Option<NumericSuffix>,
}

/// Validation failures produced while interpreting an editable numeric field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NumericValueError {
    Empty,
    InvalidNumber,
    NonFinite,
    DivisionByZero,
    InvalidClamp,
}

impl fmt::Display for NumericValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "numeric value is empty",
            Self::InvalidNumber => "numeric value is not a valid expression",
            Self::NonFinite => "numeric value must be finite",
            Self::DivisionByZero => "numeric value cannot be divided by zero",
            Self::InvalidClamp => "numeric clamp bounds are invalid",
        };
        formatter.write_str(message)
    }
}

impl Error for NumericValueError {}

/// Optional inclusive bounds for a numeric field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NumericClamp {
    minimum: Option<f64>,
    maximum: Option<f64>,
}

impl NumericClamp {
    /// Creates validated inclusive bounds.
    pub(crate) fn new(
        minimum: Option<f64>,
        maximum: Option<f64>,
    ) -> Result<Self, NumericValueError> {
        if minimum.is_some_and(|value| !value.is_finite())
            || maximum.is_some_and(|value| !value.is_finite())
            || matches!((minimum, maximum), (Some(minimum), Some(maximum)) if minimum > maximum)
        {
            return Err(NumericValueError::InvalidClamp);
        }

        Ok(Self { minimum, maximum })
    }

    /// Applies these bounds to a finite value.
    pub(crate) fn apply(self, value: f64) -> Result<f64, NumericValueError> {
        require_finite(value)?;

        let value = self.minimum.map_or(value, |minimum| value.max(minimum));
        Ok(self.maximum.map_or(value, |maximum| value.min(maximum)))
    }
}

/// Direction used by keyboard arrow stepping.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ArrowStep {
    Decrease,
    Increase,
}

/// Parses a signed finite float followed by an optional `px`, `%`, or `°`.
///
/// The suffix is returned as decoration and does not scale the parsed value.
pub(crate) fn parse_decorated_number(input: &str) -> Result<DecoratedNumber, NumericValueError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(NumericValueError::Empty);
    }

    let (number, suffix) = strip_suffix(input);
    let number = number.trim();
    if number.is_empty() {
        return Err(NumericValueError::InvalidNumber);
    }

    let value = number
        .parse::<f64>()
        .map_err(|_| NumericValueError::InvalidNumber)?;
    require_finite(value)?;

    Ok(DecoratedNumber { value, suffix })
}

/// Evaluates a field draft against its controlled base value.
///
/// Plain values replace the base. Leading `+`, `-`, `*`, and `/` operators
/// adjust the base. Full arithmetic supports parentheses, exponentiation,
/// unary signs, and the usual operator precedence. A plain percentage scales
/// the base, so `50%` yields half of it, while `+10%` and `-10%` adjust it by
/// ten percent. Figma treats a percent sign as decoration when its value is
/// multiplied or divided: `*50%` multiplies the base by 50, and
/// `200 * 50%` evaluates to `10_000`, not `100`. Percentage values inside a
/// larger arithmetic expression likewise retain their displayed numeric
/// value. Pixel and degree suffixes are display-only decoration.
pub(crate) fn evaluate_numeric_expression(
    input: &str,
    base: f64,
    clamp: Option<NumericClamp>,
) -> Result<f64, NumericValueError> {
    require_finite(base)?;

    let input = input.trim();
    if input.is_empty() {
        return Err(NumericValueError::Empty);
    }

    let (operator, operand_text) = match input.as_bytes()[0] {
        b'+' => (Some(b'+'), &input[1..]),
        b'-' => (Some(b'-'), &input[1..]),
        b'*' => (Some(b'*'), &input[1..]),
        b'/' => (Some(b'/'), &input[1..]),
        _ => (None, input),
    };

    let simple_operand = match parse_decorated_number(operand_text) {
        Ok(operand) => Some(operand),
        Err(NumericValueError::InvalidNumber) => None,
        Err(error) => return Err(error),
    };
    let result = if let Some(operand) = simple_operand {
        let is_percent = operand.suffix == Some(NumericSuffix::Percent);
        let relative_operand = if is_percent {
            finite_result(base * operand.value / 100.)?
        } else {
            operand.value
        };

        match operator {
            None if is_percent => relative_operand,
            None => operand.value,
            Some(b'+') => finite_result(base + relative_operand)?,
            Some(b'-') => finite_result(base - relative_operand)?,
            Some(b'*') => finite_result(base * operand.value)?,
            Some(b'/') if operand.value == 0. => {
                return Err(NumericValueError::DivisionByZero);
            }
            Some(b'/') => finite_result(base / operand.value)?,
            Some(_) => unreachable!("the operator is selected from an exhaustive byte match"),
        }
    } else {
        let operand = evaluate_arithmetic_expression(operand_text)?;
        match operator {
            None => operand,
            Some(b'+') => finite_result(base + operand)?,
            Some(b'-') => finite_result(base - operand)?,
            Some(b'*') => finite_result(base * operand)?,
            Some(b'/') if operand == 0. => return Err(NumericValueError::DivisionByZero),
            Some(b'/') => finite_result(base / operand)?,
            Some(_) => unreachable!("the operator is selected from an exhaustive byte match"),
        }
    };

    clamp.map_or(Ok(result), |bounds| bounds.apply(result))
}

/// Rounds a finite value to the nearest integer, returned as an `f64`.
pub(crate) fn round_to_integer(value: f64) -> Result<f64, NumericValueError> {
    require_finite(value)?;
    Ok(value.round())
}

/// Applies Figma-style arrow stepping: one unit normally, ten with Shift.
#[cfg(test)]
pub(crate) fn step_with_arrow(
    base: f64,
    direction: ArrowStep,
    shift: bool,
    clamp: Option<NumericClamp>,
) -> Result<f64, NumericValueError> {
    step_with_arrow_amount(base, direction, if shift { 10. } else { 1. }, clamp)
}

/// Applies one validated host-configured arrow-key nudge.
pub(crate) fn step_with_arrow_amount(
    base: f64,
    direction: ArrowStep,
    amount: f64,
    clamp: Option<NumericClamp>,
) -> Result<f64, NumericValueError> {
    require_finite(base)?;
    if !amount.is_finite() || amount <= 0. {
        return Err(NumericValueError::InvalidNumber);
    }
    let result = match direction {
        ArrowStep::Decrease => finite_result(base - amount)?,
        ArrowStep::Increase => finite_result(base + amount)?,
    };
    clamp.map_or(Ok(result), |bounds| bounds.apply(result))
}

fn strip_suffix(input: &str) -> (&str, Option<NumericSuffix>) {
    if let Some(number) = input.strip_suffix('%') {
        (number, Some(NumericSuffix::Percent))
    } else if let Some(number) = input.strip_suffix('°') {
        (number, Some(NumericSuffix::Degrees))
    } else if input.len() >= 2 && input[input.len() - 2..].eq_ignore_ascii_case("px") {
        (&input[..input.len() - 2], Some(NumericSuffix::Pixels))
    } else {
        (input, None)
    }
}

fn evaluate_arithmetic_expression(input: &str) -> Result<f64, NumericValueError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(NumericValueError::Empty);
    }
    let (expression, suffix) = strip_suffix(input);
    if suffix == Some(NumericSuffix::Percent) {
        // A terminal percent belongs to the final numeric token and is parsed
        // by the expression parser. Pixel and degree suffixes decorate the
        // complete result.
        return ExpressionParser::new(input).parse();
    }
    ExpressionParser::new(expression.trim()).parse()
}

struct ExpressionParser<'a> {
    input: &'a [u8],
    cursor: usize,
}

impl<'a> ExpressionParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            cursor: 0,
        }
    }

    fn parse(mut self) -> Result<f64, NumericValueError> {
        let value = self.parse_sum()?;
        self.skip_whitespace();
        if self.cursor != self.input.len() {
            return Err(NumericValueError::InvalidNumber);
        }
        finite_result(value)
    }

    fn parse_sum(&mut self) -> Result<f64, NumericValueError> {
        let mut value = self.parse_product()?;
        loop {
            self.skip_whitespace();
            match self.peek() {
                Some(b'+') => {
                    self.cursor += 1;
                    value = finite_result(value + self.parse_product()?)?;
                }
                Some(b'-') => {
                    self.cursor += 1;
                    value = finite_result(value - self.parse_product()?)?;
                }
                _ => return Ok(value),
            }
        }
    }

    fn parse_product(&mut self) -> Result<f64, NumericValueError> {
        let mut value = self.parse_power()?;
        loop {
            self.skip_whitespace();
            match self.peek() {
                Some(b'*') => {
                    self.cursor += 1;
                    value = finite_result(value * self.parse_power()?)?;
                }
                Some(b'/') => {
                    self.cursor += 1;
                    let divisor = self.parse_power()?;
                    if divisor == 0. {
                        return Err(NumericValueError::DivisionByZero);
                    }
                    value = finite_result(value / divisor)?;
                }
                _ => return Ok(value),
            }
        }
    }

    fn parse_power(&mut self) -> Result<f64, NumericValueError> {
        let value = self.parse_unary()?;
        self.skip_whitespace();
        if self.peek() == Some(b'^') {
            self.cursor += 1;
            finite_result(value.powf(self.parse_power()?))
        } else {
            Ok(value)
        }
    }

    fn parse_unary(&mut self) -> Result<f64, NumericValueError> {
        self.skip_whitespace();
        match self.peek() {
            Some(b'+') => {
                self.cursor += 1;
                self.parse_unary()
            }
            Some(b'-') => {
                self.cursor += 1;
                finite_result(-self.parse_unary()?)
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<f64, NumericValueError> {
        self.skip_whitespace();
        let value = if self.peek() == Some(b'(') {
            self.cursor += 1;
            let value = self.parse_sum()?;
            self.skip_whitespace();
            if self.peek() != Some(b')') {
                return Err(NumericValueError::InvalidNumber);
            }
            self.cursor += 1;
            value
        } else {
            self.parse_number()?
        };
        self.skip_whitespace();
        if self.peek() == Some(b'%') {
            self.cursor += 1;
        }
        Ok(value)
    }

    fn parse_number(&mut self) -> Result<f64, NumericValueError> {
        self.skip_whitespace();
        let start = self.cursor;
        let mut has_digit = false;
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            has_digit = true;
            self.cursor += 1;
        }
        if self.peek() == Some(b'.') {
            self.cursor += 1;
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                has_digit = true;
                self.cursor += 1;
            }
        }
        if !has_digit {
            return Err(NumericValueError::InvalidNumber);
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.cursor += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.cursor += 1;
            }
            let exponent_start = self.cursor;
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.cursor += 1;
            }
            if exponent_start == self.cursor {
                return Err(NumericValueError::InvalidNumber);
            }
        }
        let number = std::str::from_utf8(&self.input[start..self.cursor])
            .map_err(|_| NumericValueError::InvalidNumber)?;
        let value = number
            .parse::<f64>()
            .map_err(|_| NumericValueError::InvalidNumber)?;
        require_finite(value)?;
        Ok(value)
    }

    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.cursor += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.cursor).copied()
    }
}

fn require_finite(value: f64) -> Result<(), NumericValueError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(NumericValueError::NonFinite)
    }
}

fn finite_result(value: f64) -> Result<f64, NumericValueError> {
    require_finite(value)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evaluate(input: &str, base: f64) -> Result<f64, NumericValueError> {
        evaluate_numeric_expression(input, base, None)
    }

    #[test]
    fn parses_plain_signed_floats() {
        assert_eq!(
            parse_decorated_number("  -12.5  "),
            Ok(DecoratedNumber {
                value: -12.5,
                suffix: None,
            })
        );
        assert_eq!(
            parse_decorated_number("+.25"),
            Ok(DecoratedNumber {
                value: 0.25,
                suffix: None,
            })
        );
        assert_eq!(
            parse_decorated_number("1.2e3"),
            Ok(DecoratedNumber {
                value: 1200.,
                suffix: None,
            })
        );
    }

    #[test]
    fn parses_supported_suffixes_as_decoration() {
        assert_eq!(
            parse_decorated_number("16 px"),
            Ok(DecoratedNumber {
                value: 16.,
                suffix: Some(NumericSuffix::Pixels),
            })
        );
        assert_eq!(
            parse_decorated_number("-25%"),
            Ok(DecoratedNumber {
                value: -25.,
                suffix: Some(NumericSuffix::Percent),
            })
        );
        assert_eq!(
            parse_decorated_number("90 °"),
            Ok(DecoratedNumber {
                value: 90.,
                suffix: Some(NumericSuffix::Degrees),
            })
        );
        assert_eq!(
            parse_decorated_number("2PX"),
            Ok(DecoratedNumber {
                value: 2.,
                suffix: Some(NumericSuffix::Pixels),
            })
        );
    }

    #[test]
    fn plain_expression_replaces_the_controlled_base() {
        assert_eq!(evaluate("42", 100.), Ok(42.));
        assert_eq!(evaluate("42px", -100.), Ok(42.));
        assert_eq!(evaluate("90°", 12.), Ok(90.));
    }

    #[test]
    fn prefix_operators_adjust_the_controlled_base() {
        assert_eq!(evaluate("+5.5", 10.), Ok(15.5));
        assert_eq!(evaluate("-2.5", 10.), Ok(7.5));
        assert_eq!(evaluate("*3", 10.), Ok(30.));
        assert_eq!(evaluate("/4", 10.), Ok(2.5));
        assert_eq!(evaluate("*-2", 10.), Ok(-20.));
        assert_eq!(evaluate("--2", 10.), Ok(12.));
    }

    #[test]
    fn evaluates_parentheses_precedence_and_exponents() {
        assert_eq!(evaluate("2 + 3 * 4", 99.), Ok(14.));
        assert_eq!(evaluate("(2 + 3) * 4", 99.), Ok(20.));
        assert_eq!(evaluate("2^3^2", 99.), Ok(512.));
        assert_eq!(evaluate("1e2 / (5 * 2)", 99.), Ok(10.));
        assert_eq!(evaluate("+(2 * 3)", 10.), Ok(16.));
        assert_eq!(evaluate("*(1 + .5)", 10.), Ok(15.));
        assert_eq!(evaluate("(2 + 3)px", 99.), Ok(5.));
        assert_eq!(evaluate("(45 * 2)°", 99.), Ok(90.));
    }

    #[test]
    fn percentages_inside_arithmetic_retain_their_numeric_value() {
        assert_eq!(evaluate("200 * 50%", 10.), Ok(10_000.));
        assert_eq!(evaluate("(25% + 25%) * 80", 10.), Ok(4_000.));
    }

    #[test]
    fn top_level_percentages_scale_or_adjust_the_base() {
        assert_eq!(evaluate("50%", 80.), Ok(40.));
        assert_eq!(evaluate("+25%", 80.), Ok(100.));
        assert_eq!(evaluate("-25%", 80.), Ok(60.));
    }

    #[test]
    fn multiplied_and_divided_percentages_are_numeric_operands() {
        assert_eq!(evaluate("*25%", 80.), Ok(2_000.));
        assert_eq!(evaluate("/25%", 80.), Ok(3.2));
    }

    #[test]
    fn rejects_empty_garbage_and_unsupported_suffixes() {
        assert_eq!(evaluate("", 10.), Err(NumericValueError::Empty));
        assert_eq!(evaluate("   ", 10.), Err(NumericValueError::Empty));
        assert_eq!(
            evaluate("not-a-number", 10.),
            Err(NumericValueError::InvalidNumber)
        );
        assert_eq!(evaluate("12pt", 10.), Err(NumericValueError::InvalidNumber));
        assert_eq!(evaluate("+px", 10.), Err(NumericValueError::InvalidNumber));
        assert_eq!(evaluate("1 2", 10.), Err(NumericValueError::InvalidNumber));
        assert_eq!(
            evaluate("(1 + 2", 10.),
            Err(NumericValueError::InvalidNumber)
        );
        assert_eq!(
            evaluate("1 + 2)", 10.),
            Err(NumericValueError::InvalidNumber)
        );
        assert_eq!(evaluate("2^^3", 10.), Err(NumericValueError::InvalidNumber));
    }

    #[test]
    fn rejects_non_finite_input_base_and_results() {
        assert_eq!(
            evaluate_numeric_expression("1", f64::NAN, None),
            Err(NumericValueError::NonFinite)
        );
        assert_eq!(evaluate("NaN", 10.), Err(NumericValueError::NonFinite));
        assert_eq!(evaluate("inf", 10.), Err(NumericValueError::NonFinite));
        assert_eq!(evaluate("*1e308", 1e308), Err(NumericValueError::NonFinite));
        assert_eq!(
            step_with_arrow(f64::INFINITY, ArrowStep::Increase, false, None),
            Err(NumericValueError::NonFinite)
        );
    }

    #[test]
    fn rejects_division_by_positive_or_negative_zero() {
        assert_eq!(evaluate("/0", 10.), Err(NumericValueError::DivisionByZero));
        assert_eq!(evaluate("/-0", 10.), Err(NumericValueError::DivisionByZero));
        assert_eq!(evaluate("/0%", 10.), Err(NumericValueError::DivisionByZero));
        assert_eq!(
            evaluate("10 / (3 - 3)", 10.),
            Err(NumericValueError::DivisionByZero)
        );
    }

    #[test]
    fn validates_and_applies_inclusive_clamps() {
        let clamp = NumericClamp::new(Some(0.), Some(100.)).unwrap();
        assert_eq!(
            evaluate_numeric_expression("+20", 90., Some(clamp)),
            Ok(100.)
        );
        assert_eq!(evaluate_numeric_expression("-20", 10., Some(clamp)), Ok(0.));
        assert_eq!(evaluate_numeric_expression("50", 10., Some(clamp)), Ok(50.));

        let minimum_only = NumericClamp::new(Some(5.), None).unwrap();
        assert_eq!(minimum_only.apply(-1.), Ok(5.));
        let maximum_only = NumericClamp::new(None, Some(5.)).unwrap();
        assert_eq!(maximum_only.apply(9.), Ok(5.));
    }

    #[test]
    fn rejects_invalid_clamps() {
        assert_eq!(
            NumericClamp::new(Some(2.), Some(1.)),
            Err(NumericValueError::InvalidClamp)
        );
        assert_eq!(
            NumericClamp::new(Some(f64::NAN), None),
            Err(NumericValueError::InvalidClamp)
        );
        assert_eq!(
            NumericClamp::new(None, Some(f64::INFINITY)),
            Err(NumericValueError::InvalidClamp)
        );
        let clamp = NumericClamp::new(None, None).unwrap();
        assert_eq!(
            clamp.apply(f64::NEG_INFINITY),
            Err(NumericValueError::NonFinite)
        );
    }

    #[test]
    fn rounds_only_finite_values_to_integers() {
        assert_eq!(round_to_integer(1.49), Ok(1.));
        assert_eq!(round_to_integer(1.5), Ok(2.));
        assert_eq!(round_to_integer(-1.5), Ok(-2.));
        assert_eq!(
            round_to_integer(f64::NAN),
            Err(NumericValueError::NonFinite)
        );
    }

    #[test]
    fn arrow_step_uses_one_normally_and_ten_with_shift() {
        assert_eq!(
            step_with_arrow(12., ArrowStep::Increase, false, None),
            Ok(13.)
        );
        assert_eq!(
            step_with_arrow(12., ArrowStep::Decrease, false, None),
            Ok(11.)
        );
        assert_eq!(
            step_with_arrow(12., ArrowStep::Increase, true, None),
            Ok(22.)
        );
        assert_eq!(
            step_with_arrow(12., ArrowStep::Decrease, true, None),
            Ok(2.)
        );

        let clamp = NumericClamp::new(Some(0.), Some(20.)).unwrap();
        assert_eq!(
            step_with_arrow(15., ArrowStep::Increase, true, Some(clamp)),
            Ok(20.)
        );
        assert_eq!(
            step_with_arrow(5., ArrowStep::Decrease, true, Some(clamp)),
            Ok(0.)
        );
    }
}
