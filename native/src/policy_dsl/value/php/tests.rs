use super::*;
use crate::policy_dsl::language::GrammarDialect;
use crate::policy_dsl::oracle;
use crate::policy_dsl::value::Lang;
use ast_grep_core::tree_sitter::LanguageExt;
use serde_json::Value as Json;

const ORACLE: &str = oracle!("php/value/php.expected.json");

/// The value of the one expression statement `source` holds; `Unknown` where the grammar does not
/// read one (a syntax error).
fn decode_source(source: &str) -> Value {
    let text = format!("{source};\n");
    let root = GrammarDialect::Php.ast_grep(&text);
    let statements: Vec<_> = root.root().children().filter(|c| c.is_named()).collect();
    let [statement] = statements.as_slice() else {
        return Value::Unknown;
    };
    if statement.kind() != "expression_statement" {
        return Value::Unknown;
    }
    let Some(expression) = statement.children().find(|child| child.is_named()) else {
        return Value::Unknown;
    };
    let mut spend = || true;
    let mut context = Context::new(&mut spend);
    decode(&expression, &mut context)
}

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).expect("hex"))
        .collect()
}

fn key_agrees(key: &Key, oracle: &Json) -> bool {
    match (key, oracle[0].as_str()) {
        (Key::Int(value), Some("int")) => oracle[1].as_str() == Some(value.to_string().as_str()),
        (Key::Str(value), Some("string")) => oracle[1].as_str().map(hex).as_ref() == Some(value),
        _ => false,
    }
}

/// Whether Rerail's value is the one PHP recorded (`generate_php.php`'s tagged encoding).
pub(crate) fn agrees(value: &Value, oracle: &Json) -> bool {
    match (value, oracle[0].as_str()) {
        (Value::None, Some("null")) => true,
        (Value::Bool(value), Some("bool")) => oracle[1].as_bool() == Some(*value),
        (Value::Int(value), Some("int")) => oracle[1].as_str() == Some(value.to_string().as_str()),
        (Value::Float(value), Some("float")) => {
            let bits = u64::from_str_radix(oracle[1].as_str().unwrap_or_default(), 16)
                .map(f64::from_bits)
                .expect("float bits");
            bits.to_bits() == value.to_bits() || (bits.is_nan() && value.is_nan())
        }
        (Value::PhpStr(value), Some("string")) => {
            oracle[1].as_str().map(hex).as_ref() == Some(value)
        }
        (Value::PhpArray(items), Some("array")) => {
            let entries = oracle[1].as_array().expect("entries");
            items.len() == entries.len()
                && items.iter().zip(entries).all(|((key, value), entry)| {
                    key_agrees(key, &entry[0]) && agrees(value, &entry[1])
                })
        }
        _ => false,
    }
}

/// Whether every fact a value Rerail knows only part of (an array with a key or an element it does
/// not know) holds of the value PHP recorded: its type, its length where known, and whether it is
/// empty where known.
fn partial_agrees(value: &Value, oracle: &Json) -> bool {
    if oracle[0].as_str() != Some("array") {
        return false;
    }
    let entries = oracle[1].as_array().expect("entries");
    type_name(value) == Some("array")
        && array_len(value).is_some_and(|len| len.is_none_or(|len| len == entries.len()))
        && truthy(value).is_none_or(|nonempty| nonempty != entries.is_empty())
}

/// Every case in the PHP oracle (`fixtures/php/value/generate_php.php`):
/// - where PHP raises an error, Rerail knows nothing;
/// - a value Rerail knows is the value PHP gives the constant expression, or, where PHP refuses
///   it as a constant expression, the value it evaluates to at run time; of a value it knows only
///   part of, each part it claims is;
/// - where PHP evaluates the constant expression, Rerail knows all of its value, and where Rerail
///   knows part of a value it knows all of it, unless the case says why not.
#[test]
fn php_constant_expressions_decode_as_php_evaluates_them() {
    let oracle: Json = serde_json::from_str(ORACLE).expect("oracle JSON");
    assert_eq!(oracle["php"].as_str().map(|v| &v[..3]), Some("8.5"));
    assert_eq!(oracle["intSize"].as_u64(), Some(8));
    let mut failures = Vec::new();
    let mut known = 0;
    for case in oracle["cases"].as_array().expect("cases") {
        let source = case["src"].as_str().expect("src");
        let value = decode_source(source);
        let constant = &case["const"];
        let refused = constant[0].as_str() == Some("error");
        let expected = if refused { &case["eval"] } else { constant };
        let exempt = case.get("unknown").is_some();
        if value.is_known() {
            known += 1;
            if expected[0].as_str() == Some("error") {
                failures.push(format!(
                    "{source:?}: Rerail reads {value:?}, PHP raises {expected}"
                ));
            } else if value.is_constant() {
                if !agrees(&value, expected) {
                    failures.push(format!(
                        "{source:?}: Rerail reads {value:?}, PHP gives {expected}"
                    ));
                }
            } else if !partial_agrees(&value, expected) {
                failures.push(format!(
                    "{source:?}: Rerail reads {value:?}, PHP gives {expected}"
                ));
            } else if !exempt {
                failures.push(format!(
                    "{source:?}: PHP gives {expected}, Rerail knows only {value:?}"
                ));
            }
        } else if !refused && !exempt {
            failures.push(format!(
                "{source:?}: PHP gives {constant}, Rerail knows nothing"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} disagreements:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(known > 400, "Rerail knows only {known} values");
}

/// `===`, `==` and `<=>` over every pair of the oracle's operands, as PHP computes them: `equals`
/// is `===`; `order` is `<=>`, where a pair each side of which PHP finds greater does not order.
#[test]
fn php_comparisons_agree_with_php() {
    let oracle: Json = serde_json::from_str(ORACLE).expect("oracle JSON");
    let operands: Vec<Value> = oracle["pairs"]["operands"]
        .as_array()
        .expect("operands")
        .iter()
        .map(|source| decode_source(source.as_str().expect("operand")))
        .collect();
    assert!(!operands.is_empty(), "the oracle has no operands");
    if let Some(operand) = operands.iter().find(|operand| !operand.is_constant()) {
        panic!("Rerail does not know the operand {operand:?}");
    }
    let rows = oracle["pairs"]["results"].as_array().expect("results");
    assert_eq!(rows.len(), operands.len(), "one row per operand");
    let mut failures = Vec::new();
    for (row, left) in rows.iter().zip(&operands) {
        let row = row.as_array().expect("row");
        assert_eq!(row.len(), operands.len(), "one result per operand");
        for (result, right) in row.iter().zip(&operands) {
            let identical = result[0].as_bool().expect("===");
            let loose = result[1].as_bool().expect("==");
            let spaceship = result[2].as_i64().expect("<=>");
            let order = left.order(right, Lang::Php).expect("known operands");
            let rerail_spaceship = match order {
                None => 1,
                Some(order) => order as i64,
            };
            if left.equals(right, Lang::Php) != Some(identical)
                || (order == Some(Ordering::Equal)) != loose
                || rerail_spaceship != spaceship
            {
                failures.push(format!(
                    "{left:?} vs {right:?}: PHP ===, ==, <=> give {result}, Rerail {:?}, {order:?}",
                    left.equals(right, Lang::Php)
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} disagreements:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
