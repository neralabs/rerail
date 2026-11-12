//! A `Program` as `.rr` text, which `rr::parse` reads back as the same program: for programs the engine builds
//! (a wave's rules, the converter's output) and passes on as text.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use rerail_rr_syntax::syntax::quote;
use super::translate::{relation, var};
use crate::code_rule_dsl::logic::front::{AggOp, Atom, Error, Literal, Native, Program, Term, Value, axis};
use crate::code_rule_dsl::logic::strings::{SET, is_test};

fn ty(t: &str) -> &str {
    match t {
        "int" => "number",
        "sym" => "symbol",
        other => other,
    }
}

fn term(t: &Term) -> Result<String, Error> {
    Ok(match t {
        Term::Var(v) => var(v),
        Term::Wild => "_".to_owned(),
        Term::Int(n) => n.to_string(),
        Term::Str(s) => quote(s),
        Term::Top => "top".to_owned(),
        Term::Call(f, _) if is_test(f) || f == SET => return Err(Error::Syntax(format!("@{f} as a value: no .rr form"))),
        Term::Call(f, args) => format!("@{}({})", if f == "cat" { "dot" } else { f }, terms(args)?),
    })
}

fn terms(ts: &[Term]) -> Result<String, Error> {
    Ok(ts.iter().map(term).collect::<Result<Vec<_>, _>>()?.join(", "))
}

/// A glob matching exactly `s`.
pub(super) fn glob_literal(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '{' | '}' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A regex test's `.rr` pattern: `match` is whole-string, the engine's `rx` a search. A `match("X", s)` lowers to
/// `rx` of `^(?:X)$` and prints back as written; another `rx` is the search `(?s:.*)(?:P)(?s:.*)`, its flag on the
/// dots only (a leading `(?s)` would give P's own `.` newlines), `irx` that under `(?i:…)`.
pub fn search(op: &str, p: &str) -> String {
    match (op, whole(p)) {
        ("rx", Some(x)) => x.to_owned(),
        ("rx", None) => format!("(?s:.*)(?:{p})(?s:.*)"),
        _ => format!("(?i:(?s:.*)(?:{p})(?s:.*))"),
    }
}

/// `X` where `p` is `^(?:X)$`, the group closing at the end.
fn whole(p: &str) -> Option<&str> {
    let inner = p.strip_prefix("^(?:")?.strip_suffix(")$")?;
    // The group `(?:` opens must close only at the last `)`: inside, every `(` closes again.
    let (mut depth, mut chars, mut class) = (0usize, inner.chars(), false);
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '[' if !class => class = true,
            ']' if class => class = false,
            '(' if !class => depth += 1,
            ')' if !class => depth = depth.checked_sub(1)?,
            _ => {}
        }
    }
    (depth == 0 && !class).then_some(inner)
}

/// The text test `op(s, pattern)`, holding or (`neg`) failing.
fn test(op: &str, s: &Term, pattern: &Term, neg: bool) -> Result<String, Error> {
    let s = term(s)?;
    let bang = if neg { "!" } else { "" };
    let one = |p: &Term| -> Result<String, Error> {
        let Term::Str(p) = p else {
            // A regex the rule computes: `match`, whose lowering anchored it (`lower::anchored`).
            if op == "rx" && let Some(re) = super::lower::unanchored(p) {
                return Ok(format!("{bang}match({}, {s})", term(re)?));
            }
            let p = term(p)?;
            // These internal functors search. Printing `rx(p, s)` would turn one into an
            // undeclared relation; printing `match(p, s)` would instead require the whole string.
            if op == "rx" || op == "irx" {
                return Ok(format!("@{op}({s}, {p}) = {}", if neg { 0 } else { 1 }));
            }
            return Ok(if op == "equals" { format!("{s} {} {p}", if neg { "!=" } else { "=" }) } else { format!("{bang}{op}({p}, {s})") });
        };
        Ok(match op {
            "equals" => format!("{s} {} {}", if neg { "!=" } else { "=" }, quote(p)),
            "iequals" => format!("{bang}iglob({}, {s})", quote(&glob_literal(p))),
            // A search (`rx` of no `^(?:X)$`) is the data-layer builtin `@rx_matches` (`functions::catalogue`).
            "rx" if whole(p).is_none() => format!("@rx_matches({s}, {}) = {}", quote(p), if neg { 0 } else { 1 }),
            "rx" | "irx" => format!("{bang}match({}, {s})", quote(&search(op, p))),
            _ => format!("{bang}{op}({}, {s})", quote(p)),
        })
    };
    match pattern {
        Term::Call(f, items) if f == SET && items.len() == 1 => one(&items[0]),
        Term::Call(f, items) if f == SET && neg => Ok(items.iter().map(one).collect::<Result<Vec<_>, _>>()?.join(", ")),
        Term::Call(f, items) if f == SET => Ok(format!("({})", items.iter().map(one).collect::<Result<Vec<_>, _>>()?.join(" ; "))),
        p => one(p),
    }
}

