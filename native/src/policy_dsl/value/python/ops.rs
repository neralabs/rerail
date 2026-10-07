//! Python's operators on known values, as CPython computes them. Anything that would raise
//! (division by zero, a type error, an `OverflowError`) or build an `int` past `MAX_INT_BITS` is
//! `Unknown`.

use crate::policy_dsl::value::{
    BigInt, Lang, MAX_ELEMENTS, MAX_INT_BITS, MAX_STRING_BYTES, Value, int_to_float,
};
use num_integer::Integer;
use num_traits::{One, Signed, ToPrimitive, Zero};
use std::cmp::Ordering;

/// An operand as a number: `bool` and `int` exactly, `float`, `complex`.
enum Num {
    Int(BigInt),
    Float(f64),
    Complex(f64, f64),
}

fn num(value: &Value) -> Option<Num> {
    match value {
        Value::Bool(value) => Some(Num::Int(BigInt::from(u8::from(*value)))),
        Value::Int(value) => Some(Num::Int(value.clone())),
        Value::Float(value) => Some(Num::Float(*value)),
        Value::Complex(real, imag) => Some(Num::Complex(*real, *imag)),
        _ => None,
    }
}

/// A real operand as a `float`; `None` for a complex one or an `int` too large for one.
fn as_float(num: &Num) -> Option<f64> {
    match num {
        Num::Int(value) => int_to_float(value),
        Num::Float(value) => Some(*value),
        Num::Complex(..) => None,
    }
}

fn as_complex(num: &Num) -> Option<(f64, f64)> {
    match num {
        Num::Complex(real, imag) => Some((*real, *imag)),
        real => as_float(real).map(|real| (real, 0.0)),
    }
}

/// `-x`, `+x`, `~x` (and `not x`, which never fails).
pub(crate) fn unary(operator: &str, operand: &Value) -> Value {
    if operator == "not" {
        return operand
            .truthy(Lang::Python)
            .map_or(Value::Unknown, |truthy| Value::Bool(!truthy));
    }
    match (operator, num(operand)) {
        ("-", Some(Num::Int(value))) => Value::int(-value),
        ("+", Some(Num::Int(value))) => Value::int(value),
        ("~", Some(Num::Int(value))) => Value::int(-value - 1),
        ("-", Some(Num::Float(value))) => Value::Float(-value),
        ("+", Some(Num::Float(value))) => Value::Float(value),
        ("-", Some(Num::Complex(real, imag))) => Value::Complex(-real, -imag),
        ("+", Some(Num::Complex(real, imag))) => Value::Complex(real, imag),
        _ => Value::Unknown,
    }
}

/// A binary operator's result.
pub(crate) fn binary(operator: &str, left: &Value, right: &Value) -> Value {
    if let (Some(left_num), Some(right_num)) = (num(left), num(right)) {
        // `bool` op `bool` stays `bool` for the bitwise operators.
        if let (Value::Bool(left), Value::Bool(right)) = (left, right) {
            match operator {
                "&" => return Value::Bool(*left & *right),
                "|" => return Value::Bool(*left | *right),
                "^" => return Value::Bool(*left ^ *right),
                _ => {}
            }
        }
        return numeric(operator, &left_num, &right_num);
    }
    sequence(operator, left, right)
}

fn numeric(operator: &str, left: &Num, right: &Num) -> Value {
    match (left, right) {
        (Num::Int(left), Num::Int(right)) => integer(operator, left, right),
        (Num::Complex(..), _) | (_, Num::Complex(..)) => {
            match (as_complex(left), as_complex(right)) {
                (Some(left), Some(right)) => complex(operator, left, right),
                _ => Value::Unknown,
            }
        }
        _ => match (as_float(left), as_float(right)) {
            (Some(left), Some(right)) => float(operator, left, right),
            _ => Value::Unknown,
        },
    }
}

