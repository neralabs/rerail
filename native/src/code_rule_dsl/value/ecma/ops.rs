//! ECMAScript's operators on known primitive values, as `getStaticValue` computes them. A RegExp
//! converts to its string (`/a/ + ''` is `'/a/'`). An operation that would throw (a BigInt mixed
//! with a Number, a BigInt divided by zero) or that needs an array's or an object's conversion
//! (`[1] + 1`) is `Unknown`.

use crate::code_rule_dsl::value::key::{
    number_to_string, string_to_bigint, string_to_number, to_int32, to_uint32,
};
use crate::code_rule_dsl::value::{BigInt, Lang, MAX_STRING_BYTES, Value};
use std::cmp::Ordering;

fn primitive(value: &Value) -> bool {
    matches!(
        value,
        Value::None
            | Value::Undefined
            | Value::Bool(_)
            | Value::Float(_)
            | Value::BigInt(_)
            | Value::Utf16(_)
    )
}

/// ToPrimitive: a primitive is itself, and a RegExp its string (its `valueOf` returns the object,
/// so either hint reaches `toString`). `None` for an array or an object, whose conversion is not
/// modelled.
pub(crate) fn to_primitive(value: &Value) -> Option<Value> {
    match value {
        Value::RegExp(units) => Some(Value::Utf16(units.clone())),
        value if primitive(value) => Some(value.clone()),
        _ => None,
    }
}

/// ToNumber of a primitive; `None` for a BigInt (a TypeError) or an object.
pub(crate) fn to_number(value: &Value) -> Option<f64> {
    match value {
        Value::Undefined => Some(f64::NAN),
        Value::None => Some(0.0),
        Value::Bool(value) => Some(f64::from(u8::from(*value))),
        Value::Float(value) => Some(*value),
        Value::Utf16(units) => string_to_number(units),
        _ => None,
    }
}

/// ToString of a primitive or a RegExp.
pub(crate) fn to_string(value: &Value) -> Option<Vec<u16>> {
    let text = match value {
        Value::Undefined => "undefined".to_owned(),
        Value::None => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Float(value) => number_to_string(*value),
        Value::BigInt(value) => value.to_string(),
        Value::Utf16(units) | Value::RegExp(units) => return Some(units.clone()),
        _ => return None,
    };
    Some(text.encode_utf16().collect())
}

/// `typeof`.
fn type_of(value: &Value) -> Option<&'static str> {
    Some(match value {
        Value::Undefined => "undefined",
        Value::None | Value::Array(_) | Value::RegExp(_) | Value::Container { .. } => "object",
        Value::Bool(_) => "boolean",
        Value::Float(_) => "number",
        Value::BigInt(_) => "bigint",
        Value::Utf16(_) => "string",
        _ => return None,
    })
}

fn utf16(text: &str) -> Value {
    Value::Utf16(text.encode_utf16().collect())
}

/// `-x`, `+x`, `!x`, `~x`, `typeof x`, `void x`.
pub(crate) fn unary(operator: &str, operand: &Value) -> Value {
    match operator {
        "void" => return Value::Undefined,
        "!" => {
            return operand
                .truthy(Lang::Js)
                .map_or(Value::Unknown, |truthy| Value::Bool(!truthy));
        }
        "typeof" => return type_of(operand).map_or(Value::Unknown, utf16),
        _ => {}
    }
    let Some(operand) = to_primitive(operand) else {
        return Value::Unknown;
    };
    if let Value::BigInt(value) = operand {
        return match operator {
            "-" => value.checked_neg().map_or(Value::Unknown, Value::BigInt),
            "~" => Value::BigInt(!value),
            // `+1n` throws.
            _ => Value::Unknown,
        };
    }
    let Some(number) = to_number(&operand) else {
        return Value::Unknown;
    };
    match operator {
        "-" => Value::Float(-number),
        "+" => Value::Float(number),
        "~" => Value::Float(f64::from(!to_int32(number))),
        _ => Value::Unknown,
    }
}

