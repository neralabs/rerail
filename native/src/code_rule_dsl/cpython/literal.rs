//! `literal_json`: what W's `pythonAst` worker gives for an expression node's `literal_json` field, CPython 3.13's
//! `json.dumps(ast.literal_eval(node), sort_keys=True, separators=(",", ":"))`, computed natively over Ruff's tree of
//! the file (`ast.parse`'s, as `syntax` reads it). No value where `literal_eval` raises (a name, a call but `set()`, an
//! f-string, `-x`, an unhashable dict key or set element) or `json.dumps` does (bytes, a set, a complex number,
//! `...`, a tuple or other non-scalar key, keys of different kinds to sort, an int of more than 4300 digits).
//!
//! `literal_eval`'s rules (Lib/ast.py): a constant is its value; a tuple, list, set or dict its elements' (a dict's
//! later equal key keeps the first key and takes the later value; `**x` is malformed); `set()` an empty set; `+n` and
//! `-n` of an int, float or complex constant (not a bool) its value; `a ± b` with `a` a signed int or float and `b` a
//! complex constant a complex number. `json.dumps`: `ensure_ascii` (every character past `~` and every control as
//! `\uXXXX`, lowercase, a surrogate pair past the BMP; `\"`, `\\`, `\b \f \n \r \t`), a float as `float.__repr__`
//! (`1e+16`, `1e-05`, `0.1`, `-0.0`; `Infinity`, `-Infinity`), a tuple as a list, a dict's items sorted by key (str
//! keys by code point, number and bool keys by value; other mixes raise) and its keys as strings (`true`, `null`,
//! `1`, `1.5`). A lone surrogate escape (`"\ud800"`), which Ruff decodes as U+FFFD, is decoded here from the source;
//! such a part that also has a `\N{…}` escape gives no value.

use super::syntax::syntax_error;
use super::value::{Int, PyStr, float_repr, join, string_part};
use ruff_python_ast::visitor::{Visitor, walk_expr};
use ruff_python_ast::{Expr, Number, UnaryOp};
use ruff_python_parser::{Mode, ParseOptions, parse_unchecked};
use ruff_text_size::Ranged;
use std::cmp::Ordering;

/// The CPython `ast` class of an expression with a value (`literal_json`'s nodes): W's worker finds a node by its span
/// and the classes its tree-sitter kind may be (`KINDS`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Class {
    Constant,
    Tuple,
    List,
    Dict,
    UnaryOp,
}

impl Class {
    /// Whether W's worker reads a node of tree-sitter kind `kind` as this class (its `KINDS`; `type` is any expression).
    pub(crate) fn of_kind(self, kind: &str) -> bool {
        match self {
            Class::Constant => matches!(kind, "string" | "concatenated_string" | "integer" | "float" | "true" | "false" | "none" | "ellipsis" | "type"),
            Class::Tuple => matches!(kind, "tuple" | "tuple_pattern" | "pattern_list" | "type"),
            Class::List => matches!(kind, "list" | "list_pattern" | "type"),
            Class::Dict => matches!(kind, "dictionary" | "type"),
            Class::UnaryOp => matches!(kind, "unary_operator" | "type"),
        }
    }
}

/// One expression's JSON: its UTF-8 byte range in the source, its class and the text.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Literal {
    pub(crate) start: u32,
    pub(crate) end: u32,
    pub(crate) class: Class,
    pub(crate) json: String,
}

/// Every expression of `source` with a `literal_json`, in source order of their visit (outer before inner). None where
/// `ast.parse` rejects the file (or raises another exception).
pub(crate) fn literals(source: &str) -> Vec<Literal> {
    if !matches!(syntax_error(source), Ok(None)) {
        return Vec::new();
    }
    let parsed = parse_unchecked(source, ParseOptions::from(Mode::Module).with_target_version(super::VERSION));
    let mut walk = Walk { source, out: Vec::new() };
    if let ruff_python_ast::Mod::Module(module) = parsed.syntax() {
        walk.visit_body(&module.body);
    }
    walk.out
}

