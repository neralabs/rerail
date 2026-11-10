//! `ast.unparse` of an expression as CPython 3.13 writes it (Lib/ast.py's `_Unparser`), over `ast`'s tree: each node
//! printed at the precedence its parent gives it (`TEST` for the node unparsed: `_Unparser.get_precedence`'s
//! default), parentheses where that precedence is above the node's own; constants as their `repr` (an infinite float
//! as `1e309`); f-strings with `_Unparser`'s choice of quotes (`visit_JoinedStr`, `_str_literal_helper`), each field
//! unparsed apart at `TEST`'s next. `_Unparser`'s quirks are kept: a `BoolOp`'s operands each one level above the one
//! before, `yield`'s and `await`'s operand and both sides of `:=` at `ATOM`, an int or bool before `.` spaced.
//!
//! Besides expressions, the located nodes that are no statements: a `keyword`, an `arg`, an `alias`. A statement or a
//! pattern gives no text, nor does a tree `ast` holds a value of that `ast::F::Unknown` stands for, nor what CPython
//! raises on (an int of more than 4300 digits, an f-string no quotes can hold).

use super::ast::{Const, F, Node};
use super::value::{PyStr, bytes_repr, complex_repr, float_text, printable, str_repr, unicode_escape};

/// `_Precedence`: higher binds tighter.
type Prec = u8;
const NAMED_EXPR: Prec = 1;
const TUPLE: Prec = 2;
const YIELD: Prec = 3;
const TEST: Prec = 4;
const OR: Prec = 5;
const AND: Prec = 6;
const NOT: Prec = 7;
const CMP: Prec = 8;
const EXPR: Prec = 9;
const BOR: Prec = EXPR;
const BXOR: Prec = 10;
const BAND: Prec = 11;
const SHIFT: Prec = 12;
const ARITH: Prec = 13;
const TERM: Prec = 14;
const FACTOR: Prec = 15;
const POWER: Prec = 16;
const AWAIT: Prec = 17;
const ATOM: Prec = 18;

/// `_Precedence.next`: the next level, `ATOM` the last.
fn next(p: Prec) -> Prec {
    (p + 1).min(ATOM)
}

/// `_INFSTR`: what an infinite float is written as (`"1e" + repr(sys.float_info.max_10_exp + 1)`).
const INFSTR: &str = "1e309";

/// `_ALL_QUOTES`, in `_Unparser`'s order of preference.
const ALL_QUOTES: [&str; 4] = ["'", "\"", "\"\"\"", "'''"];

/// The classes of `ast.expr`.
pub(crate) fn is_expr(class: &str) -> bool {
    matches!(
        class,
        "BoolOp"
            | "NamedExpr"
            | "BinOp"
            | "UnaryOp"
            | "Lambda"
            | "IfExp"
            | "Dict"
            | "Set"
            | "ListComp"
            | "SetComp"
            | "DictComp"
            | "GeneratorExp"
            | "Await"
            | "Yield"
            | "YieldFrom"
            | "Compare"
            | "Call"
            | "FormattedValue"
            | "JoinedStr"
            | "TemplateStr"
            | "Constant"
            | "Attribute"
            | "Subscript"
            | "Starred"
            | "Name"
            | "List"
            | "Tuple"
            | "Slice"
    )
}

/// `ast.unparse(n)` of an expression, a `keyword`, an `arg` or an `alias`; `None` for another node, or where CPython
/// raises or the tree holds a value it does not give exactly.
pub(crate) fn unparse(n: &Node) -> Option<String> {
    let mut o = String::new();
    match n.class {
        "keyword" => keyword(n, &mut o)?,
        "arg" => arg(n, &mut o)?,
        "alias" => {
            o.push_str(ident(n, "name")?);
            if let Some(a) = opt_ident(n, "asname")? {
                o.push_str(" as ");
                o.push_str(a);
            }
        }
        c if is_expr(c) => expr(n, TEST, &mut o)?,
        _ => return None,
    }
    Some(o)
}

/// Field `name`'s node (`None` where it holds anything else).
fn req<'n>(n: &'n Node, name: &str) -> Option<&'n Node> {
    match n.get(name) {
        F::Node(c) => Some(c),
        _ => None,
    }
}

