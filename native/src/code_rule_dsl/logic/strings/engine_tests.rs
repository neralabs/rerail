//! Tests of this module that read what the engine registers as it loads (the languages, their `.rr` libraries,
//! the ECMA crate's functions): the engine's facade of the module mounts them.

/// `@to_precision` is JavaScript's toPrecision of the numeral's value; no value past its range or for a non-number.
#[test]
fn to_precision_is_javascripts() {
    use rusqlite::types::Value;
    let f = |s: &str, d: i64| super::functions::eval("to_precision", &[s], &[d]);
    assert_eq!(f("9007199254740993", 16), Value::Text("9007199254740992".to_owned()));
    assert_eq!(f("1.50", 3), Value::Text("1.50".to_owned()));
    assert_eq!(f("12300", 3), Value::Text("1.23e+4".to_owned()));
    assert_eq!(f("1e3", 1), Value::Text("1e+3".to_owned()));
    assert_eq!(f("2e999", 1), Value::Text("Infinity".to_owned()));
    assert_eq!(f("1.5", 0), Value::Null);
    assert_eq!(f("x", 2), Value::Null);
}