/// The JSON text of one expression (its file's text `source`), for tests.
#[cfg(test)]
pub(crate) fn literal_json_of(source: &str) -> Option<String> {
    let found = literals(source);
    let first = found.first()?;
    let whole = source.trim_end();
    (first.start == 0 && first.end as usize == whole.len()).then(|| first.json.clone())
}

struct Walk<'s> {
    source: &'s str,
    out: Vec<Literal>,
}

impl<'a> Visitor<'a> for Walk<'_> {
    fn visit_expr(&mut self, expr: &'a Expr) {
        // Each expression once: its value, then its children's.
        let class = match expr {
            Expr::StringLiteral(_) | Expr::NumberLiteral(_) | Expr::BooleanLiteral(_) | Expr::NoneLiteral(_) => Some(Class::Constant),
            Expr::Tuple(_) => Some(Class::Tuple),
            Expr::List(_) => Some(Class::List),
            Expr::Dict(_) => Some(Class::Dict),
            Expr::UnaryOp(_) => Some(Class::UnaryOp),
            _ => None,
        };
        if let Some(class) = class
            && let Ok(value) = convert(expr, self.source)
            && let Some(json) = dumps(&value)
        {
            let range = expr.range();
            self.out.push(Literal { start: range.start().to_u32(), end: range.end().to_u32(), class, json });
        }
        walk_expr(self, expr);
    }
}

/// A value `literal_eval` makes.
#[derive(Clone, Debug)]
enum Value {
    None,
    Bool(bool),
    Int(Int),
    Float(f64),
    Complex(f64, f64),
    Str(PyStr),
    Bytes,
    Ellipsis,
    Tuple(Vec<Value>),
    List(Vec<Value>),
    Set,
    Dict(Vec<(Value, Value)>),
}

/// `literal_eval` raised.
struct Raised;

fn convert(e: &Expr, source: &str) -> Result<Value, Raised> {
    Ok(match e {
        Expr::NoneLiteral(_) => Value::None,
        Expr::BooleanLiteral(b) => Value::Bool(b.value),
        Expr::EllipsisLiteral(_) => Value::Ellipsis,
        Expr::NumberLiteral(n) => number(&n.value, &source[n.range()])?,
        Expr::StringLiteral(s) => {
            let mut parts: Vec<PyStr> = Vec::new();
            for part in s.value.as_slice() {
                parts.push(string_part(&part.value, &source[part.range()]).ok_or(Raised)?);
            }
            Value::Str(join(parts))
        }
        Expr::BytesLiteral(_) => Value::Bytes,
        Expr::Tuple(t) => Value::Tuple(t.elts.iter().map(|x| convert(x, source)).collect::<Result<_, _>>()?),
        Expr::List(l) => Value::List(l.elts.iter().map(|x| convert(x, source)).collect::<Result<_, _>>()?),
        Expr::Set(s) => {
            for x in &s.elts {
                if !hashable(&convert(x, source)?) {
                    return Err(Raised);
                }
            }
            Value::Set
        }
        Expr::Call(c) if c.arguments.args.is_empty() && c.arguments.keywords.is_empty() && matches!(&*c.func, Expr::Name(n) if n.id.as_str() == "set") => Value::Set,
        Expr::Dict(d) => {
            let mut items: Vec<(Value, Value)> = Vec::with_capacity(d.items.len());
            for item in &d.items {
                let key = convert(item.key.as_ref().ok_or(Raised)?, source)?;
                let value = convert(&item.value, source)?;
                if !hashable(&key) {
                    return Err(Raised);
                }
                match items.iter_mut().find(|(k, _)| equal(k, &key)) {
                    Some(slot) => slot.1 = value,
                    None => items.push((key, value)),
                }
            }
            Value::Dict(items)
        }
        Expr::BinOp(b) if matches!(b.op, ruff_python_ast::Operator::Add | ruff_python_ast::Operator::Sub) => {
            let left = signed(&b.left, source)?;
            let right = plain_number(&b.right, source)?;
            let sub = matches!(b.op, ruff_python_ast::Operator::Sub);
            match (&left, &right) {
                (Value::Int(_) | Value::Float(_), Value::Complex(re, im)) => {
                    // `left ± right`: the int as a float (too large: OverflowError).
                    let l = match &left {
                        Value::Int(i) => i.to_f64().ok_or(Raised)?,
                        Value::Float(f) => *f,
                        _ => unreachable!("a real number"),
                    };
                    if sub { Value::Complex(l - re, -im) } else { Value::Complex(l + re, *im) }
                }
                // `_convert_signed_num(node)` of the BinOp itself: malformed.
                _ => return Err(Raised),
            }
        }
        _ => signed(e, source)?,
    })
}