/// An optional field's node: `Some(None)` where it is `None`, `None` where it holds what is no node.
fn opt<'n>(n: &'n Node, name: &str) -> Option<Option<&'n Node>> {
    match n.get(name) {
        F::Node(c) => Some(Some(c)),
        F::Absent => Some(None),
        _ => None,
    }
}

/// A list field's items, each a node (`None` where one is not, or the field is no list).
fn nodes<'n>(n: &'n Node, name: &str) -> Option<Vec<&'n Node>> {
    match n.get(name) {
        F::List(items) => items
            .iter()
            .map(|i| match i {
                F::Node(c) => Some(&**c),
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

fn ident<'n>(n: &'n Node, name: &str) -> Option<&'n str> {
    match n.get(name) {
        F::Ident(s) => Some(s),
        _ => None,
    }
}

fn opt_ident<'n>(n: &'n Node, name: &str) -> Option<Option<&'n str>> {
    match n.get(name) {
        F::Ident(s) => Some(Some(s)),
        F::Absent => Some(None),
        _ => None,
    }
}

/// `items_view`: one item with a trailing comma, else the items joined by `, `.
fn items_view(items: &[&Node], o: &mut String) -> Option<()> {
    if let [one] = items {
        expr(one, TEST, o)?;
        o.push(',');
        return Some(());
    }
    interleave(items, TEST, o)
}

/// The items joined by `, `, each at precedence `p`.
fn interleave(items: &[&Node], p: Prec, o: &mut String) -> Option<()> {
    for (k, item) in items.iter().enumerate() {
        if k > 0 {
            o.push_str(", ");
        }
        expr(item, p, o)?;
    }
    Some(())
}

fn open(paren: bool, o: &mut String) {
    if paren {
        o.push('(');
    }
}

fn close(paren: bool, o: &mut String) {
    if paren {
        o.push(')');
    }
}

