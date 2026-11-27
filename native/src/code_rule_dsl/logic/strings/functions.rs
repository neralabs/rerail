//! The `core.rerail` functors that compute text from text (`@head`, `@join`, `@replace_all`, …), and Soufflé's
//! built-in functions beyond SQL's (`to_number`, `max`, `min`, `%`), each a native SQL function of the same name
//! (`lower`, `max`, `min` and `mod` in place of SQLite's own). A rule's call `@f(a, b)` is lowered to `f(a, b)` in
//! SQL, its symbol arguments decoded to text and a symbol result encoded back (`plan`'s `text_of` / `symbol_of`).
//! Each function's meaning is `core.rerail`'s comment on it, or Soufflé's. A NULL argument, a regex that does not
//! compile, or a value Soufflé has none for (`to_number("a")`, `x % 0`) gives NULL: no value, so the rule gives no
//! row for it.

use std::cell::RefCell;
use std::collections::HashMap;

use rusqlite::Connection;
use rusqlite::functions::{Context, FunctionFlags};
use rusqlite::types::Value;

use crate::code_rule_dsl::facts::vtab::syntax::{LONG_TEXT_NODES, NODE_READS};

/// An argument or result: a symbol (text) or a number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Number,
}

use Kind::{Number, Text};

/// Each function: its name, its arguments and its result.
const FUNCTIONS: &[(&str, &[Kind], Kind)] = &[
    ("qname", &[Text, Text], Text),
    ("pkg_up", &[Text, Number], Text),
    ("head", &[Text], Text),
    ("dir", &[Text], Text),
    ("join", &[Text, Text], Text),
    ("glob_sub", &[Text, Text, Text], Text),
    ("package_of", &[Text], Text),
    ("replace_all", &[Text, Text, Text], Text),
    ("convert", &[Text, Text, Text], Text),
    ("slice", &[Text, Number, Number], Text),
    ("count_matches", &[Text, Text], Number),
    ("sha256", &[Text], Text),
    // Git objects of the repository holding the run's root (`strings::git`): never stale, so never invalidated.
    ("git_commit", &[Text], Number),
    ("git_has", &[Text, Text], Number),
    ("git_tracked", &[Text], Number),
    ("json_str", &[Text], Text),
    ("nfkc", &[Text], Text),
    // JavaScript's `Number.prototype.toPrecision` of a decimal numeral's value (`value::ecma::to_precision`):
    // "Infinity" past the doubles, none for a non-number or digits outside 1-100 (JavaScript's RangeError).
    ("to_precision", &[Text, Number], Text),
    // JavaScript's `toLowerCase`: Unicode's default case mapping (SQLite's own `lower` is ASCII only).
    ("lower", &[Text], Text),
    // `.rerail`'s `cat(a, b, …)`, two at a time: the native engine's concatenation (SQL concatenates with `||`).
    ("str_concat", &[Text, Text], Text),
    // `str_concat` run backwards: the text between a prefix and a suffix, none when the text has not both
    // (`simplify::concat_inverses`: `$v = cat($a, $x, $b)` with `$v` bound binds `$x`).
    ("str_between", &[Text, Text, Text], Text),
    // -1, 0 or 1 as `a` sorts before, with or after `b` (by bytes): the order of two symbols (`plan::lower`).
    ("str_cmp", &[Text, Text], Number),
    // Soufflé's `to_number(s)`, `max(a, b)`, `min(a, b)` and `a % b` (`mod`, with no value for a zero divisor).
    ("to_number", &[Text], Number),
    ("max", &[Number, Number], Number),
    ("min", &[Number, Number], Number),
    ("mod", &[Number, Number], Number),
    // Soufflé's floats, as keys that order as the floats do (`float_key`): `to_float`, arithmetic, `to_string`.
    ("to_float", &[Text], Number),
    ("float_add", &[Number, Number], Number),
    ("float_sub", &[Number, Number], Number),
    ("float_mul", &[Number, Number], Number),
    ("float_div", &[Number, Number], Number),
    ("float_str", &[Number], Text),
    // `.rerail`'s `match("(?<x>…)", s)`: the named group's text (`strings::group`), none when the regex does not match.
    ("rx_group", &[Text, Text, Text], Text),
    // `.rerail`'s integer arithmetic, `strlen` and `to_string`, as the native engine computes them (`native_call`); SQL
    // lowers the calls to its own operators, and these names give it the same functions. No value on overflow or
    // for a zero divisor; division truncates, as SQLite's does.
    ("rr_add", &[Number, Number], Number),
    ("rr_sub", &[Number, Number], Number),
    ("rr_mul", &[Number, Number], Number),
    ("rr_div", &[Number, Number], Number),
    ("rr_max", &[Number, Number], Number),
    ("rr_min", &[Number, Number], Number),
    // `.rerail`'s `@place(path, line)`, `@place_path` and `@place_line` (`plan::sql_term`'s encoding): a symbol is its
    // atom, so the path is a number here. `-2 - (path << 32 | line)`, `unset` (-1) for a line outside `0 .. 2^32`.
    ("rr_place", &[Number, Number], Number),
    ("rr_place_path", &[Number], Number),
    ("rr_place_line", &[Number], Number),
    // Characters, as SQLite's `length` counts a text's.
    ("rr_strlen", &[Text], Number),
    // SQLite's string functions, as the data-layer programs' SQL stages read them (decision 0107): positions from 1,
    // characters not bytes; `sql_substr2` is SQLite's `substr(x, y)`, `sql_trim2` its `trim(x, chars)`.
    ("sql_instr", &[Text, Text], Number),
    ("sql_substr", &[Text, Number, Number], Text),
    ("sql_substr2", &[Text, Number], Text),
    // `substr(CAST(x AS BLOB), y, z)`: bytes, not characters (a node's bytes between two offsets).
    ("sql_substr_bytes", &[Text, Number, Number], Text),
    ("sql_length", &[Text], Number),
    ("sql_trim", &[Text], Text),
    ("sql_trim2", &[Text, Text], Text),
    ("sql_ltrim", &[Text], Text),
    ("sql_ltrim2", &[Text, Text], Text),
    ("sql_rtrim2", &[Text, Text], Text),
    ("sql_upper", &[Text], Text),
    ("sql_lower", &[Text], Text),
    ("sql_replace", &[Text, Text, Text], Text),
    ("sql_like", &[Text, Text], Number),
    ("sql_cast_int", &[Text], Number),
    // `char(n)`: the character of code point n (the data-layer keys' `char(0)` separator).
    ("sql_char", &[Number], Text),
    // The data-layer programs' regex functions, as W's SQL has them (`residency::admission::relational`): Rust's
    // regex; `rx_extract`'s group, NULL without a match; `rx_replace` every match.
    ("rx_extract", &[Text, Text, Number], Text),
    ("rx_replace", &[Text, Text, Text], Text),
    ("rr_to_string", &[Number], Text),
    // C's `%0Nd`: an integer's digits zero-padded to `width` columns, the sign first, never truncated (`@printf`'s
    // lowering, `rr::lower::printf_pieces`).
    ("zero_pad", &[Number, Number], Text),
    // A node's text past `text`'s inline bound (`facts::vtab::syntax::LongTexts::long_text`): none for a shorter one, which
    // `text` holds. The long-text twin of a rule reading `text(v, t)` reads it (`syntax::long_text_twins`).
    ("rr_node_text", &[Number], Text),
    // A node's exact text at any length (`facts::vtab::syntax::LongTexts::text_any`): `node_text`'s answer (decision 0174).
    ("rr_node_text_any", &[Number], Text),
    // A node's value as W reads it (`facts::vtab::syntax::LongTexts::value_any`): `value`'s answer (decision 0174).
    ("rr_node_value", &[Number], Text),
    // `.rerail`'s `substr(s, i, n)`: n characters from the i-th, counted from 0, as SQL's `substr(s, i + 1, n)` gives
    // them (`substr`).
    ("rr_substr", &[Text, Number, Number], Text),
    // `core.rerail`'s semver functors, as node-semver reads ranges (`semver::Range`): `semver_valid` 1 or 0;
    // `semver_subset` none unless both are ranges (an evaluation error in W, which gives no row).
    ("semver_valid", &[Text], Number),
    ("semver_subset", &[Text, Text], Number),
    // Python's `repr` of a string (`cpython::value::str_repr`), and an Ed25519 signature's check (`crypto`).
    ("py_repr", &[Text], Text),
    ("ed25519_ok", &[Text, Text, Text], Number),
    // Rust's Unicode character properties, in the toolchain's version (as Ruff reads them), not regex tables.
    ("is_lowercase", &[Text], Number),
    ("is_uppercase", &[Text], Number),
    ("is_cased_lowercase", &[Text], Number),
    ("is_cased_uppercase", &[Text], Number),
    // 1 when Ruff's parser parses the text as a Python module without a syntax error, else 0 (`install_py_parses`).
    ("py_parses", &[Text], Number),
    ("py_consecutive_names", &[Text], Number),
];