/// An atom. `match_all("re", sp, m)` and `match_group("re", sp, m, g, gsp)` are the built-ins where the program
/// declares no relation of that name (a `.rr` program cannot): they print as themselves, where a relation of a
/// reserved name gets a trailing `_` (`translate::relation`).
fn atom(a: &Atom, prog: &Program) -> Result<String, Error> {
    let bang = if a.neg { "!" } else { "" };
    if let Some((set, has)) = axis(&a.rel)
        && prog.kindsets.contains_key(set)
        && let [n, x] = a.args.as_slice()
    {
        return Ok(if has { format!("{bang}has({}, {}, {set})", term(x)?, term(n)?) } else { format!("{bang}inside({}, {}, {set})", term(n)?, term(x)?) });
    }
    let builtin = BUILTINS.contains(&a.rel.as_str()) && !prog.decls.contains_key(a.rel.as_str());
    let name = if builtin { a.rel.clone() } else { relation(&a.rel).into() };
    Ok(format!("{bang}{name}({})", terms(&a.args)?))
}

/// The built-in constraints that read like relations (`atom`).
const BUILTINS: [&str; 2] = ["match_all", "match_group"];

fn literal(l: &Literal, prog: &Program) -> Result<String, Error> {
    Ok(match l {
        Literal::Atom(a) => atom(a, prog)?,
        Literal::Cmp { l: Term::Call(f, args), op: "=", r: Term::Int(b @ (0 | 1)) } if is_test(f) && args.len() == 2 => test(f, &args[0], &args[1], *b == 0)?,
        Literal::Cmp { l, op, r } => format!("{} {op} {}", term(l)?, term(r)?),
        Literal::Agg { var: v, op, over, atom: a } => {
            let over = || var(over.as_deref().unwrap_or("_"));
            let op = match op {
                AggOp::Count => "count".to_owned(),
                AggOp::Min => format!("min {}", over()),
                AggOp::Max => format!("max {}", over()),
                AggOp::Sum => format!("sum {}", over()),
                AggOp::Product => format!("product {}", over()),
                AggOp::Join { by, sep } => {
                    let by: Vec<String> = by.iter().map(|k| var(k)).collect();
                    let by = if by.is_empty() { String::new() } else { format!(" by ({})", by.join(", ")) };
                    format!("join {}{by} sep {}", over(), quote(sep))
                }
            };
            format!("{} = {op} : {{ {} }}", var(v), atom(a, prog)?)
        }
        Literal::Nearest { n, a, kindset } => format!("nearest({}, {}, {kindset})", var(n), var(a)),
        Literal::Ast { document, node, matcher, captures } => {
            let mut out = format!("ast({}, {}, {matcher}", var(document), var(node));
            for (name, v) in captures {
                write!(out, ", {name}: {}", var(v)).expect("a string");
            }
            out + ")"
        }
    })
}