/// Expression `n` printed where its parent gives it precedence `p`.
fn expr(n: &Node, p: Prec, o: &mut String) -> Option<()> {
    match n.class {
        "BoolOp" => {
            let (op, mut prec) = match req(n, "op")?.class {
                "And" => (" and ", AND),
                _ => (" or ", OR),
            };
            let paren = p > prec;
            open(paren, o);
            for (k, v) in nodes(n, "values")?.into_iter().enumerate() {
                if k > 0 {
                    o.push_str(op);
                }
                prec = next(prec);
                expr(v, prec, o)?;
            }
            close(paren, o);
        }
        "NamedExpr" => {
            let paren = p > NAMED_EXPR;
            open(paren, o);
            expr(req(n, "target")?, ATOM, o)?;
            o.push_str(" := ");
            expr(req(n, "value")?, ATOM, o)?;
            close(paren, o);
        }
        "BinOp" => {
            let (op, prec) = match req(n, "op")?.class {
                "Add" => ("+", ARITH),
                "Sub" => ("-", ARITH),
                "Mult" => ("*", TERM),
                "MatMult" => ("@", TERM),
                "Div" => ("/", TERM),
                "Mod" => ("%", TERM),
                "LShift" => ("<<", SHIFT),
                "RShift" => (">>", SHIFT),
                "BitOr" => ("|", BOR),
                "BitXor" => ("^", BXOR),
                "BitAnd" => ("&", BAND),
                "FloorDiv" => ("//", TERM),
                "Pow" => ("**", POWER),
                _ => return None,
            };
            // `**` is right-associative.
            let (left, right) = if op == "**" { (next(prec), prec) } else { (prec, next(prec)) };
            let paren = p > prec;
            open(paren, o);
            expr(req(n, "left")?, left, o)?;
            o.push(' ');
            o.push_str(op);
            o.push(' ');
            expr(req(n, "right")?, right, o)?;
            close(paren, o);
        }
        "UnaryOp" => {
            let (op, prec) = match req(n, "op")?.class {
                "Invert" => ("~", FACTOR),
                "Not" => ("not", NOT),
                "UAdd" => ("+", FACTOR),
                "USub" => ("-", FACTOR),
                _ => return None,
            };
            let paren = p > prec;
            open(paren, o);
            o.push_str(op);
            if prec != FACTOR {
                o.push(' ');
            }
            expr(req(n, "operand")?, prec, o)?;
            close(paren, o);
        }
        "Lambda" => {
            let paren = p > TEST;
            open(paren, o);
            o.push_str("lambda");
            let mut args = String::new();
            arguments(req(n, "args")?, &mut args)?;
            if !args.is_empty() {
                o.push(' ');
                o.push_str(&args);
            }
            o.push_str(": ");
            expr(req(n, "body")?, TEST, o)?;
            close(paren, o);
        }
        "IfExp" => {
            let paren = p > TEST;
            open(paren, o);
            expr(req(n, "body")?, next(TEST), o)?;
            o.push_str(" if ");
            expr(req(n, "test")?, next(TEST), o)?;
            o.push_str(" else ");
            expr(req(n, "orelse")?, TEST, o)?;
            close(paren, o);
        }
        "Dict" => {
            let F::List(keys) = n.get("keys") else { return None };
            let values = nodes(n, "values")?;
            o.push('{');
            for (k, (key, value)) in keys.iter().zip(values).enumerate() {
                if k > 0 {
                    o.push_str(", ");
                }
                match key {
                    F::Absent => {
                        o.push_str("**");
                        expr(value, EXPR, o)?;
                    }
                    F::Node(key) => {
                        expr(key, TEST, o)?;
                        o.push_str(": ");
                        expr(value, TEST, o)?;
                    }
                    _ => return None,
                }
            }
            o.push('}');
        }
        "Set" => {
            let elts = nodes(n, "elts")?;
            if elts.is_empty() {
                // `{}` is a dict and `set` may be shadowed.
                o.push_str("{*()}");
            } else {
                o.push('{');
                interleave(&elts, TEST, o)?;
                o.push('}');
            }
        }
        "ListComp" | "SetComp" | "GeneratorExp" => {
            let (l, r) = match n.class {
                "ListComp" => ('[', ']'),
                "SetComp" => ('{', '}'),
                _ => ('(', ')'),
            };
            o.push(l);
            expr(req(n, "elt")?, TEST, o)?;
            generators(n, o)?;
            o.push(r);
        }
        "DictComp" => {
            o.push('{');
            expr(req(n, "key")?, TEST, o)?;
            o.push_str(": ");
            expr(req(n, "value")?, TEST, o)?;
            generators(n, o)?;
            o.push('}');
        }
        "Await" => {
            let paren = p > AWAIT;
            open(paren, o);
            o.push_str("await ");
            expr(req(n, "value")?, ATOM, o)?;
            close(paren, o);
        }
        "Yield" => {
            let paren = p > YIELD;
            open(paren, o);
            o.push_str("yield");
            if let Some(v) = opt(n, "value")? {
                o.push(' ');
                expr(v, ATOM, o)?;
            }
            close(paren, o);
        }
        "YieldFrom" => {
            let paren = p > YIELD;
            open(paren, o);
            o.push_str("yield from ");
            expr(req(n, "value")?, ATOM, o)?;
            close(paren, o);
        }
        "Compare" => {
            let paren = p > CMP;
            open(paren, o);
            expr(req(n, "left")?, next(CMP), o)?;
            let ops = nodes(n, "ops")?;
            let comparators = nodes(n, "comparators")?;
            for (op, c) in ops.into_iter().zip(comparators) {
                o.push_str(match op.class {
                    "Eq" => " == ",
                    "NotEq" => " != ",
                    "Lt" => " < ",
                    "LtE" => " <= ",
                    "Gt" => " > ",
                    "GtE" => " >= ",
                    "Is" => " is ",
                    "IsNot" => " is not ",
                    "In" => " in ",
                    "NotIn" => " not in ",
                    _ => return None,
                });
                expr(c, next(CMP), o)?;
            }
            close(paren, o);
        }
        "Call" => {
            expr(req(n, "func")?, ATOM, o)?;
            o.push('(');
            let args = nodes(n, "args")?;
            interleave(&args, TEST, o)?;
            for (k, kw) in nodes(n, "keywords")?.into_iter().enumerate() {
                if k > 0 || !args.is_empty() {
                    o.push_str(", ");
                }
                keyword(kw, o)?;
            }
            o.push(')');
        }
        "FormattedValue" => formatted_value(n, o)?,
        "JoinedStr" => joined(n, o)?,
        "Constant" => {
            let F::Const(c) = n.get("value") else { return None };
            if let Const::Ellipsis = c {
                o.push_str("...");
            } else {
                if opt_ident(n, "kind")? == Some("u") {
                    o.push('u');
                }
                constant(c, o)?;
            }
        }
        "Attribute" => {
            let value = req(n, "value")?;
            expr(value, ATOM, o)?;
            // `3.__abs__()` is a syntax error: an int (a bool too) is spaced from its `.`.
            if value.class == "Constant" && matches!(value.get("value"), F::Const(Const::Int(_) | Const::Bool(_))) {
                o.push(' ');
            }
            o.push('.');
            o.push_str(ident(n, "attr")?);
        }
        "Subscript" => {
            expr(req(n, "value")?, ATOM, o)?;
            o.push('[');
            let slice = req(n, "slice")?;
            match slice.class {
                // A tuple that is not empty is written without its parentheses.
                "Tuple" if !nodes(slice, "elts")?.is_empty() => items_view(&nodes(slice, "elts")?, o)?,
                _ => expr(slice, TEST, o)?,
            }
            o.push(']');
        }
        "Starred" => {
            o.push('*');
            expr(req(n, "value")?, EXPR, o)?;
        }
        "Name" => o.push_str(ident(n, "id")?),
        "List" => {
            o.push('[');
            interleave(&nodes(n, "elts")?, TEST, o)?;
            o.push(']');
        }
        "Tuple" => {
            let elts = nodes(n, "elts")?;
            let paren = elts.is_empty() || p > TUPLE;
            open(paren, o);
            items_view(&elts, o)?;
            close(paren, o);
        }
        "Slice" => {
            if let Some(lower) = opt(n, "lower")? {
                expr(lower, TEST, o)?;
            }
            o.push(':');
            if let Some(upper) = opt(n, "upper")? {
                expr(upper, TEST, o)?;
            }
            if let Some(step) = opt(n, "step")? {
                o.push(':');
                expr(step, TEST, o)?;
            }
        }
        _ => return None,
    }
    Some(())
}