/// The function the native engine computes `.rerail`'s call `f` with: `f` itself when it is one of these, or its
/// `rr_` name (`@add`, `@strlen`, …).
pub fn native_call(f: &str) -> Option<(&'static str, &'static [Kind], Kind)> {
    match f {
        "add" | "sub" | "mul" | "div" | "max" | "min" | "strlen" | "to_string" | "substr" | "place" | "place_path" | "place_line" => signature(&format!("rr_{f}")),
        _ => signature(f),
    }
}

/// The function a data-layer program's SQL builtin `@f` with `n` arguments is (decision 0107: t17's catalogue, the
/// names its SQL stages used), and whether its first two arguments swap (`@like(x, pattern)` is SQLite's
/// `like(pattern, x)`). `@lower` and `@substr` are not here: `.rerail` has its own of those names.
pub fn catalogue(f: &str, n: usize) -> Option<(&'static str, bool)> {
    Some(match (f, n) {
        ("instr", 2) => ("sql_instr", false),
        ("length", 1) => ("sql_length", false),
        ("trim", 1) => ("sql_trim", false),
        ("trim", 2) => ("sql_trim2", false),
        ("ltrim", 1) => ("sql_ltrim", false),
        ("ltrim", 2) => ("sql_ltrim2", false),
        ("rtrim", 2) => ("sql_rtrim2", false),
        ("upper", 1) => ("sql_upper", false),
        ("replace", 3) => ("sql_replace", false),
        ("like", 2) => ("sql_like", true),
        ("cast_int", 1) => ("sql_cast_int", false),
        ("char", 1) => ("sql_char", false),
        ("substr_bytes", 3) => ("sql_substr_bytes", false),
        ("rx_matches", 2) => ("rx", false),
        _ => return None,
    })
}

