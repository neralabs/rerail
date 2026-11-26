//! Functors (B§10 step 9, L4-f option 3): Pure answers the executor knows by name, a K-form step of kind `call`
//! naming the function, its arguments then its answer. A function reads its arguments' values (a symbol's text, read
//! from the atoms where it lies) and gives a number or a text. A text answer is the symbol of that text: found in
//! the atoms, or asked of the launch's driver, which interns the texts asked (`intern_texts`, sorted) and runs the
//! launch again (`body::emit_only`). An argument a function has no answer for (a symbol where it reads a number's
//! text, an overflow, a zero divisor) gives no row, as Soufflé's functors give none.
//!
//! The functions are `core.rerail`'s and Soufflé's, each by its meaning there, and `replace`, a text's every
//! occurrence replaced by another (a pattern's metavariables named so its language's parser reads them: 0686, L4-h).

/// What a function reads or gives: a number, or a symbol's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ty {
    Num,
    Text,
}

/// A function's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Out {
    Num(i64),
    Text(String),
}

/// A function's arguments: their count, `None` for any count of one or more.
type Args = Option<&'static [Ty]>;

use Ty::{Num, Text};

/// The functions: name, arguments (each `Text` for a variadic one), answer.
pub const FUNCS: &[(&str, Args, Ty)] = &[
    ("cat", None, Text),
    ("concat", None, Text),
    ("strlen", Some(&[Text]), Num),
    ("substr", Some(&[Text, Num, Num]), Text),
    ("to_string", Some(&[Num]), Text),
    ("to_number", Some(&[Text]), Num),
    ("lower", Some(&[Text]), Text),
    ("upper", Some(&[Text]), Text),
    ("ord", Some(&[Text]), Num),
    ("add", Some(&[Num, Num]), Num),
    ("sub", Some(&[Num, Num]), Num),
    ("mul", Some(&[Num, Num]), Num),
    ("div", Some(&[Num, Num]), Num),
    ("mod", Some(&[Num, Num]), Num),
    ("max", Some(&[Num, Num]), Num),
    ("min", Some(&[Num, Num]), Num),
    ("dot", Some(&[Text, Text]), Text),
    ("head", Some(&[Text]), Text),
    ("dir", Some(&[Text]), Text),
    ("join", Some(&[Text, Text]), Text),
    ("replace", Some(&[Text, Text, Text]), Text),
];

/// Function `name`'s place in `FUNCS`.
pub fn find(name: &str) -> Option<usize> {
    FUNCS.iter().position(|f| f.0 == name)
}

/// Whether function `f` takes `n` arguments.
pub fn takes(f: usize, n: usize) -> bool {
    match FUNCS[f].1 {
        Some(a) => a.len() == n,
        None => n >= 1,
    }
}

/// Argument `i`'s type.
pub fn arg_ty(f: usize, i: usize) -> Ty {
    FUNCS[f].1.map_or(Text, |a| a[i])
}

/// Function `f`'s answer for arguments `args` (a number, or a symbol whose text `text` reads), `None` where it has
/// none.
pub fn call<'a>(f: usize, args: &[i64], text: &dyn Fn(i64) -> Option<&'a str>) -> Option<Out> {
    let t = |i: usize| text(args[i]);
    let n = |i: usize| args[i];
    let num = Out::Num;
    let txt = |s: String| Some(Out::Text(s));
    match FUNCS[f].0 {
        "cat" | "concat" => {
            let mut s = String::new();
            for i in 0..args.len() {
                s.push_str(t(i)?);
            }
            txt(s)
        }
        "strlen" => Some(num(t(0)?.chars().count() as i64)),
        // `n` characters from the `i`-th, counted from 0, clipped to the text (SQL's `substr(s, i + 1, n)`).
        "substr" => {
            let (i, k) = (usize::try_from(n(1)).ok()?, usize::try_from(n(2)).ok()?);
            txt(t(0)?.chars().skip(i).take(k).collect())
        }
        "to_string" => txt(n(0).to_string()),
        "to_number" => t(0)?.trim().parse::<i64>().ok().map(num),
        "lower" => txt(t(0)?.to_lowercase()),
        "upper" => txt(t(0)?.to_uppercase()),
        // A symbol's ordinal: its id.
        "ord" => Some(num(n(0))),
        "add" => n(0).checked_add(n(1)).map(num),
        "sub" => n(0).checked_sub(n(1)).map(num),
        "mul" => n(0).checked_mul(n(1)).map(num),
        "div" => n(0).checked_div(n(1)).map(num),
        "mod" => n(0).checked_rem(n(1)).map(num),
        "max" => Some(num(n(0).max(n(1)))),
        "min" => Some(num(n(0).min(n(1)))),
        // `a.b`, and `b` where `a` is empty (`core.rerail`: `@dot("os", "path") = "os.path"`).
        "dot" => {
            let (a, b) = (t(0)?, t(1)?);
            txt(if a.is_empty() { b.to_string() } else { format!("{a}.{b}") })
        }
        // The first dotted segment (`@head("os.path") = "os"`).
        "head" => txt(t(0)?.split('.').next().unwrap_or("").to_string()),
        // A path's parent: "" for a path of one part (`core.rerail`: `@dir("src/a/b.ts")` = `"src/a"`).
        "dir" => txt(t(0)?.rsplit_once('/').map_or("", |(d, _)| d).to_string()),
        // `rel` from `dir`, normalised, `..` resolved (`@join(@dir("src/a/b.ts"), "../c")` = `"src/c"`).
        "join" => txt(join(t(0)?, t(1)?)),
        // Every occurrence of the text `from` (no regex: `core.rerail`'s `replace_all` is one) by `by`, left to right.
        "replace" => {
            let (s, from, by) = (t(0)?, t(1)?, t(2)?);
            txt(if from.is_empty() { s.to_string() } else { s.replace(from, by) })
        }
        other => unreachable!("a function of FUNCS: {other}"),
    }
}

/// `join`'s meaning (`core.rerail`): `rel` from `dir`, its `.` and empty parts dropped and each `..` taking one part
/// away; a `rel` from `/` is from the root.
fn join(dir: &str, rel: &str) -> String {
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