/// A comprehension's `for` clauses (`visit_comprehension`).
fn generators(n: &Node, o: &mut String) -> Option<()> {
    for g in nodes(n, "generators")? {
        o.push_str(if matches!(g.get("is_async"), F::Int(1)) { " async for " } else { " for " });
        expr(req(g, "target")?, TUPLE, o)?;
        o.push_str(" in ");
        expr(req(g, "iter")?, next(TEST), o)?;
        for test in nodes(g, "ifs")? {
            o.push_str(" if ");
            expr(test, next(TEST), o)?;
        }
    }
    Some(())
}

fn keyword(n: &Node, o: &mut String) -> Option<()> {
    match opt_ident(n, "arg")? {
        None => o.push_str("**"),
        Some(a) => {
            o.push_str(a);
            o.push('=');
        }
    }
    expr(req(n, "value")?, TEST, o)
}

fn arg(n: &Node, o: &mut String) -> Option<()> {
    o.push_str(ident(n, "arg")?);
    if let Some(a) = opt(n, "annotation")? {
        o.push_str(": ");
        expr(a, TEST, o)?;
    }
    Some(())
}

/// `visit_arguments`: positional parameters with their defaults (the last ones'), `/` after the positional-only
/// ones, `*` or `*args` before keyword-only ones, `**kwargs`.
fn arguments(n: &Node, o: &mut String) -> Option<()> {
    let posonly = nodes(n, "posonlyargs")?;
    let all: Vec<&Node> = posonly.iter().copied().chain(nodes(n, "args")?).collect();
    let defaults = nodes(n, "defaults")?;
    let pad = all.len().checked_sub(defaults.len())?;
    let mut first = true;
    for (k, a) in all.iter().enumerate() {
        if !first {
            o.push_str(", ");
        }
        first = false;
        arg(a, o)?;
        if k >= pad {
            o.push('=');
            expr(defaults[k - pad], TEST, o)?;
        }
        if k + 1 == posonly.len() {
            o.push_str(", /");
        }
    }
    let vararg = opt(n, "vararg")?;
    let kwonly = nodes(n, "kwonlyargs")?;
    if vararg.is_some() || !kwonly.is_empty() {
        if !first {
            o.push_str(", ");
        }
        first = false;
        o.push('*');
        if let Some(v) = vararg {
            arg(v, o)?;
        }
    }
    let F::List(kw_defaults) = n.get("kw_defaults") else { return None };
    for (a, d) in kwonly.iter().zip(kw_defaults) {
        o.push_str(", ");
        arg(a, o)?;
        match d {
            F::Node(d) => {
                o.push('=');
                expr(d, TEST, o)?;
            }
            F::Absent => {}
            _ => return None,
        }
    }
    if let Some(k) = opt(n, "kwarg")? {
        if !first {
            o.push_str(", ");
        }
        o.push_str("**");
        arg(k, o)?;
    }
    Some(())
}