/// Every function: its name, arguments and result (the executor coverage gate probes each, `plan::coverage`).
pub fn all() -> &'static [(&'static str, &'static [Kind], Kind)] {
    FUNCTIONS
}

/// A function's name (as SQL names it), arguments and result, if `name` is one of these.
pub fn signature(name: &str) -> Option<(&'static str, &'static [Kind], Kind)> {
    FUNCTIONS.iter().find(|(n, _, _)| *n == name).copied()
}

/// Registers every function on `db`.
pub(in crate::code_rule_dsl) fn register(db: &Connection) -> rusqlite::Result<()> {
    for &(name, args, _) in FUNCTIONS.iter().filter(|(name, _, _)| !NODE_READS.contains(name)) {
        let arity = i32::try_from(args.len()).expect("few arguments");
        db.create_scalar_function(name, arity, FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC, move |ctx| Ok(call(name, args, ctx)))?;
    }
    // A node's text or value, from the batch this connection's run binds (`node_read`).
    let at = crate::code_rule_dsl::facts::vtab::Scoped::at(db);
    for name in NODE_READS {
        db.create_scalar_function(name, 1, FunctionFlags::SQLITE_UTF8, move |ctx| {
            let text = ctx.get::<Option<i64>>(0)?.and_then(|n| node_read(name, crate::code_rule_dsl::facts::vtab::bound_source_at(at, LONG_TEXT_NODES).as_deref(), n));
            Ok(text.map_or(Value::Null, Value::Text))
        })?;
    }
    db.create_aggregate_function("rr_product", 1, FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC, Product)?;
    Ok(())
}

/// A node read (`syntax::NODE_READS`) of node `n`: `@rr_node_text`'s text past `text`'s inline bound,
/// `@rr_node_text_any`'s text at any length, `@rr_node_value`'s value, read through `nodes`, the run's binding of the
/// long-text nodes with their batch (`syntax::LongTexts`); none without one. Its value is the run's, so `eval` has none.
pub fn node_read(name: &str, nodes: Option<&dyn crate::code_rule_dsl::facts::vtab::Source>, n: i64) -> Option<String> {
    crate::code_rule_dsl::facts::vtab::syntax::LongTexts::of(nodes?, name, n)
}

/// `product x : {…}` as the SQL aggregate `rr_product(x)`: the product of the integers, saturating at i64's range;
/// NULL over no rows (the caller's `coalesce` makes that 1), and NULL arguments are skipped as `sum` skips them.
struct Product;

impl rusqlite::functions::Aggregate<Option<i64>, Option<i64>> for Product {
    fn init(&self, _: &mut Context<'_>) -> rusqlite::Result<Option<i64>> {
        Ok(None)
    }

    fn step(&self, ctx: &mut Context<'_>, acc: &mut Option<i64>) -> rusqlite::Result<()> {
        if let Some(v) = ctx.get::<Option<i64>>(0)? {
            *acc = Some(acc.map_or(v, |a| a.saturating_mul(v)));
        }
        Ok(())
    }

    fn finalize(&self, _: &mut Context<'_>, acc: Option<Option<i64>>) -> rusqlite::Result<Option<i64>> {
        Ok(acc.flatten())
    }
}
fn call(name: &str, kinds: &[Kind], ctx: &Context) -> Value {
    let mut text = Vec::new();
    let mut numbers = Vec::new();
    for (i, kind) in kinds.iter().enumerate() {
        match kind {
            Text => match ctx.get::<Option<String>>(i) {
                Ok(Some(s)) => text.push(s),
                _ => return Value::Null,
            },
            Number => match ctx.get::<Option<i64>>(i) {
                Ok(Some(n)) => numbers.push(n),
                _ => return Value::Null,
            },
        }
    }
    let text: Vec<&str> = text.iter().map(String::as_str).collect();
    eval(name, &text, &numbers)
}

/// Function `name` of its text and number arguments, in its signature's order within each kind (`signature`):
/// the value SQL's function of that name gives, NULL for none. The native engine's `func` step calls it too.
/// ECMAScript's `Number.prototype.toPrecision` of a finite number (`@to_precision`), registered at load by the crate
/// that knows the language (`value::ecma`): this crate is below it.
static TO_PRECISION: std::sync::OnceLock<fn(f64, u32) -> String> = std::sync::OnceLock::new();

/// Registers `@to_precision`'s formatting (once, at load).
pub fn install_to_precision(to: fn(f64, u32) -> String) {
    let _ = TO_PRECISION.set(to);
}

