//! Natural collation: the natural-compare npm package (1.4.0, Lauri Rooden), which ESLint's
//! sort-keys uses for `natural: true`, ported step for step over UTF-16 code units.

use std::cmp::Ordering;

/// `naturalCompare(a, b)`: characters by a fixed remapping of ASCII (digits, then capitals, then
/// small letters, punctuation between), and a run of digits that starts with 1-9 as its number.
pub(crate) fn natural_compare(a: &[u16], b: &[u16]) -> Ordering {
    if a == b {
        return Ordering::Equal;
    }
    let (mut position_a, mut position_b) = (0usize, 0usize);
    // The package shares one scan index between both strings.
    let mut index = 0usize;
    let mut code_b = 1.0f64;
    while code_b != 0.0 {
        let mut code_a = code(a, position_a);
        position_a += 1;
        code_b = code(b, position_b);
        position_b += 1;
        if code_a < 76.0 && code_b < 76.0 && code_a > 66.0 && code_b > 66.0 {
            code_a = digits(a, position_a, &mut index);
            position_a = index;
            code_b = digits(b, position_b, &mut index);
            position_b = index;
        }
        if code_a != code_b {
            return if code_a < code_b {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
    }
    Ordering::Equal
}

/// `getCode(str, pos)`: past the end is 0; ASCII from 45 to 127 is remapped, the rest kept.
fn code(text: &[u16], position: usize) -> f64 {
    let code = f64::from(text.get(position).copied().unwrap_or(0));
    if !(45.0..=127.0).contains(&code) {
        code
    } else if code < 46.0 {
        65.0
    } else if code < 48.0 {
        code - 1.0
    } else if code < 58.0 {
        code + 18.0
    } else if code < 65.0 {
        code - 11.0
    } else if code < 91.0 {
        code + 11.0
    } else if code < 97.0 {
        code - 37.0
    } else if code < 123.0 {
        code + 5.0
    } else {
        code - 63.0
    }
}

/// `getCode(str, pos, code)`: the number the digits from `position - 1` spell, leaving `index`
/// after them.
fn digits(text: &[u16], position: usize, index: &mut usize) -> f64 {
    *index = position;
    loop {
        let code = code(text, *index);
        if code < 76.0 && code > 65.0 {
            *index += 1;
        } else {
            break;
        }
    }
    let run = text
        .get(position - 1..*index)
        .map(String::from_utf16_lossy)
        .unwrap_or_default();
    run.parse::<f64>().unwrap_or(f64::NAN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value as Json;

    use crate::code_rule_dsl::oracle;

    const ORACLE: &str = oracle!("ecma/value/natural.expected.json");

    /// Every pair in the natural-compare oracle (`rerail-fixtures/ecma/value/generate_natural.mjs`), as is and
    /// with both sides lower-cased as sort-keys does without `caseSensitive`.
    #[test]
    fn natural_order_is_natural_compare() {
        let oracle: Json = serde_json::from_str(ORACLE).expect("oracle JSON");
        let strings = oracle["strings"]
            .as_array()
            .expect("strings")
            .iter()
            .map(|units| {
                units
                    .as_array()
                    .expect("units")
                    .iter()
                    .map(|unit| unit.as_u64().expect("unit") as u16)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let lower = |units: &[u16]| {
            String::from_utf16(units)
                .map(|text| text.to_lowercase().encode_utf16().collect::<Vec<_>>())
                .unwrap_or_else(|_| units.to_vec())
        };
        let order = |value: &Json| match value.as_i64() {
            Some(-1) => Ordering::Less,
            Some(0) => Ordering::Equal,
            _ => Ordering::Greater,
        };
        let mut pairs = 0;
        for (row, results) in oracle["compare"]
            .as_array()
            .expect("compare")
            .iter()
            .enumerate()
        {
            for (column, result) in results.as_array().expect("row").iter().enumerate() {
                let (a, b) = (&strings[row], &strings[column]);
                assert_eq!(
                    natural_compare(a, b),
                    order(&result[0]),
                    "{:?} {:?}",
                    String::from_utf16_lossy(a),
                    String::from_utf16_lossy(b)
                );
                assert_eq!(
                    natural_compare(&lower(a), &lower(b)),
                    order(&result[1]),
                    "lower {:?} {:?}",
                    String::from_utf16_lossy(a),
                    String::from_utf16_lossy(b)
                );
                pairs += 1;
            }
        }
        assert!(pairs > 1000, "{pairs} pairs");
    }
}