/// `_write_constant`: the value's `repr`, an infinite float or complex part as `1e309`.
fn constant(c: &Const, o: &mut String) -> Option<()> {
    match c {
        Const::None => o.push_str("None"),
        Const::Bool(true) => o.push_str("True"),
        Const::Bool(false) => o.push_str("False"),
        Const::Int(i) => o.push_str(&i.decimal()?),
        Const::Float(f) => o.push_str(&float_text(*f).replace("inf", INFSTR).replace("nan", &format!("({INFSTR}-{INFSTR})"))),
        Const::Complex(re, im) => o.push_str(&complex_repr(*re, *im).replace("inf", INFSTR).replace("nan", &format!("({INFSTR}-{INFSTR})"))),
        Const::Str(s) => o.push_str(&str_repr(s)),
        Const::Bytes(b) => o.push_str(&bytes_repr(b)),
        Const::Ellipsis => o.push_str("Ellipsis"),
    }
    Some(())
}

/// `visit_FormattedValue`: `{`, the value unparsed apart at `TEST`'s next (spaced from a `{` it starts with), its
/// conversion, its format spec, `}`.
fn formatted_value(n: &Node, o: &mut String) -> Option<()> {
    o.push('{');
    let mut inner = String::new();
    expr(req(n, "value")?, next(TEST), &mut inner)?;
    if inner.starts_with('{') {
        o.push(' ');
    }
    o.push_str(&inner);
    let F::Int(conversion) = n.get("conversion") else { return None };
    if *conversion != -1 {
        o.push('!');
        o.push(char::from_u32(u32::try_from(*conversion).ok()?)?);
    }
    if let Some(spec) = opt(n, "format_spec")? {
        o.push(':');
        fstring_inner(spec, true, o)?;
    }
    o.push('}');
    Some(())
}

/// `_write_fstring_inner`: a `JoinedStr`'s pieces, a str piece's braces doubled (in a format spec its backslashes,
/// quotes and newlines escaped too). A format spec's str piece is written as it is: one with a lone surrogate gives
/// CPython a text that is no UTF-8 (none here).
fn fstring_inner(n: &Node, spec: bool, o: &mut String) -> Option<()> {
    match n.class {
        "JoinedStr" => {
            for v in nodes(n, "values")? {
                fstring_inner(v, spec, o)?;
            }
        }
        "Constant" => {
            let F::Const(Const::Str(PyStr::Text(s))) = n.get("value") else { return None };
            let mut v = s.replace('{', "{{").replace('}', "}}");
            if spec {
                v = v.replace('\\', "\\\\").replace('\'', "\\'").replace('"', "\\\"").replace('\n', "\\n");
            }
            o.push_str(&v);
        }
        "FormattedValue" => formatted_value(n, o)?,
        _ => return None,
    }
    Some(())
}

/// A str piece of a `JoinedStr` as `_write_fstring_inner` buffers it: its braces doubled.
fn doubled(s: &PyStr) -> PyStr {
    match s {
        PyStr::Text(t) => PyStr::Text(t.replace('{', "{{").replace('}', "}}")),
        PyStr::Points(p) => PyStr::Points(p.iter().flat_map(|&c| if c == 0x7b || c == 0x7d { vec![c, c] } else { vec![c] }).collect()),
    }
}