/// Python's `repr` of a string (`@py_repr`), registered at load by the crate that knows the language
/// (`cpython::value::str_repr`): this crate is below it.
static PY_REPR: std::sync::OnceLock<fn(&str) -> String> = std::sync::OnceLock::new();

/// Registers `@py_repr`'s formatting (once, at load).
pub fn install_py_repr(repr: fn(&str) -> String) {
    let _ = PY_REPR.set(repr);
}

/// Whether Ruff's parser parses a text as a Python module (`@py_parses`), registered at load by the crate that knows
/// the language (`cpython::syntax::parses`): this crate is below it.
static PY_PARSES: std::sync::OnceLock<fn(&str) -> bool> = std::sync::OnceLock::new();

/// Registers `@py_parses`'s parser (once, at load).
pub fn install_py_parses(parses: fn(&str) -> bool) {
    let _ = PY_PARSES.set(parses);
}

/// Ruff's token-level prose test (`@py_consecutive_names`), installed by the language owner.
static PY_CONSECUTIVE_NAMES: std::sync::OnceLock<fn(&str) -> bool> = std::sync::OnceLock::new();

/// Registers `@py_consecutive_names`'s tokenizer predicate (once, at load).
pub fn install_py_consecutive_names(consecutive: fn(&str) -> bool) {
    let _ = PY_CONSECUTIVE_NAMES.set(consecutive);
}

/// A cased character exists and none has the opposite case, using Rust's Unicode character properties.
fn cased(s: &str, has_case: fn(char) -> bool, opposite: fn(char) -> bool) -> bool {
    let mut found = false;
    for c in s.chars() {
        if opposite(c) {
            return false;
        }
        found = found || has_case(c);
    }
    found
}

pub fn eval(name: &str, text: &[&str], numbers: &[i64]) -> Value {
    match name {
        "qname" | "pkg_up" | "head" | "dir" | "join" | "glob_sub" | "package_of" => text_value(eval_path(name, text, numbers)),
        "replace_all" | "convert" | "slice" | "rx_group" | "rx_extract" | "rx_replace" | "str_between" => text_value(eval_pattern(name, text, numbers)),
        "lower" | "sha256" | "json_str" | "str_concat" | "nfkc" | "py_repr" | "to_precision" => text_value(eval_format(name, text, numbers)),
        "py_parses" | "py_consecutive_names" | "is_lowercase" | "is_uppercase" | "is_cased_lowercase" | "is_cased_uppercase" | "ed25519_ok" => eval_language(name, text),
        "git_commit" | "git_has" | "git_tracked" | "semver_valid" | "semver_subset" | "count_matches" | "str_cmp" | "to_number" => eval_query(name, text),
        "to_float" | "float_add" | "float_sub" | "float_mul" | "float_div" | "float_str" => eval_float(name, text, numbers),
        "sql_instr" | "sql_substr" | "sql_substr2" | "sql_substr_bytes" | "sql_length" | "sql_trim" | "sql_trim2" | "sql_ltrim" | "sql_ltrim2" | "sql_rtrim2" | "sql_upper" | "sql_lower" | "sql_replace" | "sql_like" | "sql_cast_int" | "sql_char" => eval_sql(name, text, numbers),
        _ => eval_number(name, text, numbers),
    }
}

fn text_value(value: Option<String>) -> Value {
    value.map_or(Value::Null, Value::Text)
}

fn eval_path(name: &str, text: &[&str], numbers: &[i64]) -> Option<String> {
    Some(match name {
        "qname" => format!("{}#{}", text[0], text[1]),
        "pkg_up" => pkg_up(text[0], numbers[0]),
        "head" => text[0].split('.').next().unwrap_or("").to_owned(),
        "dir" => text[0].rsplit_once('/').map_or("", |(d, _)| d).to_owned(),
        "join" => join(text[0], text[1]),
        "glob_sub" => glob_sub(text[0], text[1], text[2]),
        _ => text[0].to_owned(),
    })
}

fn eval_pattern(name: &str, text: &[&str], numbers: &[i64]) -> Option<String> {
    match name {
        "replace_all" => with_regex(text[1], |re| re.replace_all(text[0], text[2]).into_owned()),
        "convert" => Some(convert(text[0], text[1], text[2])),
        "slice" => Some(slice(text[0], numbers[0], numbers[1])),
        "rx_group" => super::group(text[0], text[1], text[2]),
        "rx_extract" => with_regex(text[1], |re| re.captures(text[0])?.get(usize::try_from(numbers[0]).ok()?).map(|m| m.as_str().to_owned())).flatten(),
        "rx_replace" => with_regex(text[1], |re| re.replace_all(text[0], text[2]).into_owned()),
        _ => text[0].strip_prefix(text[1]).and_then(|rest| rest.strip_suffix(text[2])).map(str::to_owned),
    }
}