/// A binary operator's result (`&&`, `||` and `??` short-circuit in the caller).
pub(crate) fn binary(operator: &str, left: &Value, right: &Value) -> Value {
    if !left.is_known() || !right.is_known() {
        return Value::Unknown;
    }
    match operator {
        "===" => return strict_equals(left, right).map_or(Value::Unknown, Value::Bool),
        "!==" => {
            return strict_equals(left, right).map_or(Value::Unknown, |equal| Value::Bool(!equal));
        }
        "==" => return loose_equals(left, right).map_or(Value::Unknown, Value::Bool),
        "!=" => {
            return loose_equals(left, right).map_or(Value::Unknown, |equal| Value::Bool(!equal));
        }
        "<" | ">" | "<=" | ">=" => return relational(operator, left, right),
        _ => {}
    }
    let (Some(left), Some(right)) = (to_primitive(left), to_primitive(right)) else {
        return Value::Unknown;
    };
    let (left, right) = (&left, &right);
    if operator == "+" && (matches!(left, Value::Utf16(_)) || matches!(right, Value::Utf16(_))) {
        let (Some(left), Some(right)) = (to_string(left), to_string(right)) else {
            return Value::Unknown;
        };
        if (left.len() + right.len()) * 2 > MAX_STRING_BYTES {
            return Value::Unknown;
        }
        return Value::Utf16([left, right].concat());
    }
    match (left, right) {
        (Value::BigInt(left), Value::BigInt(right)) => bigint(operator, *left, *right),
        // Mixing a BigInt with a Number throws.
        (Value::BigInt(_), _) | (_, Value::BigInt(_)) => Value::Unknown,
        _ => match (to_number(left), to_number(right)) {
            (Some(left), Some(right)) => number(operator, left, right),
            _ => Value::Unknown,
        },
    }
}

fn number(operator: &str, left: f64, right: f64) -> Value {
    let shift = to_uint32(right) & 31;
    Value::Float(match operator {
        "+" => left + right,
        "-" => left - right,
        "*" => left * right,
        "/" => left / right,
        "%" => left % right,
        "**" => {
            // Unlike C's pow, 1 ** NaN and (-1) ** Infinity are NaN.
            if right.is_nan() || (left.abs() == 1.0 && right.is_infinite()) {
                f64::NAN
            } else {
                left.powf(right)
            }
        }
        "<<" => f64::from(to_int32(left).wrapping_shl(shift)),
        ">>" => f64::from(to_int32(left) >> shift),
        ">>>" => f64::from(to_uint32(left) >> shift),
        "&" => f64::from(to_int32(left) & to_int32(right)),
        "|" => f64::from(to_int32(left) | to_int32(right)),
        "^" => f64::from(to_int32(left) ^ to_int32(right)),
        _ => return Value::Unknown,
    })
}

fn bigint(operator: &str, left: i128, right: i128) -> Value {
    let result = match operator {
        "+" => left.checked_add(right),
        "-" => left.checked_sub(right),
        "*" => left.checked_mul(right),
        // Truncating, and a RangeError on zero.
        "/" => left.checked_div(right),
        "%" => left.checked_rem(right),
        "**" => u32::try_from(right)
            .ok()
            .and_then(|right| left.checked_pow(right)),
        "&" => Some(left & right),
        "|" => Some(left | right),
        "^" => Some(left ^ right),
        "<<" => shift_left(left, right),
        ">>" => right
            .checked_neg()
            .and_then(|right| shift_left(left, right)),
        _ => None,
    };
    result.map_or(Value::Unknown, Value::BigInt)
}

/// `left << right` for BigInts; a negative shift shifts right, flooring.
fn shift_left(left: i128, right: i128) -> Option<i128> {
    if right < 0 {
        let shift = u32::try_from(right.unsigned_abs())
            .unwrap_or(u32::MAX)
            .min(127);
        return Some(left >> shift);
    }
    let shift = u32::try_from(right).ok().filter(|shift| *shift < 127)?;
    left.checked_shl(shift)
        .filter(|shifted| shifted >> shift == left)
}