fn integer(operator: &str, left: &BigInt, right: &BigInt) -> Value {
    match operator {
        "+" => Value::int(left + right),
        "-" => Value::int(left - right),
        "*" => {
            if left.bits() + right.bits() > MAX_INT_BITS + 1 {
                return Value::Unknown;
            }
            Value::int(left * right)
        }
        "//" if !right.is_zero() => Value::int(left.div_floor(right)),
        "%" if !right.is_zero() => Value::int(left.mod_floor(right)),
        // True division rounds once only while both operands are exact floats.
        "/" => {
            const EXACT: u64 = 53;
            if right.is_zero() || left.bits() > EXACT || right.bits() > EXACT {
                return Value::Unknown;
            }
            match (int_to_float(left), int_to_float(right)) {
                (Some(left), Some(right)) => Value::Float(left / right),
                _ => Value::Unknown,
            }
        }
        "**" => power(left, right),
        "<<" => {
            if right.is_negative() {
                return Value::Unknown;
            }
            if left.is_zero() {
                return Value::int(0);
            }
            match right.to_u64() {
                Some(shift) if left.bits() + shift <= MAX_INT_BITS => {
                    Value::int(left << usize::try_from(shift).unwrap_or(usize::MAX))
                }
                _ => Value::Unknown,
            }
        }
        ">>" => {
            if right.is_negative() {
                return Value::Unknown;
            }
            // Past the operand's width every shift gives 0 or -1.
            let shift = right
                .to_u64()
                .map_or(left.bits() + 1, |shift| shift.min(left.bits() + 1));
            Value::int(left >> usize::try_from(shift).unwrap_or(usize::MAX))
        }
        "&" => Value::int(left & right),
        "|" => Value::int(left | right),
        "^" => Value::int(left ^ right),
        _ => Value::Unknown,
    }
}

/// `int ** int`: exact for a non-negative exponent, a `float` for a negative one.
fn power(base: &BigInt, exponent: &BigInt) -> Value {
    if exponent.is_negative() {
        return match (int_to_float(base), int_to_float(exponent)) {
            (Some(base), Some(exponent)) => float("**", base, exponent),
            _ => Value::Unknown,
        };
    }
    if base.is_zero() {
        return Value::int(u8::from(exponent.is_zero()));
    }
    if base.is_one() {
        return Value::int(1);
    }
    if *base == BigInt::from(-1) {
        return Value::int(if exponent.is_even() { 1 } else { -1 });
    }
    // The result has at least `(bits - 1) * exponent + 1` bits.
    match exponent.to_u32() {
        Some(exponent) if (base.bits() - 1).saturating_mul(u64::from(exponent)) < MAX_INT_BITS => {
            Value::int(num_traits::pow::Pow::pow(base, exponent))
        }
        _ => Value::Unknown,
    }
}

/// CPython's `float_rem`, `float_floor_div` and `float_pow`.
fn float(operator: &str, left: f64, right: f64) -> Value {
    let checked = |value: f64| {
        // A finite computation that overflows raises OverflowError.
        if value.is_infinite() && left.is_finite() && right.is_finite() && operator == "**" {
            Value::Unknown
        } else {
            Value::Float(value)
        }
    };
    match operator {
        "+" => Value::Float(left + right),
        "-" => Value::Float(left - right),
        "*" => Value::Float(left * right),
        "/" => {
            if right == 0.0 {
                Value::Unknown
            } else {
                Value::Float(left / right)
            }
        }
        "%" => {
            if right == 0.0 {
                return Value::Unknown;
            }
            Value::Float(float_mod(left, right))
        }
        "//" => {
            if right == 0.0 {
                return Value::Unknown;
            }
            let modulo = left % right;
            let mut division = (left - modulo) / right;
            if modulo != 0.0 && ((right < 0.0) != (modulo < 0.0)) {
                division -= 1.0;
            }
            if division != 0.0 {
                let floor = division.floor();
                Value::Float(if division - floor > 0.5 {
                    floor + 1.0
                } else {
                    floor
                })
            } else {
                Value::Float(0.0f64.copysign(left / right))
            }
        }
        "**" => {
            if right == 0.0 {
                return Value::Float(1.0);
            }
            if left == 0.0 && right < 0.0 {
                return Value::Unknown;
            }
            // A negative base to a fractional power is complex.
            if left < 0.0 && left.is_finite() && right.is_finite() && right.fract() != 0.0 {
                return Value::Unknown;
            }
            checked(left.powf(right))
        }
        _ => Value::Unknown,
    }
}

fn float_mod(left: f64, right: f64) -> f64 {
    let modulo = left % right;
    if modulo != 0.0 {
        if (right < 0.0) != (modulo < 0.0) {
            modulo + right
        } else {
            modulo
        }
    } else {
        0.0f64.copysign(right)
    }
}

fn complex(operator: &str, (a, b): (f64, f64), (c, d): (f64, f64)) -> Value {
    match operator {
        "+" => Value::Complex(a + c, b + d),
        "-" => Value::Complex(a - c, b - d),
        "*" => Value::Complex(a * c - b * d, a * d + b * c),
        _ => Value::Unknown,
    }
}

