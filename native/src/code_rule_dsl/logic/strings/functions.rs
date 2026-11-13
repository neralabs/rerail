//! The `core.rr` functors that compute text from text (`@head`, `@join`, `@replace_all`, …), and Soufflé's
//! built-in functions beyond SQL's (`to_number`, `max`, `min`, `%`), each a native SQL function of the same name
//! (`lower`, `max`, `min` and `mod` in place of SQLite's own). A rule's call `@f(a, b)` is lowered to `f(a, b)` in
//! SQL, its symbol arguments decoded to text and a symbol result encoded back (`plan`'s `text_of` / `symbol_of`).
//! Each function's meaning is `core.rr`'s comment on it, or Soufflé's. A NULL argument, a regex that does not
//! compile, or a value Soufflé has none for (`to_number("a")`, `x % 0`) gives NULL: no value, so the rule gives no
//! row for it.

use std::cell::RefCell;
use std::collections::HashMap;

use rusqlite::Connection;
use rusqlite::functions::{Context, FunctionFlags};
use rusqlite::types::Value;

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
    // `.rr`'s `cat(a, b, …)`, two at a time: the native engine's concatenation (SQL concatenates with `||`).
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
    // `.rr`'s `match("(?<x>…)", s)`: the named group's text (`strings::group`), none when the regex does not match.
    ("rx_group", &[Text, Text, Text], Text),
    // `.rr`'s integer arithmetic, `strlen` and `to_string`, as the native engine computes them (`native_call`); SQL
    // lowers the calls to its own operators, and these names give it the same functions. No value on overflow or
    // for a zero divisor; division truncates, as SQLite's does.
    ("rr_add", &[Number, Number], Number),
    ("rr_sub", &[Number, Number], Number),
    ("rr_mul", &[Number, Number], Number),
    ("rr_div", &[Number, Number], Number),
    ("rr_max", &[Number, Number], Number),
    ("rr_min", &[Number, Number], Number),
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
    // A node's text past `text`'s inline bound (`facts::vtab::syntax::long_node_text`): none for a shorter one, which
    // `text` holds. The long-text twin of a rule reading `text(v, t)` reads it (`syntax::long_text_twins`).
    ("rr_node_text", &[Number], Text),
    // `.rr`'s `substr(s, i, n)`: n characters from the i-th, counted from 0, as SQL's `substr(s, i + 1, n)` gives
    // them (`substr`).
    ("rr_substr", &[Text, Number, Number], Text),
    // `core.rr`'s semver functors, as node-semver reads ranges (`semver::Range`): `semver_valid` 1 or 0;
    // `semver_subset` none unless both are ranges (an evaluation error in W, which gives no row).
    ("semver_valid", &[Text], Number),
    ("semver_subset", &[Text, Text], Number),
    // Python's `repr` of a string (`cpython::value::str_repr`), and an Ed25519 signature's check (`crypto`).
    ("py_repr", &[Text], Text),
    ("ed25519_ok", &[Text, Text, Text], Number),
];

/// The function the native engine computes `.rr`'s call `f` with: `f` itself when it is one of these, or its
/// `rr_` name (`@add`, `@strlen`, …).
pub fn native_call(f: &str) -> Option<(&'static str, &'static [Kind], Kind)> {
    match f {
        "add" | "sub" | "mul" | "div" | "max" | "min" | "strlen" | "to_string" | "substr" => signature(&format!("rr_{f}")),
        _ => signature(f),
    }
}

/// The function a data-layer program's SQL builtin `@f` with `n` arguments is (decision 0107: t17's catalogue, the
/// names its SQL stages used), and whether its first two arguments swap (`@like(x, pattern)` is SQLite's
/// `like(pattern, x)`). `@lower` and `@substr` are not here: `.rr` has its own of those names.
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
    for &(name, args, _) in FUNCTIONS {
        let arity = i32::try_from(args.len()).expect("few arguments");
        db.create_scalar_function(name, arity, FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC, move |ctx| Ok(call(name, args, ctx)))?;
    }
    db.create_aggregate_function("rr_product", 1, FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC, Product)?;
    Ok(())
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