/// `_convert_signed_num`: `+n`, `-n` or `n` of a number constant.
fn signed(e: &Expr, source: &str) -> Result<Value, Raised> {
    if let Expr::UnaryOp(u) = e
        && matches!(u.op, UnaryOp::UAdd | UnaryOp::USub)
    {
        let v = plain_number(&u.operand, source)?;
        if matches!(u.op, UnaryOp::UAdd) {
            return Ok(v);
        }
        return Ok(match v {
            Value::Int(i) => Value::Int(i.neg()),
            Value::Float(f) => Value::Float(-f),
            Value::Complex(r, i) => Value::Complex(-r, -i),
            _ => unreachable!("a number"),
        });
    }
    plain_number(e, source)
}

/// `_convert_num`: an int, float or complex constant (a bool is none).
fn plain_number(e: &Expr, source: &str) -> Result<Value, Raised> {
    match e {
        Expr::NumberLiteral(n) => number(&n.value, &source[n.range()]),
        _ => Err(Raised),
    }
}

fn number(n: &Number, text: &str) -> Result<Value, Raised> {
    Ok(match n {
        Number::Int(_) => Value::Int(Int::parse(text).ok_or(Raised)?),
        Number::Float(f) => Value::Float(*f),
        Number::Complex { real, imag } => Value::Complex(*real, *imag),
    })
}

fn hashable(v: &Value) -> bool {
    match v {
        Value::List(_) | Value::Dict(_) | Value::Set => false,
        Value::Tuple(items) => items.iter().all(hashable),
        _ => true,
    }
}

/// A number's value for comparisons: bools are ints.
enum Num<'v> {
    Int(std::borrow::Cow<'v, Int>),
    Float(f64),
    Complex(f64, f64),
}

fn num(v: &Value) -> Option<Num<'_>> {
    Some(match v {
        Value::Bool(b) => Num::Int(std::borrow::Cow::Owned(Int::small(i64::from(*b)))),
        Value::Int(i) => Num::Int(std::borrow::Cow::Borrowed(i)),
        Value::Float(f) => Num::Float(*f),
        Value::Complex(r, i) => Num::Complex(*r, *i),
        _ => return None,
    })
}

/// Python's `<` order of two real numbers (`None` for NaN).
fn cmp_real(a: &Num<'_>, b: &Num<'_>) -> Option<Ordering> {
    match (a, b) {
        (Num::Int(x), Num::Int(y)) => Some(x.order(y)),
        (Num::Float(x), Num::Float(y)) => x.partial_cmp(y),
        (Num::Int(x), Num::Float(y)) => x.cmp_float(*y),
        (Num::Float(x), Num::Int(y)) => y.cmp_float(*x).map(Ordering::reverse),
        _ => None,
    }
}

/// Python's `==` of two values `literal_eval` makes.
fn equal(a: &Value, b: &Value) -> bool {
    if let (Some(x), Some(y)) = (num(a), num(b)) {
        return match (x, y) {
            (Num::Complex(r1, i1), Num::Complex(r2, i2)) => r1 == r2 && i1 == i2,
            (Num::Complex(r, i), other) | (other, Num::Complex(r, i)) => i == 0.0 && cmp_real(&Num::Float(r), &other) == Some(Ordering::Equal),
            (x, y) => cmp_real(&x, &y) == Some(Ordering::Equal),
        };
    }
    match (a, b) {
        (Value::None, Value::None) | (Value::Ellipsis, Value::Ellipsis) => true,
        (Value::Str(x), Value::Str(y)) => x.order(y) == Ordering::Equal,
        (Value::Tuple(x), Value::Tuple(y)) => x.len() == y.len() && x.iter().zip(y).all(|(a, b)| equal(a, b)),
        _ => false,
    }
}