/// `+` and `*` on `str`, `bytes` and tuples.
fn sequence(operator: &str, left: &Value, right: &Value) -> Value {
    match (operator, left, right) {
        ("+", Value::Str(left), Value::Str(right)) => {
            if left.len() + right.len() > MAX_STRING_BYTES {
                return Value::Unknown;
            }
            Value::Str(format!("{left}{right}"))
        }
        ("+", Value::Bytes(left), Value::Bytes(right)) => {
            if left.len() + right.len() > MAX_STRING_BYTES {
                return Value::Unknown;
            }
            Value::Bytes([left.as_slice(), right].concat())
        }
        ("+", Value::Tuple(left), Value::Tuple(right)) => {
            if left.len() + right.len() > MAX_ELEMENTS {
                return Value::Unknown;
            }
            Value::Tuple([left.as_slice(), right].concat())
        }
        ("*", left, right) => {
            let (sequence, times) = if matches!(left, Value::Int(_) | Value::Bool(_)) {
                (right, left)
            } else {
                (left, right)
            };
            // A count beyond a C `ssize_t` raises OverflowError, negative or not.
            let count = match times {
                Value::Int(count) => match count.to_i64() {
                    Some(count) => count,
                    None => return Value::Unknown,
                },
                Value::Bool(count) => i64::from(*count),
                _ => return Value::Unknown,
            };
            let count = usize::try_from(count.max(0)).unwrap_or(usize::MAX);
            let (size, limit) = match sequence {
                Value::Str(value) => (value.len(), MAX_STRING_BYTES),
                Value::Bytes(value) => (value.len(), MAX_STRING_BYTES),
                Value::Tuple(value) => (value.len(), MAX_ELEMENTS),
                _ => return Value::Unknown,
            };
            // The cap applies before anything is allocated (`"a" * 10**9`).
            if size.checked_mul(count).is_none_or(|total| total > limit) {
                return Value::Unknown;
            }
            match sequence {
                Value::Str(value) => Value::Str(value.repeat(count)),
                Value::Bytes(value) => Value::Bytes(value.repeat(count)),
                Value::Tuple(value) => Value::Tuple(
                    std::iter::repeat_n(value, count)
                        .flatten()
                        .cloned()
                        .collect(),
                ),
                _ => Value::Unknown,
            }
        }
        _ => Value::Unknown,
    }
}

/// One comparison of a chain: `None` when it is not known or would raise.
pub(crate) fn compare(operator: &str, left: &Value, right: &Value) -> Option<bool> {
    match operator {
        "==" => left.equals(right, Lang::Python),
        "!=" => left.equals(right, Lang::Python).map(|equal| !equal),
        "<" | "<=" | ">" | ">=" => {
            let Some(order) = left.order(right, Lang::Python)? else {
                // A NaN makes an ordering False; types that do not order raise TypeError.
                let real = |value: &Value| {
                    matches!(value, Value::Bool(_) | Value::Int(_) | Value::Float(_))
                };
                return (real(left) && real(right)).then_some(false);
            };
            Some(match operator {
                "<" => order == Ordering::Less,
                "<=" => order != Ordering::Greater,
                ">" => order == Ordering::Greater,
                _ => order != Ordering::Less,
            })
        }
        "in" => contains(right, left),
        "not in" => contains(right, left).map(|contained| !contained),
        "is" => identical(left, right),
        "is not" => identical(left, right).map(|same| !same),
        _ => None,
    }
}

fn contains(container: &Value, item: &Value) -> Option<bool> {
    match (container, item) {
        (Value::Str(container), Value::Str(item)) => Some(container.contains(item.as_str())),
        (Value::Bytes(container), Value::Bytes(item)) => Some(
            item.is_empty()
                || container
                    .windows(item.len())
                    .any(|window| window == item.as_slice()),
        ),
        (Value::Tuple(elements), item) => {
            let mut result = Some(false);
            for element in elements {
                match item.equals(element, Lang::Python) {
                    Some(true) => return Some(true),
                    Some(false) => {}
                    None => result = None,
                }
            }
            result
        }
        _ => None,
    }
}

/// `is` between singletons; other identities depend on the interpreter.
fn identical(left: &Value, right: &Value) -> Option<bool> {
    let singleton = |value: &Value| matches!(value, Value::None | Value::Ellipsis | Value::Bool(_));
    if singleton(left) && singleton(right) {
        return Some(left == right);
    }
    if singleton(left) != singleton(right) && left.is_known() && right.is_known() {
        return Some(false);
    }
    None
}