fn eval_format(name: &str, text: &[&str], numbers: &[i64]) -> Option<String> {
    use sha2::Digest;
    use unicode_normalization::UnicodeNormalization;
    match name {
        "lower" => Some(text[0].to_lowercase()),
        "sha256" => Some(sha2::Sha256::digest(text[0].as_bytes()).iter().map(|b| format!("{b:02x}")).collect()),
        "json_str" => Some(json_str(text[0])),
        "str_concat" => Some(format!("{}{}", text[0], text[1])),
        "nfkc" => Some(text[0].nfkc().collect()),
        "py_repr" => PY_REPR.get().map(|repr| repr(text[0])),
        _ => precision(text[0], numbers[0]),
    }
}

fn precision(text: &str, digits: i64) -> Option<String> {
    let f = text.trim().parse::<f64>().ok().filter(|f| !f.is_nan() && (1..=100).contains(&digits))?;
    if f.is_infinite() {
        Some(if f > 0.0 { "Infinity".to_owned() } else { "-Infinity".to_owned() })
    } else {
        TO_PRECISION.get().map(|to| to(f, digits as u32))
    }
}

fn eval_language(name: &str, text: &[&str]) -> Value {
    let result = match name {
        "py_parses" => return PY_PARSES.get().map_or(Value::Null, |parses| Value::Integer(i64::from(parses(text[0])))),
        "py_consecutive_names" => return PY_CONSECUTIVE_NAMES.get().map_or(Value::Null, |consecutive| Value::Integer(i64::from(consecutive(text[0])))),
        "is_lowercase" => text[0].chars().all(|c| !c.is_alphabetic() || c.is_lowercase()),
        "is_uppercase" => text[0].chars().all(|c| !c.is_alphabetic() || c.is_uppercase()),
        "is_cased_lowercase" => cased(text[0], char::is_lowercase, char::is_uppercase),
        "is_cased_uppercase" => cased(text[0], char::is_uppercase, char::is_lowercase),
        _ => crate::code_rule_dsl::logic::crypto::ed25519_ok(text[0], text[1], text[2]),
    };
    Value::Integer(i64::from(result))
}

fn eval_query(name: &str, text: &[&str]) -> Value {
    use crate::code_rule_dsl::semver::Range;
    match name {
        "git_commit" => super::git::git_commit(text[0]),
        "git_has" => super::git::git_has(text[0], text[1]),
        "git_tracked" => super::git::git_tracked(text[0]),
        "semver_valid" => Value::Integer(i64::from(Range::parse(text[0]).is_some())),
        "semver_subset" => Range::parse(text[0]).zip(Range::parse(text[1])).map_or(Value::Null, |(a, b)| Value::Integer(i64::from(a.is_subset_of(&b)))),
        "count_matches" => with_regex(text[0], |re| Value::Integer(re.find_iter(text[1]).count() as i64)).unwrap_or(Value::Null),
        "str_cmp" => Value::Integer(text[0].cmp(text[1]) as i64),
        _ => to_number(text[0]).map_or(Value::Null, Value::Integer),
    }
}

fn eval_float(name: &str, text: &[&str], numbers: &[i64]) -> Value {
    match name {
        "to_float" => text[0].trim().parse::<f64>().ok().filter(|f| !f.is_nan()).map_or(Value::Null, |f| Value::Integer(float_key(f))),
        "float_str" => Value::Text(float_of(numbers[0]).to_string()),
        _ => {
            let f = float_arithmetic(name, float_of(numbers[0]), float_of(numbers[1]));
            if f.is_nan() { Value::Null } else { Value::Integer(float_key(f)) }
        }
    }
}

fn float_arithmetic(name: &str, a: f64, b: f64) -> f64 {
    match name {
        "float_add" => a + b,
        "float_sub" => a - b,
        "float_mul" => a * b,
        _ => a / b,
    }
}

fn eval_number(name: &str, text: &[&str], numbers: &[i64]) -> Value {
    let value = match name {
        "rr_add" => numbers[0].checked_add(numbers[1]),
        "rr_sub" => numbers[0].checked_sub(numbers[1]),
        "rr_mul" => numbers[0].checked_mul(numbers[1]),
        "rr_div" => numbers[0].checked_div(numbers[1]),
        "rr_max" | "max" => Some(numbers[0].max(numbers[1])),
        "rr_min" | "min" => Some(numbers[0].min(numbers[1])),
        "mod" => numbers[0].checked_rem(numbers[1]),
        _ => return eval_node(name, text, numbers),
    };
    value.map_or(Value::Null, Value::Integer)
}

fn eval_node(name: &str, text: &[&str], numbers: &[i64]) -> Value {
    match name {
        "rr_place" => {
            let (p, l) = (numbers[0], numbers[1]);
            let place = (0..1i64 << 32).contains(&l).then(|| p.checked_mul(1 << 32).and_then(|x| x.checked_add(l)).and_then(|x| (-2i64).checked_sub(x))).flatten();
            Value::Integer(place.unwrap_or(-1))
        }
        "rr_place_path" => (-2i64).checked_sub(numbers[0]).map_or(Value::Null, |n| Value::Integer(n / (1 << 32))),
        "rr_place_line" => (-2i64).checked_sub(numbers[0]).map_or(Value::Null, |n| Value::Integer(n - n / (1 << 32) * (1 << 32))),
        "rr_strlen" => Value::Integer(text[0].chars().count() as i64),
        "rr_substr" => Value::Text(substr(text[0], numbers[0], numbers[1])),
        "rr_to_string" => Value::Text(numbers[0].to_string()),
        "zero_pad" => Value::Text(format!("{:0w$}", numbers[0], w = usize::try_from(numbers[1].clamp(0, 64)).unwrap_or(0))),
        _ => Value::Null,
    }
}