/// A natively made relation's rule as the construct it lowers from: `dfs_first(…)`, `dfs_first_memo(…)`, or
/// `match_all(re, s)` over its input, the groups bound to the head's columns by name.
fn native_rule(head: &Atom, native: &Native, prog: &Program) -> Result<String, Error> {
    Ok(match native {
        Native::DfsFirst { edge, target, roots } => {
            let args = terms(&head.args)?;
            let call = match roots {
                Some(roots) => format!("dfs_first_memo({args}, {}, {}, {})", relation(edge), relation(target), relation(roots)),
                None => format!("dfs_first({args}, {}, {})", relation(edge), relation(target)),
            };
            format!("{} :- {call}.", atom(head, prog)?)
        }
        Native::PythonCandidates {input,targets,context,member} => format!("{} :- python_candidates({}, {}, {}, {}, {}).",atom(head, prog)?,terms(&head.args)?,relation(input),relation(targets),quote(context),quote(member)),
        Native::Python {kind,input,targets} => {
            let tail=targets.as_ref().map(|t|format!(", {}",relation(t))).unwrap_or_default();
            format!("{} :- python_{}({}, {}{}).",atom(head, prog)?,kind,terms(&head.args)?,relation(input),tail)
        }
        Native::Components { edge, own } => {
            format!("{} :- components({}, {}, {}).", atom(head, prog)?, terms(&head.args)?, relation(edge), relation(own))
        }
        // The reach index's relations are plain rules (decision 0119): printed as the closure they hold, which is
        // recognized again when read back.
        Native::Reach { seed, edge } => {
            let (r, x, y, z) = (relation(&head.rel), var("rr_x"), var("rr_y"), var("rr_z"));
            format!("{r}({x}, {y}) :- {}({x}, {y}).\n{r}({x}, {z}) :- {r}({x}, {y}), {}({y}, {z}).", relation(seed), relation(edge))
        }
        Native::ReachThrough { reach, map } => {
            let (x, e, q) = (var("rr_x"), var("rr_e"), var("rr_q"));
            format!("{}({x}, {q}) :- {}({x}, {e}), {}({e}, {q}).", relation(&head.rel), relation(reach), relation(map))
        }
        Native::MatchAll { input, pattern, groups, positions } => {
            let s = (0..).map(|i| format!("rr_text{i}")).find(|v| !groups.contains(v)).expect("a free name");
            let coordinates: Vec<_> = (0..).map(|i| format!("rr_position{i}")).filter(|v| !groups.contains(v) && v != &s).take(if *positions { 3 } else { 0 }).map(|v| var(&v)).collect();
            let args = std::iter::once(var(&s)).chain(groups.iter().map(|g| var(g))).chain(coordinates.iter().cloned()).collect::<Vec<_>>().join(", ");
            let matched = std::iter::once(quote(pattern)).chain(std::iter::once(var(&s))).chain(coordinates).collect::<Vec<_>>().join(", ");
            format!("{}({args}) :- {}({}), match_all({matched}).", relation(&head.rel), relation(input), var(&s))
        }
    })
}