/// `json.dumps(v, sort_keys=True, separators=(",", ":"))`, `None` where it raises.
fn dumps(v: &Value) -> Option<String> {
    let mut out = String::new();
    encode(v, &mut out)?;
    Some(out)
}

fn encode(v: &Value, out: &mut String) -> Option<()> {
    match v {
        Value::None => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Int(i) => out.push_str(&i.decimal()?),
        Value::Float(f) => out.push_str(&float_json(*f)),
        Value::Str(s) => string_json(s, out),
        Value::Tuple(items) | Value::List(items) => {
            out.push('[');
            for (k, item) in items.iter().enumerate() {
                if k > 0 {
                    out.push(',');
                }
                encode(item, out)?;
            }
            out.push(']');
        }
        Value::Dict(items) => {
            // Keys must be str, int, float, bool or None, sorted with `<` (a str and a number do not compare).
            let mut items: Vec<&(Value, Value)> = items.iter().collect();
            if items.iter().any(|(k, _)| !matches!(k, Value::None | Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::Str(_))) {
                return None;
            }
            if items.len() > 1 {
                let strs = items.iter().all(|(k, _)| matches!(k, Value::Str(_)));
                let nums = items.iter().all(|(k, _)| matches!(k, Value::Bool(_) | Value::Int(_) | Value::Float(_)));
                if strs {
                    items.sort_by(|(a, _), (b, _)| match (a, b) {
                        (Value::Str(x), Value::Str(y)) => x.order(y),
                        _ => unreachable!("str keys"),
                    });
                } else if nums {
                    let mut bad = false;
                    items.sort_by(|(a, _), (b, _)| {
                        cmp_real(&num(a).expect("a number"), &num(b).expect("a number")).unwrap_or_else(|| {
                            bad = true;
                            Ordering::Equal
                        })
                    });
                    if bad {
                        return None;
                    }
                } else {
                    return None;
                }
            }
            out.push('{');
            for (k, (key, value)) in items.iter().enumerate() {
                if k > 0 {
                    out.push(',');
                }
                match key {
                    Value::Str(s) => string_json(s, out),
                    Value::None => out.push_str("\"null\""),
                    Value::Bool(b) => out.push_str(if *b { "\"true\"" } else { "\"false\"" }),
                    Value::Int(i) => {
                        out.push('"');
                        out.push_str(&i.decimal()?);
                        out.push('"');
                    }
                    Value::Float(f) => {
                        out.push('"');
                        out.push_str(&float_json(*f));
                        out.push('"');
                    }
                    _ => unreachable!("a scalar key"),
                }
                out.push(':');
                encode(value, out)?;
            }
            out.push('}');
        }
        Value::Complex(..) | Value::Bytes | Value::Ellipsis | Value::Set => return None,
    }
    Some(())
}

/// `json`'s `ensure_ascii` string literal.
fn string_json(s: &PyStr, out: &mut String) {
    use std::fmt::Write;
    out.push('"');
    for p in s.points() {
        match p {
            0x22 => out.push_str("\\\""),
            0x5c => out.push_str("\\\\"),
            0x0a => out.push_str("\\n"),
            0x0d => out.push_str("\\r"),
            0x09 => out.push_str("\\t"),
            0x08 => out.push_str("\\b"),
            0x0c => out.push_str("\\f"),
            0x20..=0x7e => out.push(char::from(p as u8)),
            0x10000.. => {
                let v = p - 0x10000;
                let _ = write!(out, "\\u{:04x}\\u{:04x}", 0xd800 | (v >> 10), 0xdc00 | (v & 0x3ff));
            }
            _ => {
                let _ = write!(out, "\\u{p:04x}");
            }
        }
    }
    out.push('"');
}

/// `json`'s float: `float.__repr__`, `Infinity`, `-Infinity`, `NaN`.
fn float_json(f: f64) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    float_repr(f)
}