fn eval_sql(name: &str, text: &[&str], numbers: &[i64]) -> Value {
    match name {
        "sql_instr" => Value::Integer(text[0].find(text[1]).map_or(0, |at| text[0][..at].chars().count() as i64 + 1)),
        "sql_length" => Value::Integer(text[0].chars().count() as i64),
        "sql_like" => Value::Integer(i64::from(sql_like(text[1], text[0]))),
        "sql_cast_int" => Value::Integer(sql_cast_int(text[0])),
        "sql_char" => Value::Text(u32::try_from(numbers[0]).ok().and_then(char::from_u32).unwrap_or('\u{FFFD}').to_string()),
        "sql_substr" | "sql_substr2" | "sql_substr_bytes" => text_value(eval_sql_substr(name, text[0], numbers)),
        _ => text_value(eval_sql_text(name, text)),
    }
}

fn eval_sql_substr(name: &str, text: &str, numbers: &[i64]) -> Option<String> {
    match name {
        "sql_substr" => Some(sql_substr(text, numbers[0], Some(numbers[1]))),
        "sql_substr2" => Some(sql_substr(text, numbers[0], None)),
        _ => (!text.is_empty()).then(|| sql_substr_bytes(text, numbers[0], numbers[1])),
    }
}

fn eval_sql_text(name: &str, text: &[&str]) -> Option<String> {
    Some(match name {
        "sql_trim" => text[0].trim_matches(' ').to_owned(),
        "sql_trim2" => text[0].trim_matches(|c| text[1].contains(c)).to_owned(),
        "sql_ltrim" => text[0].trim_start_matches(' ').to_owned(),
        "sql_ltrim2" => text[0].trim_start_matches(|c| text[1].contains(c)).to_owned(),
        "sql_rtrim2" => text[0].trim_end_matches(|c| text[1].contains(c)).to_owned(),
        "sql_upper" => text[0].to_ascii_uppercase(),
        "sql_lower" => text[0].to_ascii_lowercase(),
        _ => if text[1].is_empty() { text[0].to_owned() } else { text[0].replace(text[1], text[2]) },
    })
}

/// SQLite's `substr(x, y, z)` over characters (`z` none for `substr(x, y)`): `y` from 1, or from the end where
/// negative; `y` 0 takes one character fewer; a negative `z` takes the characters before `y`.
pub(in crate::code_rule_dsl) fn sql_substr(x: &str, y: i64, z: Option<i64>) -> String {
    let chars: Vec<char> = x.chars().collect();
    let (start, end) = substr_range(chars.len(), y, z);
    chars[start..end].iter().collect()
}

/// SQLite's `substr(CAST(x AS BLOB), y, z)` read back as text: `sql_substr` over bytes (a cut inside a character
/// reads as U+FFFD, where SQLite would keep the bytes).
fn sql_substr_bytes(x: &str, y: i64, z: i64) -> String {
    let (start, end) = substr_range(x.len(), y, Some(z));
    String::from_utf8_lossy(&x.as_bytes()[start..end]).into_owned()
}

/// `substr`'s range of `len` items, as SQLite's `substrFunc` computes it.
fn substr_range(len: usize, y: i64, z: Option<i64>) -> (usize, usize) {
    let len = len as i64;
    let (mut p1, mut p2) = (y, z.unwrap_or(i64::MAX / 2));
    let negative = p2 < 0;
    if negative {
        p2 = -p2;
    }
    if p1 < 0 {
        p1 += len;
        if p1 < 0 {
            p2 += p1;
            if p2 < 0 {
                p2 = 0;
            }
            p1 = 0;
        }
    } else if p1 > 0 {
        p1 -= 1;
    } else if p2 > 0 {
        p2 -= 1;
    }
    if negative {
        p1 -= p2;
        if p1 < 0 {
            p2 += p1;
            p1 = 0;
        }
    }
    let start = p1.clamp(0, len) as usize;
    let end = p1.saturating_add(p2).clamp(0, len) as usize;
    (start, end.max(start))
}

/// SQLite's `x LIKE pattern` (`like(pattern, x)`): `%` any run, `_` one character, ASCII letters in either case.
pub(in crate::code_rule_dsl) fn sql_like(pattern: &str, x: &str) -> bool {
    fn at(p: &[char], s: &[char]) -> bool {
        match p.split_first() {
            None => s.is_empty(),
            Some(('%', rest)) => (0..=s.len()).any(|k| at(rest, &s[k..])),
            Some(('_', rest)) => !s.is_empty() && at(rest, &s[1..]),
            Some((c, rest)) => s.first().is_some_and(|d| c.eq_ignore_ascii_case(d)) && at(rest, &s[1..]),
        }
    }
    at(&pattern.chars().collect::<Vec<_>>(), &x.chars().collect::<Vec<_>>())
}