/// `visit_JoinedStr`: `f`, then the pieces between the first quotes (of `_ALL_QUOTES`, in order) that every piece
/// allows; where a str piece allows none of those the others left, every str piece as `repr` writes it, in `'''`.
fn joined(n: &Node, o: &mut String) -> Option<()> {
    o.push('f');
    // Each piece: a str piece's text (its lone surrogates escaped once written), or another's written text.
    let mut parts: Vec<Result<PyStr, String>> = Vec::new();
    for v in nodes(n, "values")? {
        if v.class == "Constant" {
            let F::Const(Const::Str(s)) = v.get("value") else { return None };
            parts.push(Ok(doubled(s)));
        } else {
            let mut buffer = String::new();
            fstring_inner(v, false, &mut buffer)?;
            parts.push(Err(buffer));
        }
    }
    let mut quotes: Vec<&'static str> = ALL_QUOTES.to_vec();
    let mut written: Vec<String> = Vec::with_capacity(parts.len());
    let mut fallback = false;
    for part in &parts {
        match part {
            Ok(value) => {
                let (escaped, new) = str_literal(value, &quotes);
                if new.iter().all(|q| !quotes.contains(q)) {
                    fallback = true;
                    break;
                }
                quotes = new;
                written.push(escaped);
            }
            Err(value) => {
                if value.contains('\n') {
                    quotes.retain(|q| q.len() == 3);
                    // `assert quote_types` fails.
                    if quotes.is_empty() {
                        return None;
                    }
                }
                let new: Vec<&'static str> = quotes.iter().copied().filter(|q| !value.contains(q)).collect();
                if !new.is_empty() {
                    quotes = new;
                }
                written.push(value.clone());
            }
        }
    }
    if fallback {
        quotes = vec!["'''"];
        written = parts
            .iter()
            .map(|part| match part {
                Ok(value) => {
                    // `repr('"' + value)`, which quotes with `'`, less its `'"` and its last `'`.
                    let r = str_repr(&PyStr::Points(std::iter::once(u32::from('"')).chain(value.points()).collect()));
                    r[2..r.len() - 1].to_owned()
                }
                Err(value) => value.clone(),
            })
            .collect();
    }
    let quote = quotes[0];
    o.push_str(quote);
    written.iter().for_each(|w| o.push_str(w));
    o.push_str(quote);
    Some(())
}

/// `_str_literal_helper(s, quote_types=quotes, escape_special_whitespace=True)`: `s` with backslashes and the
/// characters `str.isprintable` rejects escaped (`unicode_escape`), and the quotes of `quotes` it allows (one
/// ending the text sorted last; a triple one that must end it escapes its last character); where none does, `s`'s
/// `repr` less its quotes, and the first of `quotes` holding its quote (else that quote).
fn str_literal(s: &PyStr, quotes: &[&'static str]) -> (String, Vec<&'static str>) {
    let mut escaped = String::new();
    for c in s.points() {
        if c == u32::from('\\') || !printable(c) {
            unicode_escape(c, &mut escaped);
        } else {
            escaped.push(char::from_u32(c).expect("a printable character is no surrogate"));
        }
    }
    let mut possible: Vec<&'static str> = quotes.to_vec();
    if escaped.contains('\n') {
        possible.retain(|q| q.len() == 3);
    }
    possible.retain(|q| !escaped.contains(q));
    if possible.is_empty() {
        let r = str_repr(s);
        let first = if r.starts_with('"') { '"' } else { '\'' };
        let quote = quotes.iter().copied().find(|q| q.contains(first)).unwrap_or(if first == '"' { "\"" } else { "'" });
        return (r[1..r.len() - 1].to_owned(), vec![quote]);
    }
    if let Some(last) = escaped.chars().last() {
        possible.sort_by_key(|q| q.starts_with(last));
        if possible[0].starts_with(last) {
            escaped.pop();
            escaped.push('\\');
            escaped.push(last);
        }
    }
    (escaped, possible)
}