/// The program's `.rr` text.
pub fn print(prog: &Program) -> Result<String, Error> {
    // Keep the public schema when printing frontend unit columns back to rule source.
    let mut logical;
    let prog = if prog.nullary.is_empty() { prog } else {
        logical=prog.clone();
        for name in &prog.nullary {
            if let Some(cols)=logical.decls.get_mut(name){cols.clear();}
            for row in logical.facts.get_mut(name).into_iter().flatten(){row.clear();}
        }
        let decode=|a:&mut Atom|{if prog.nullary.contains(a.rel.as_str()){a.args.clear();}};
        for rule in &mut logical.rules {
            decode(&mut rule.head);
            for lit in &mut rule.body{if let Literal::Atom(a)|Literal::Agg{atom:a,..}=lit{decode(a);}}
        }
        logical.nullary.clear();&logical
    };
    let mut out = String::new();
    for p in &prog.policies {
        writeln!(out, ".policy {p}").expect("a string");
    }
    for (key, text) in &prog.settings {
        writeln!(out, ".{key} {text}").expect("a string");
    }
    for (name, kinds) in &prog.kindsets {
        writeln!(out, ".decl {name}(kind: symbol)").expect("a string");
        writeln!(out, "{}", kinds.iter().map(|k| format!("{name}({}).", quote(k))).collect::<Vec<_>>().join(" ")).expect("a string");
    }
    let mut lattices = BTreeSet::new();
    // The tree axes' relations are made again from `inside(n, a, set)` and `has(a, n, set)` (`front::finish`).
    let made = |rel: &str| axis(rel).is_some_and(|(set, _)| prog.kindsets.contains_key(set));
    for (name, cols) in &prog.decls {
        if made(name) {
            continue;
        }
        let lattice = prog.lattice.get(name);
        let mut printed = Vec::new();
        for (i, (c, t)) in cols.iter().enumerate() {
            match lattice {
                Some(l) if i == l.keys => {
                    let column = match &l.merge {
                        Some(f) => {
                            if lattices.insert(f.clone()) {
                                writeln!(out, ".lattice {f}<{}> {{ Lub -> @{f} }}", ty(&l.ty)).expect("a string");
                            }
                            format!("{f}<>")
                        }
                        None => format!("{}<>", l.ty),
                    };
                    printed.push(format!("{c}: {column}"));
                }
                // `core.rr`'s `docof` over nodes and spans stays an input (`lower::projections`).
                _ if i == 0 && name == "docof" && prog.inputs.iter().any(|r| r == "docof") => printed.push(format!("{c}: loc")),
                _ if prog.tapes.get(name).is_some_and(|cols| cols.contains(&i)) => printed.push(format!("{c}: tape")),
                _ => printed.push(format!("{c}: {}", ty(t))),
            }
        }
        write!(out, ".decl {}({})", relation(name), printed.join(", ")).expect("a string");
        if let Some(k) = lattice.and_then(|l| l.widen) {
            write!(out, " widen {k}").expect("a string");
        }
        out.push('\n');
    }
    // An input with parameters is printed with them, as a setting (`.input r(IO=…)`).
    let with_params: BTreeSet<&str> = prog.settings.iter().filter(|(k, _)| k == "input").filter_map(|(_, t)| t.split('(').next()).collect();
    let inputs: Vec<String> = prog.inputs.iter().filter(|i| !made(i) && !with_params.contains(i.as_str())).map(|i| relation(i)).collect();
    if !inputs.is_empty() {
        writeln!(out, ".input {}", inputs.join(", ")).expect("a string");
    }
    if !prog.outputs.is_empty() {
        writeln!(out, ".output {}", prog.outputs.iter().map(|o| relation(o)).collect::<Vec<_>>().join(", ")).expect("a string");
    }
    if !prog.demand.is_empty() {
        writeln!(out, ".pragma \"magic-transform\" {}", quote(&prog.demand.iter().map(|d| if d == "*" { d.clone() } else { relation(d) }).collect::<Vec<_>>().join(", "))).expect("a string");
    }
    for (rel, rows) in &prog.facts {
        for row in rows {
            let values = row.iter().map(|v| match v {
                Value::Int(n) => n.to_string(),
                Value::Str(s) => quote(s),
            });
            writeln!(out, "{}({}).", relation(rel), values.collect::<Vec<_>>().join(", ")).expect("a string");
        }
    }
    for rule in &prog.rules {
        // A relation made natively is printed as what made it: its rule only orders it after its inputs.
        if let Some(native) = prog.natives.get(rule.head.rel.as_str()) {
            writeln!(out, "{}", native_rule(&rule.head, native, prog)?).expect("a string");
            continue;
        }
        let body = rule.body.iter().map(|l| literal(l, prog)).collect::<Result<Vec<_>, _>>()?;
        let head = atom(&rule.head, prog)?;
        if body.is_empty() {
            writeln!(out, "{head}.").expect("a string");
        } else {
            writeln!(out, "{head} :- {}.", body.join(", ")).expect("a string");
        }
    }
    Ok(out)
}