/// SQLite's `CAST(x AS INTEGER)` of text: its leading integer (after spaces, with a sign), 0 where it has none.
pub(in crate::code_rule_dsl) fn sql_cast_int(x: &str) -> i64 {
    let t = x.trim_start();
    let (sign, digits) = match t.as_bytes().first() {
        Some(b'-') => (-1, &t[1..]),
        Some(b'+') => (1, &t[1..]),
        _ => (1, t),
    };
    let end = digits.bytes().position(|b| !b.is_ascii_digit()).unwrap_or(digits.len());
    digits[..end].parse::<i64>().map_or(0, |v| sign * v)
}

/// Soufflé's `to_number(s)`: `0x…` hexadecimal and `0b…` binary (either after a `-`), else decimal; as C's
/// `strtoll`, leading white space and a sign are read and the text after the number's digits is ignored
/// (`to_number(" 12px")` = 12). `None` where Soufflé stops with an error: no digits (`"a"`, `""`, `"0b"`) or out of
/// range.
pub(in crate::code_rule_dsl) fn to_number(s: &str) -> Option<i64> {
    if let Some(rest) = s.strip_prefix("-0b") {
        return strtoll(&format!("-{rest}"), 2);
    }
    if let Some(rest) = s.strip_prefix("0b") {
        return strtoll(rest, 2);
    }
    strtoll(s, if s.starts_with("0x") || s.starts_with("-0x") { 16 } else { 10 })
}

/// C's `strtoll(s, base)` as C++'s `stoll` uses it: `None` where `stoll` throws (no digits, out of range).
fn strtoll(s: &str, base: u32) -> Option<i64> {
    let s = s.trim_start_matches([' ', '\t', '\n', '\u{b}', '\u{c}', '\r']);
    let (neg, s) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    // Base 16 reads an optional `0x` when a hex digit follows it (else the `0` alone is the number).
    let s = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(rest) if base == 16 && rest.starts_with(|c: char| c.is_ascii_hexdigit()) => rest,
        _ => s,
    };
    let digits = s.find(|c: char| !c.is_digit(base)).unwrap_or(s.len());
    if digits == 0 {
        return None;
    }
    let magnitude = i128::from_str_radix(&s[..digits], base).ok()?;
    i64::try_from(if neg { -magnitude } else { magnitude }).ok()
}

/// A float as an integer key that orders as the floats do: its bits, with every bit but the sign flipped for a
/// negative float; -0.0 is 0.0. The engine's values are integers (decision 0008), so a float column holds these.
pub fn float_key(f: f64) -> i64 {
    let bits = (if f == 0.0 { 0.0 } else { f }).to_bits() as i64;
    if bits < 0 { bits ^ i64::MAX } else { bits }
}

/// The float of a key (`float_key`).
pub(in crate::code_rule_dsl) fn float_of(key: i64) -> f64 {
    f64::from_bits((if key < 0 { key ^ i64::MAX } else { key }) as u64)
}

/// `f(re)` with `re` compiled once per thread; `None` if it does not compile.
fn with_regex<T>(re: &str, f: impl FnOnce(&regex::Regex) -> T) -> Option<T> {
    thread_local! {
        static CACHE: RefCell<HashMap<String, Option<regex::Regex>>> = RefCell::new(HashMap::new());
    }
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let compiled = cache.entry(re.to_owned()).or_insert_with(|| regex::Regex::new(re).ok());
        compiled.as_ref().map(f)
    })
}

/// `@pkg_up("a.b.c", 1)` = `"a.b"`: `q` without its last `k` dotted parts; `""` past the root.
pub fn pkg_up(q: &str, k: i64) -> String {
    let parts: Vec<&str> = if q.is_empty() { Vec::new() } else { q.split('.').collect() };
    let keep = parts.len().saturating_sub(usize::try_from(k.max(0)).unwrap_or(usize::MAX));
    parts[..keep].join(".")
}