/// IsStrictlyEqual: SameValue except that NaN is not NaN and `0` is `-0`.
fn strict_equals(left: &Value, right: &Value) -> Option<bool> {
    match (left, right) {
        (Value::Float(left), Value::Float(right)) => Some(left == right),
        _ => left.equals(right, Lang::Js),
    }
}

/// IsLooselyEqual between primitives; an object against a primitive other than `null` and
/// `undefined` converts: a RegExp to its string, an array or an object in a way not modelled.
fn loose_equals(left: &Value, right: &Value) -> Option<bool> {
    let nullish = |value: &Value| matches!(value, Value::None | Value::Undefined);
    if !primitive(left) || !primitive(right) {
        return if nullish(left) || nullish(right) {
            // An object is loosely equal to neither.
            Some(false)
        } else if primitive(left) || primitive(right) {
            loose_equals(&to_primitive(left)?, &to_primitive(right)?)
        } else {
            // Two literals are two objects.
            Some(false)
        };
    }
    if nullish(left) || nullish(right) {
        return Some(nullish(left) && nullish(right));
    }
    Some(match (left, right) {
        (Value::Utf16(left), Value::Utf16(right)) => left == right,
        (Value::BigInt(left), Value::BigInt(right)) => left == right,
        (Value::BigInt(big), Value::Utf16(text)) | (Value::Utf16(text), Value::BigInt(big)) => {
            string_to_bigint(text).is_some_and(|value| value == BigInt::from(*big))
        }
        (Value::BigInt(big), other) | (other, Value::BigInt(big)) => {
            crate::code_rule_dsl::value::int_float_order(&BigInt::from(*big), to_number(other)?)
                == Some(Ordering::Equal)
        }
        _ => to_number(left)? == to_number(right)?,
    })
}

/// `<`, `>`, `<=`, `>=`.
fn relational(operator: &str, left: &Value, right: &Value) -> Value {
    relation(operator, left, right).map_or(Value::Unknown, Value::Bool)
}

/// A relational operator's result over known primitives, as ECMAScript evaluates it: `<` and
/// `>` from IsLessThan, `<=` and `>=` as its negation with the operands swapped, and an undefined
/// comparison (NaN, a string that is no BigInt) false either way. `None` for anything that is
/// not a primitive (an object, an array or a regex converts through ToPrimitive, not modelled).
pub(crate) fn relation(operator: &str, left: &Value, right: &Value) -> Option<bool> {
    Some(match operator {
        "<" => is_less_than(left, right)? == Some(true),
        ">" => is_less_than(right, left)? == Some(true),
        "<=" => is_less_than(right, left)? == Some(false),
        ">=" => is_less_than(left, right)? == Some(false),
        _ => return None,
    })
}

/// IsLessThan after ToPrimitive (a RegExp is its string): strings by code unit; a BigInt against
/// a string through StringToBigInt; otherwise ToNumeric, with a BigInt against a Number compared
/// exactly. `Some(None)` where the result is undefined; `None` where it is not known.
fn is_less_than(left: &Value, right: &Value) -> Option<Option<bool>> {
    let (left, right) = (&to_primitive(left)?, &to_primitive(right)?);
    let order = match (left, right) {
        (Value::Utf16(left), Value::Utf16(right)) => Some(left.cmp(right)),
        (Value::BigInt(big), Value::Utf16(text)) => {
            string_to_bigint(text).map(|value| BigInt::from(*big).cmp(&value))
        }
        (Value::Utf16(text), Value::BigInt(big)) => {
            string_to_bigint(text).map(|value| value.cmp(&BigInt::from(*big)))
        }
        (Value::BigInt(left), Value::BigInt(right)) => Some(left.cmp(right)),
        (Value::BigInt(big), other) => {
            crate::code_rule_dsl::value::int_float_order(&BigInt::from(*big), to_number(other)?)
        }
        (other, Value::BigInt(big)) => {
            crate::code_rule_dsl::value::int_float_order(&BigInt::from(*big), to_number(other)?)
                .map(Ordering::reverse)
        }
        _ => to_number(left)?.partial_cmp(&to_number(right)?),
    };
    Some(order.map(Ordering::is_lt))
}
