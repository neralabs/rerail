//! A program's version as `.rerail` text, which `rr::parse_version` reads back as the same program: for programs the
//! engine builds (a wave's rules, the converter's output) and passes on as text. Read by the version's handles
//! (0276 §6): nothing decodes it to a tree.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::sync::Arc;

use rerail_rr_syntax::syntax::quote;
use super::translate::{relation, var};
use crate::code_rule_dsl::logic::front::{Error, Native, Value, axis};
use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;
use crate::code_rule_dsl::logic::program_image::{AggRef, AtomRef, CmpOp, Lit, LitRef, RuleRef, TermRef, Version};
use crate::code_rule_dsl::logic::strings::{SET, is_test};

fn ty(t: &str) -> &str {
    match t {
        "int" => "number",
        "sym" => "symbol",
        other => other,
    }
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

/// A regex test's `.rerail` pattern: `match` is whole-string, the engine's `rx` a search. A `match("X", s)` lowers to
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

/// What the printer reads of a version beside its rules: its fields, read once.
struct Printer<'v> {
    v: &'v Version,
    nullary: BTreeSet<String>,
    kindsets: BTreeMap<String, Vec<String>>,
}

/// A text test's pattern: a string, a set of them, or a term the rule computes.
enum Pattern<'t> {
    Str(Cow<'t, str>),
    Set(Vec<Pattern<'t>>),
    Term(TermRef<'t>),
}

impl<'v> Printer<'v> {
    fn var(&self, r: RuleRef<'_>, s: crate::code_rule_dsl::logic::program_image::Slot) -> String {
        var(self.v.text(r.var(s)))
    }

    fn term(&self, r: RuleRef<'_>, t: TermRef<'_>) -> Result<String, Error> {
        Ok(match t {
            TermRef::Var(s) => self.var(r, s),
            TermRef::Wild => "_".to_owned(),
            TermRef::Int(n) => n.to_string(),
            TermRef::Str(a) => quote(self.v.text(a)),
            TermRef::Top => "top".to_owned(),
            // A lowered program's atom (decision 0270) is the store's: a lowered program is kept as its image.
            TermRef::Branded(b) => return Err(Error::Syntax(format!("the atom {b}: no .rerail form"))),
            TermRef::Call(f, _) if is_test(self.v.text(f)) || self.v.text(f) == SET => {
                return Err(Error::Syntax(format!("@{} as a value: no .rerail form", self.v.text(f))));
            }
            TermRef::Call(f, args) => {
                let f = self.v.text(f);
                format!("@{}({})", if f == "cat" { "dot" } else { f }, self.terms(r, args)?)
            }
        })
    }

    fn terms<'t>(&self, r: RuleRef<'_>, ts: impl Iterator<Item = TermRef<'t>>) -> Result<String, Error> {
        Ok(ts.map(|t| self.term(r, t)).collect::<Result<Vec<_>, _>>()?.join(", "))
    }

    /// An atom's arguments: none for a relation of no columns as written (its presence column the engine's).
    fn args(&self, r: RuleRef<'_>, a: AtomRef<'_>) -> Result<String, Error> {
        if self.nullary.contains(self.v.rel_name(a.rel())) { Ok(String::new()) } else { self.terms(r, a.args()) }
    }

    /// The pattern `lower::anchored` made `p` of, if `p` is one.
    fn unanchored<'t>(&self, p: TermRef<'t>) -> Option<TermRef<'t>> {
        let TermRef::Call(f, mut args) = p else { return None };
        if self.v.text(f) != "concat" {
            return None;
        }
        let (Some(TermRef::Str(open)), Some(TermRef::Call(g, mut inner)), None) = (args.next(), args.next(), args.next()) else { return None };
        if self.v.text(open) != "^(?:" || self.v.text(g) != "concat" {
            return None;
        }
        match (inner.next(), inner.next(), inner.next()) {
            (Some(p), Some(TermRef::Str(close)), None) if self.v.text(close) == ")$" => Some(p),
            _ => None,
        }
    }

    /// Term `t` as a text test's pattern.
    fn pattern<'t>(&self, t: TermRef<'t>) -> Pattern<'t>
    where
        'v: 't,
    {
        match t {
            TermRef::Str(a) => Pattern::Str(Cow::Borrowed(self.v.text(a))),
            TermRef::Call(f, items) if self.v.text(f) == SET => Pattern::Set(items.map(|i| self.pattern(i)).collect()),
            t => Pattern::Term(t),
        }
    }

    /// The text test `op(s, pattern)`, holding or (`neg`) failing.
    fn test(&self, r: RuleRef<'_>, op: &str, s: &str, pattern: &Pattern<'_>, neg: bool) -> Result<String, Error> {
        let bang = if neg { "!" } else { "" };
        let one = |p: &Pattern<'_>| -> Result<String, Error> {
            let p = match p {
                Pattern::Str(p) => p,
                Pattern::Set(_) => return Err(Error::Syntax("a set inside a set: no .rerail form".into())),
                Pattern::Term(t) => {
                    // A regex the rule computes: `match`, whose lowering anchored it (`lower::anchored`).
                    if op == "rx" && let Some(re) = self.unanchored(*t) {
                        return Ok(format!("{bang}match({}, {s})", self.term(r, re)?));
                    }
                    let p = self.term(r, *t)?;
                    // These internal functors search. Printing `rx(p, s)` would turn one into an
                    // undeclared relation; printing `match(p, s)` would instead require the whole string.
                    if op == "rx" || op == "irx" {
                        return Ok(format!("@{op}({s}, {p}) = {}", if neg { 0 } else { 1 }));
                    }
                    return Ok(if op == "equals" { format!("{s} {} {p}", if neg { "!=" } else { "=" }) } else { format!("{bang}{op}({p}, {s})") });
                }
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
            Pattern::Set(items) if items.len() == 1 => one(&items[0]),
            Pattern::Set(items) if neg => Ok(items.iter().map(one).collect::<Result<Vec<_>, _>>()?.join(", ")),
            Pattern::Set(items) => Ok(format!("({})", items.iter().map(one).collect::<Result<Vec<_>, _>>()?.join(" ; "))),
            p => one(p),
        }
    }

    /// An atom. `match_all("re", sp, m)` and `match_group("re", sp, m, g, gsp)` are the built-ins where the program
    /// declares no relation of that name (a `.rerail` program cannot): they print as themselves, where a relation of a
    /// reserved name gets a trailing `_` (`translate::relation`).
    fn atom(&self, r: RuleRef<'_>, a: AtomRef<'_>) -> Result<String, Error> {
        let bang = if a.neg() { "!" } else { "" };
        let rel = self.v.rel_name(a.rel());
        if let Some((set, has)) = axis(rel)
            && self.kindsets.contains_key(set)
            && let (Some(n), Some(x), None) = { let mut it = a.args(); (it.next(), it.next(), it.next()) }
        {
            let (n, x) = (self.term(r, n)?, self.term(r, x)?);
            return Ok(if has { format!("{bang}has({x}, {n}, {set})") } else { format!("{bang}inside({n}, {x}, {set})") });
        }
        let builtin = BUILTINS.contains(&rel) && !self.v.declares(rel);
        let name = if builtin { rel.to_owned() } else { relation(rel) };
        Ok(format!("{bang}{name}({})", self.args(r, a)?))
    }

    fn literal(&self, r: RuleRef<'_>, l: LitRef<'_>) -> Result<String, Error> {
        Ok(match l.view() {
            Lit::Atom(a) => self.atom(r, a)?,
            Lit::Cmp { l: TermRef::Call(f, args), op: CmpOp::Eq, r: TermRef::Int(b @ (0 | 1)) } if is_test(self.v.text(f)) && args.count() == 2 => {
                let mut args = args;
                let (s, p) = (args.next().expect("two arguments"), args.next().expect("two arguments"));
                self.test(r, self.v.text(f), &self.term(r, s)?, &self.pattern(p), b == 0)?
            }
            Lit::Cmp { l, op, r: rt } => format!("{} {} {}", self.term(r, l)?, op.as_str(), self.term(r, rt)?),
            Lit::Agg { var: v, op, over, atom: a } => {
                let over = || over.map_or_else(|| "_".to_owned(), |s| self.var(r, s));
                let op_ref = op;
                let op = match op {
                    AggRef::Count => "count".to_owned(),
                    AggRef::Min => format!("min {}", over()),
                    AggRef::Max => format!("max {}", over()),
                    AggRef::Sum => format!("sum {}", over()),
                    AggRef::Product => format!("product {}", over()),
                    AggRef::Join { by, sep } => {
                        let by: Vec<String> = by.map(|k| self.var(r, k)).collect();
                        let by = if by.is_empty() { String::new() } else { format!(" by ({})", by.join(", ")) };
                        format!("join {}{by} sep {}", over(), quote(self.v.text(sep)))
                    }
                    AggRef::Bounded { max, .. } => format!("{} {}", if max { "max" } else { "min" }, over()),
                };
                // A bounded aggregate's bound is the comparison it was written with, which lowering recognizes again
                // (decision 0511).
                let bound = match op_ref {
                    AggRef::Bounded { max, bound, strict } => {
                        format!(", {} {} {}", over(), crate::code_rule_dsl::logic::front::ast::AggOp::bound_cmp(max, strict), self.var(r, bound))
                    }
                    _ => String::new(),
                };
                format!("{} = {op} : {{ {}{bound} }}", self.var(r, v), self.atom(r, a)?)
            }
            Lit::Nearest { n, a, kindset } => format!("nearest({}, {}, {})", self.var(r, n), self.var(r, a), self.v.text(kindset)),
            Lit::Ast { document, node, matcher, captures } => {
                let mut out = format!("ast({}, {}, {}", self.var(r, document), self.var(r, node), self.v.text(matcher));
                for (name, s) in captures {
                    write!(out, ", {}: {}", self.v.text(name), self.var(r, s)).expect("a string");
                }
                out + ")"
            }
        })
    }

    /// A natively made relation's rule `r` as the construct it lowers from: `dfs_first(…)`, `dfs_first_memo(…)`, or
    /// `match_all(re, s)` over its input, the groups bound to the head's columns by name.
    fn native_rule(&self, r: RuleRef<'_>, native: &Native) -> Result<String, Error> {
        let head = r.head();
        let head_rel = self.v.rel_name(head.rel());
        let args = || self.args(r, head);
        Ok(match native {
            Native::DfsFirst { edge, target, roots } => {
                let args = args()?;
                let call = match roots {
                    Some(roots) => format!("dfs_first_memo({args}, {}, {}, {})", relation(edge), relation(target), relation(roots)),
                    None => format!("dfs_first({args}, {}, {})", relation(edge), relation(target)),
                };
                format!("{} :- {call}.", self.atom(r, head)?)
            }
            Native::PythonCandidates { input, targets, context, member } => {
                format!("{} :- python_candidates({}, {}, {}, {}, {}).", self.atom(r, head)?, args()?, relation(input), relation(targets), quote(context), quote(member))
            }
            Native::Python { kind, input, targets } => {
                let tail = targets.as_ref().map(|t| format!(", {}", relation(t))).unwrap_or_default();
                format!("{} :- python_{}({}, {}{}).", self.atom(r, head)?, kind, args()?, relation(input), tail)
            }
            Native::Components { edge, own } => {
                format!("{} :- components({}, {}, {}).", self.atom(r, head)?, args()?, relation(edge), relation(own))
            }
            // The reach index's relations are plain rules (decision 0119): printed as the closure they hold, which is
            // recognized again when read back.
            Native::Reach { seed, edge } => {
                let (rel, x, y, z) = (relation(head_rel), var("rr_x"), var("rr_y"), var("rr_z"));
                format!("{rel}({x}, {y}) :- {}({x}, {y}).\n{rel}({x}, {z}) :- {rel}({x}, {y}), {}({y}, {z}).", relation(seed), relation(edge))
            }
            Native::ReachThrough { reach, map } => {
                let (x, e, q) = (var("rr_x"), var("rr_e"), var("rr_q"));
                format!("{}({x}, {q}) :- {}({x}, {e}), {}({e}, {q}).", relation(head_rel), relation(reach), relation(map))
            }
            // The text test over its input's rows, as the rule it was made from (`preparation::text::TextRelations`).
            Native::TextTest { input, column, op, patterns } => {
                let s = match head.args().nth(*column) {
                    Some(t) => self.term(r, t)?,
                    None => var("rr_text0"),
                };
                let set = Pattern::Set(patterns.iter().map(|p| Pattern::Str(Cow::Borrowed(p.as_str()))).collect());
                format!("{} :- {}({}), {}.", self.atom(r, head)?, relation(input), args()?, self.test(r, op, &s, &set, false)?)
            }
            // A count below a bound, as the count it holds (`rank_form`): read back, it is found in the form again.
            Native::Rank { query, counted, group, op } => {
                let width = |rel: &str| self.v.columns_of(rel).map_or(0, |c| c.len());
                let (wq, wc) = (width(query), width(counted));
                let g: Vec<String> = (0..*group).map(|i| var(&format!("rr_g{i}"))).collect();
                let q: Vec<String> = (*group..wq.saturating_sub(1)).map(|i| var(&format!("rr_q{i}"))).collect();
                let c: Vec<String> = (*group..wc.saturating_sub(1)).map(|i| var(&format!("rr_c{i}"))).collect();
                let (kx, ky, n) = (var("rr_kx"), var("rr_ky"), var("rr_n"));
                let head_cols = g.iter().chain(&q).cloned().chain([n.clone()]).collect::<Vec<_>>().join(", ");
                let query_cols = g.iter().chain(&q).cloned().chain([kx.clone()]).collect::<Vec<_>>().join(", ");
                let counted_cols = g.iter().chain(&c).cloned().chain([ky.clone()]).collect::<Vec<_>>().join(", ");
                format!("{}({head_cols}) :- {}({query_cols}), {n} = count : {{ {}({counted_cols}), {ky} {op} {kx} }}.", relation(head_rel), relation(query), relation(counted))
            }
            Native::MatchAll { input, pattern, groups, positions } => {
                let s = (0..).map(|i| format!("rr_text{i}")).find(|v| !groups.contains(v)).expect("a free name");
                let coordinates: Vec<_> = (0..).map(|i| format!("rr_position{i}")).filter(|v| !groups.contains(v) && v != &s).take(if *positions { 3 } else { 0 }).map(|v| var(&v)).collect();
                let args = std::iter::once(var(&s)).chain(groups.iter().map(|g| var(g))).chain(coordinates.iter().cloned()).collect::<Vec<_>>().join(", ");
                let matched = std::iter::once(quote(pattern)).chain(std::iter::once(var(&s))).chain(coordinates).collect::<Vec<_>>().join(", ");
                // The input's other columns place each text in its file (`lower::text_matches`), left unread.
                let places = self.v.decl_of(input).map_or(1, |d| d.cols().count()).saturating_sub(1);
                let read = std::iter::once(var(&s)).chain(std::iter::repeat_n("_".to_owned(), places)).collect::<Vec<_>>().join(", ");
                format!("{}({args}) :- {}({read}), match_all({matched}).", relation(head_rel), relation(input))
            }
        })
    }
}