/// `@join("src/a", "../c")` = `"src/c"`: the path `rel` names from `dir`, with `.` and `..` resolved.
pub fn join(dir: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = if rel.starts_with('/') { Vec::new() } else { dir.split('/').filter(|p| !p.is_empty()).collect() };
    for p in rel.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

/// `@glob_sub("@/*", "src/*", "@/lib/x")` = `"src/lib/x"`: `s` matched against `pat` (one `*`, or none), and what
/// the `*` matched put in `to`'s `*`; `""` if it does not match.
pub fn glob_sub(pat: &str, to: &str, s: &str) -> String {
    match pat.split_once('*') {
        None if s == pat => to.to_owned(),
        None => String::new(),
        Some((before, after)) if s.len() >= before.len() + after.len() && s.starts_with(before) && s.ends_with(after) => {
            to.replacen('*', &s[before.len()..s.len() - after.len()], 1)
        }
        Some(_) => String::new(),
    }
}

/// `@slice("_name", 1, 5)` = `"name"`: characters `start` to `end - 1`, a negative position counting from the end.
/// `.rerail`'s `substr(s, i, n)` as SQLite's `substr(s, i + 1, n)` computes it (its `substrFunc`), over characters: a
/// start before the text counts from its end, a negative length takes the characters before the start.
pub(in crate::code_rule_dsl) fn substr(s: &str, i: i64, n: i64) -> String {
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len() as i64;
    let (mut p1, mut p2) = (i.saturating_add(1), n);
    let neg = p2 < 0;
    if neg {
        p2 = p2.saturating_neg();
    }
    if p1 < 0 {
        p1 += len;
        if p1 < 0 {
            p2 = (p2 + p1).max(0);
            p1 = 0;
        }
    } else if p1 > 0 {
        p1 -= 1;
    } else if p2 > 0 {
        p2 -= 1;
    }
    if neg {
        p1 -= p2;
        if p1 < 0 {
            p2 += p1;
            p1 = 0;
        }
    }
    let a = p1.clamp(0, len);
    let b = p1.saturating_add(p2).clamp(a, len);
    chars[a as usize..b as usize].iter().collect()
}

pub fn slice(s: &str, start: i64, end: i64) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len() as i64;
    let at = |i: i64| (if i < 0 { n + i } else { i }).clamp(0, n) as usize;
    let (a, b) = (at(start), at(end));
    if a >= b { String::new() } else { chars[a..b].iter().collect() }
}

/// `@convert("max-age", "camelCase", "dash")` = `"maxAge"`. `seps` names the separators (`dash`, `underscore`,
/// `dot`, `space`, `slash`, `caseChange`), comma-separated; `""` is all of them.
pub fn convert(s: &str, to: &str, seps: &str) -> String {
    let all = seps.trim().is_empty();
    let named: Vec<&str> = seps.split(',').map(str::trim).collect();
    let on = |name: &str| all || named.contains(&name);
    let sep = |c: char| match c {
        '-' => on("dash"),
        '_' => on("underscore"),
        '.' => on("dot"),
        ' ' => on("space"),
        '/' => on("slash"),
        _ => false,
    };
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut prev_lower = false;
    for c in s.chars() {
        if sep(c) {
            words.push(std::mem::take(&mut word));
            prev_lower = false;
            continue;
        }
        if on("caseChange") && c.is_uppercase() && prev_lower {
            words.push(std::mem::take(&mut word));
        }
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
        word.push(c);
    }
    words.push(word);
    let words: Vec<String> = words.into_iter().filter(|w| !w.is_empty()).collect();
    let cap = |w: &str| {
        let mut c = w.chars();
        c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c.flat_map(char::to_lowercase)).collect())
    };
    match to {
        "lowerCase" => s.to_lowercase(),
        "upperCase" => s.to_uppercase(),
        "capitalize" => {
            let mut c = s.chars();
            c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
        }
        "camelCase" => words.iter().enumerate().map(|(i, w)| if i == 0 { w.to_lowercase() } else { cap(w) }).collect(),
        "pascalCase" => words.iter().map(|w| cap(w)).collect(),
        "snakeCase" => words.iter().map(|w| w.to_lowercase()).collect::<Vec<_>>().join("_"),
        "kebabCase" => words.iter().map(|w| w.to_lowercase()).collect::<Vec<_>>().join("-"),
        _ => s.to_owned(),
    }
}

/// `s` as a JSON string literal, quotes included, as W's signed-list stages write an element (SQLite's
/// `json_quote`/`json_group_array`, as `plan`'s SQL gives it): `"`, `\` and controls escaped, non-ASCII kept.
pub fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoking gun 6 (t34, 2026-11-13): a maintained stratum has no SQL fallback, and `@place` in a head (`spot(d,
    /// @place(p, 7))`) had no native form, so the edit failed. `@place`, `@place_path` and `@place_line` are native
    /// functions, and their values are those of SQL's encoding (`plan::sql_term`), the path a symbol's atom.
    #[test]
    fn a_place_is_native_and_encoded_as_sql_encodes_it() {
        for f in ["place", "place_path", "place_line"] {
            let (_, kinds, ret) = native_call(f).unwrap_or_else(|| panic!("@{f} has a native function"));
            assert!(kinds.iter().chain([&ret]).all(|k| *k == Kind::Number), "@{f} is over numbers: a symbol is its atom");
        }
        let int = |v: Value| match v {
            Value::Integer(n) => n,
            other => panic!("{other:?}"),
        };
        let db = rusqlite::Connection::open_in_memory().unwrap();
        for (p, l) in [(0, 0), (5, 7), (123_456, (1 << 32) - 1), (9, -1), (9, 1 << 32)] {
            let sql: i64 = db
                .query_row("SELECT CASE WHEN ?2 >= 0 AND ?2 < 4294967296 THEN -2 - (?1 * 4294967296 + ?2) ELSE -1 END", [p, l], |r| r.get(0))
                .unwrap();
            let place = int(eval("rr_place", &[], &[p, l]));
            assert_eq!(place, sql, "@place({p}, {l}) as SQL makes it");
            if sql != -1 {
                assert_eq!(int(eval("rr_place_path", &[], &[place])), p);
                assert_eq!(int(eval("rr_place_line", &[], &[place])), l);
            }
        }
    }
}