pub fn eval(name: &str, text: &[&str], numbers: &[i64]) -> Value {
    let t = |i: usize| text[i];
    let out = match name {
        "qname" => Some(format!("{}#{}", t(0), t(1))),
        "pkg_up" => Some(pkg_up(t(0), numbers[0])),
        "head" => Some(t(0).split('.').next().unwrap_or("").to_owned()),
        "str_between" => t(0).strip_prefix(t(1)).and_then(|rest| rest.strip_suffix(t(2))).map(str::to_owned),
        "dir" => Some(t(0).rsplit_once('/').map_or("", |(d, _)| d).to_owned()),
        "join" => Some(join(t(0), t(1))),
        "glob_sub" => Some(glob_sub(t(0), t(1), t(2))),
        "package_of" => Some(t(0).to_owned()),
        "replace_all" => with_regex(t(1), |re| re.replace_all(t(0), t(2)).into_owned()),
        "convert" => Some(convert(t(0), t(1), t(2))),
        "slice" => Some(slice(t(0), numbers[0], numbers[1])),
        "rx_group" => super::group(t(0), t(1), t(2)),
        "str_cmp" => return Value::Integer(t(0).cmp(t(1)) as i64),
        "git_commit" => return super::git::git_commit(t(0)),
        "git_has" => return super::git::git_has(t(0), t(1)),
        "git_tracked" => return super::git::git_tracked(t(0)),
        "rr_add" => return numbers[0].checked_add(numbers[1]).map_or(Value::Null, Value::Integer),
        "rr_sub" => return numbers[0].checked_sub(numbers[1]).map_or(Value::Null, Value::Integer),
        "rr_mul" => return numbers[0].checked_mul(numbers[1]).map_or(Value::Null, Value::Integer),
        "rr_div" => return numbers[0].checked_div(numbers[1]).map_or(Value::Null, Value::Integer),
        "rr_max" => return Value::Integer(numbers[0].max(numbers[1])),
        "rr_min" => return Value::Integer(numbers[0].min(numbers[1])),
        "rr_strlen" | "sql_length" => return Value::Integer(t(0).chars().count() as i64),
        "sql_instr" => return Value::Integer(t(0).find(t(1)).map_or(0, |at| t(0)[..at].chars().count() as i64 + 1)),
        "sql_substr" => Some(sql_substr(t(0), numbers[0], Some(numbers[1]))),
        "sql_substr2" => Some(sql_substr(t(0), numbers[0], None)),
        // SQLite's substr of an empty blob is NULL.
        "sql_substr_bytes" => (!t(0).is_empty()).then(|| sql_substr_bytes(t(0), numbers[0], numbers[1])),
        "sql_trim" => Some(t(0).trim_matches(' ').to_owned()),
        "sql_trim2" => Some(t(0).trim_matches(|c| t(1).contains(c)).to_owned()),
        "sql_ltrim" => Some(t(0).trim_start_matches(' ').to_owned()),
        "sql_ltrim2" => Some(t(0).trim_start_matches(|c| t(1).contains(c)).to_owned()),
        "sql_rtrim2" => Some(t(0).trim_end_matches(|c| t(1).contains(c)).to_owned()),
        "sql_upper" => Some(t(0).to_ascii_uppercase()),
        "sql_lower" => Some(t(0).to_ascii_lowercase()),
        "sql_replace" => Some(if t(1).is_empty() { t(0).to_owned() } else { t(0).replace(t(1), t(2)) }),
        "sql_like" => return Value::Integer(i64::from(sql_like(t(1), t(0)))),
        "sql_cast_int" => return Value::Integer(sql_cast_int(t(0))),
        "sql_char" => Some(u32::try_from(numbers[0]).ok().and_then(char::from_u32).unwrap_or('\u{FFFD}').to_string()),
        "rx_extract" => {
            let group = usize::try_from(numbers[0]).ok();
            return with_regex(t(1), |re| group.and_then(|g| re.captures(t(0))?.get(g).map(|m| Value::Text(m.as_str().to_owned())))).flatten().unwrap_or(Value::Null);
        }
        "rx_replace" => with_regex(t(1), |re| re.replace_all(t(0), t(2)).into_owned()),
        "rr_to_string" => Some(numbers[0].to_string()),
        "zero_pad" => Some(format!("{:0w$}", numbers[0], w = usize::try_from(numbers[1].clamp(0, 64)).unwrap_or(0))),
        "rr_node_text" => crate::code_rule_dsl::facts::vtab::syntax::long_node_text(numbers[0]),
        "rr_substr" => Some(substr(t(0), numbers[0], numbers[1])),
        "semver_valid" => return Value::Integer(i64::from(crate::code_rule_dsl::semver::Range::parse(t(0)).is_some())),
        "semver_subset" => {
            use crate::code_rule_dsl::semver::Range;
            return match (Range::parse(t(0)), Range::parse(t(1))) {
                (Some(a), Some(b)) => Value::Integer(i64::from(a.is_subset_of(&b))),
                _ => Value::Null,
            };
        }
        // No value where no language registered it (`install_py_repr`).
        "py_repr" => PY_REPR.get().map(|repr| repr(t(0))),
        "ed25519_ok" => return Value::Integer(i64::from(crate::code_rule_dsl::logic::crypto::ed25519_ok(t(0), t(1), t(2)))),
        "to_float" => return t(0).trim().parse::<f64>().ok().filter(|f| !f.is_nan()).map_or(Value::Null, |f| Value::Integer(float_key(f))),
        "float_add" | "float_sub" | "float_mul" | "float_div" => {
            let (a, b) = (float_of(numbers[0]), float_of(numbers[1]));
            let f = match name {
                "float_add" => a + b,
                "float_sub" => a - b,
                "float_mul" => a * b,
                _ => a / b,
            };
            return if f.is_nan() { Value::Null } else { Value::Integer(float_key(f)) };
        }
        "float_str" => Some(float_of(numbers[0]).to_string()),
        "count_matches" => return with_regex(t(0), |re| Value::Integer(re.find_iter(t(1)).count() as i64)).unwrap_or(Value::Null),
        "to_number" => return to_number(t(0)).map_or(Value::Null, Value::Integer),
        "max" => return Value::Integer(numbers[0].max(numbers[1])),
        "min" => return Value::Integer(numbers[0].min(numbers[1])),
        "mod" => return numbers[0].checked_rem(numbers[1]).map_or(Value::Null, Value::Integer),
        "lower" => Some(t(0).to_lowercase()),
        "sha256" => {
            use sha2::Digest;
            Some(sha2::Sha256::digest(t(0).as_bytes()).iter().map(|b| format!("{b:02x}")).collect())
        }
        "json_str" => Some(json_str(t(0))),
        "str_concat" => Some(format!("{}{}", t(0), t(1))),
        // A numeral past the doubles (`2e999`) is Infinity, which toPrecision prints as such.
        // No value where no language registered its formatting (`install_to_precision`).
        "to_precision" => t(0).trim().parse::<f64>().ok().filter(|f| !f.is_nan() && (1..=100).contains(&numbers[0])).and_then(|f| {
            if f.is_infinite() {
                Some(if f > 0.0 { "Infinity".to_owned() } else { "-Infinity".to_owned() })
            } else {
                TO_PRECISION.get().map(|to| to(f, numbers[0] as u32))
            }
        }),
        "nfkc" => {
            use unicode_normalization::UnicodeNormalization;
            Some(t(0).nfkc().collect())
        }
        _ => None,
    };
    out.map_or(Value::Null, Value::Text)
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
/// `.rr`'s `substr(s, i, n)` as SQLite's `substr(s, i + 1, n)` computes it (its `substrFunc`), over characters: a
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
pub mod substr_tests {
    /// `substr` gives what SQLite's `substr(s, i + 1, n)` does, over the corners: starts before, at and past the
    /// text, negative and zero lengths, and characters of more than one byte.
    #[test]
    fn substr_is_sqlites() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        for s in ["", "a", "a.b.c", "pkg.mod.name", "é.ü.x"] {
            for i in -7..8 {
                for n in -7..8 {
                    let want: String = db.query_row("SELECT substr(?1, ?2 + 1, ?3)", rusqlite::params![s, i, n], |r| r.get(0)).unwrap();
                    assert_eq!(super::substr(s, i, n), want, "substr({s:?}, {i}, {n})");
                }
            }
        }
    }
}