/// The built-in constraints that read like relations (`atom`).
const BUILTINS: [&str; 2] = ["match_all", "match_group"];

/// The program's `.rerail` text: as written, its node attributes inputs again where the lowering answered them on request
/// (decision 0174, `lower::undone_on_request`), which reading the text makes again.
pub fn print_version(v: &Arc<Version>) -> Result<String, Error> {
    if super::lower::ON_REQUEST.iter().flat_map(|round| round.iter()).any(|a| v.declares(a.req)) {
        let written = super::lower::undone_on_request(v).map_err(Error::Syntax)?;
        return print_written(&written);
    }
    print_written(v)
}

fn print_written(v: &Version) -> Result<String, Error> {
    // Relations of no columns as written keep their public schema: their presence column is the engine's.
    let p = Printer { v, nullary: ProgramFields::nullary(v), kindsets: ProgramFields::kindsets(v) };
    let mut out = String::new();
    for policy in ProgramFields::policies(v) {
        writeln!(out, ".policy {policy}").expect("a string");
    }
    for (key, text) in ProgramFields::settings(v) {
        writeln!(out, ".{key} {text}").expect("a string");
    }
    // A set that is no relation of the program (no rule reads it) is printed by its kinds; one that is stays a
    // relation, declared and stated below with the rest, once.
    for (name, kinds) in p.kindsets.iter().filter(|(name, _)| !v.declares(name)) {
        writeln!(out, ".decl {name}(kind: symbol)").expect("a string");
        writeln!(out, "{}", kinds.iter().map(|k| format!("{name}({}).", quote(k))).collect::<Vec<_>>().join(" ")).expect("a string");
    }
    let lattice = ProgramFields::lattice(v);
    let tapes = ProgramFields::tapes(v);
    // A relation's qualifiers the source states (`.decl r(…) cross_file`, decision 0652; 0099's `inline`, …), so the
    // text reads back as the version: the words a pass adds (`strategy`, a part's) are not source and stay out.
    let qualifiers = ProgramFields::qualifiers(v);
    let inputs = v.input_names();
    let mut lattices = BTreeSet::new();
    // The tree axes' relations are made again from `inside(n, a, set)` and `has(a, n, set)` (`lowering_tail::DeclareAxes`).
    let made = |rel: &str| axis(rel).is_some_and(|(set, _)| p.kindsets.contains_key(set));
    for name in v.declared_names() {
        if made(&name) {
            continue;
        }
        let cols = if p.nullary.contains(&name) { Vec::new() } else { v.columns_of(&name).unwrap_or_default() };
        let lattice = lattice.get(&name);
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
                // `core.rerail`'s `docof` over nodes and spans stays an input (`lower::projections`).
                _ if i == 0 && name == "docof" && inputs.iter().any(|r| r == "docof") => printed.push(format!("{c}: loc")),
                _ if tapes.get(&name).is_some_and(|cols| cols.contains(&i)) => printed.push(format!("{c}: tape")),
                _ => printed.push(format!("{c}: {}", ty(t))),
            }
        }
        write!(out, ".decl {}({})", relation(&name), printed.join(", ")).expect("a string");
        if let Some(k) = lattice.and_then(|l| l.widen) {
            write!(out, " widen {k}").expect("a string");
        }
        for q in qualifiers.get(&name).into_iter().flatten().filter(|q| crate::code_rule_dsl::logic::front::QUALIFIERS.contains(&q.as_str()) || *q == "extensible") {
            write!(out, " {q}").expect("a string");
        }
        out.push('\n');
    }
    // An input with parameters is printed with them, as a setting (`.input r(IO=…)`).
    let settings = ProgramFields::settings(v);
    let with_params: BTreeSet<&str> = settings.iter().filter(|(k, _)| k == "input").filter_map(|(_, t)| t.split('(').next()).collect();
    let printed_inputs: Vec<String> = inputs.iter().filter(|i| !made(i) && !with_params.contains(i.as_str())).map(|i| relation(i)).collect();
    if !printed_inputs.is_empty() {
        writeln!(out, ".input {}", printed_inputs.join(", ")).expect("a string");
    }
    let outputs = v.output_names();
    if !outputs.is_empty() {
        writeln!(out, ".output {}", outputs.iter().map(|o| relation(o)).collect::<Vec<_>>().join(", ")).expect("a string");
    }
    let demand = ProgramFields::demand(v);
    if !demand.is_empty() {
        writeln!(out, ".pragma \"magic-transform\" {}", quote(&demand.iter().map(|d| if d == "*" { d.clone() } else { relation(d) }).collect::<Vec<_>>().join(", "))).expect("a string");
    }
    for rel in v.fact_names() {
        let nullary = p.nullary.contains(&rel);
        for row in v.fact_rows(&rel) {
            let values = row.iter().filter(|_| !nullary).map(|value| match value {
                Value::Int(n) => Ok(n.to_string()),
                Value::Str(s) => Ok(quote(s)),
                Value::Branded(b) => Err(Error::Syntax(format!("the atom {b}: no .rerail form"))),
            });
            writeln!(out, "{}({}).", relation(&rel), values.collect::<Result<Vec<_>, _>>()?.join(", ")).expect("a string");
        }
    }
    let natives = ProgramFields::natives(v);
    for r in v.rules() {
        // A relation made natively is printed as what made it: its rule only orders it after its inputs.
        if let Some(native) = natives.get(v.rel_name(r.head().rel())) {
            writeln!(out, "{}", p.native_rule(r, native)?).expect("a string");
            continue;
        }
        let body = r.body().map(|l| p.literal(r, l)).collect::<Result<Vec<_>, _>>()?;
        let head = p.atom(r, r.head())?;
        if body.is_empty() {
            writeln!(out, "{head}.").expect("a string");
        } else {
            writeln!(out, "{head} :- {}.", body.join(", ")).expect("a string");
        }
    }
    Ok(out)
}
