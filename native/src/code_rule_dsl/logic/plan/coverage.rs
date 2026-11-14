//! The executor coverage gate: every operation a rule can write, run on every executor, so a missing step shows
//! here, named, and never first as a slow benchmark.
//!
//! A rule's literal runs on one of three executors: SQLite (the fallback, which has every operation), the native
//! set engine, and the Metal device. A stratum the native engine has no step for falls back to SQLite whole, and one
//! the device has no form for stays on the CPU. Neither fails: each shows only as time. So this gate enumerates the
//! operations from the language's own tables, not by hand:
//!
//! - every comparison operator, over every pair of operand kinds (numbers, symbols, constants, computed values);
//! - every function `strings::functions` registers, every functor `plan::sql_term` computes, `.rr`'s arithmetic
//!   and Soufflé's built-ins;
//! - every text test (`.rr`'s names, `rr::lower::TESTS`), with a constant, a set and a bound pattern;
//! - every aggregate (`front::AggOp`, matched exhaustively: a new one does not compile until it has a probe);
//! - every literal form (`front::Literal`, matched exhaustively), negation, recursion and computed heads.
//!
//! Each probe is a one-rule program over a few facts. It runs on SQLite, on the native engine and on the device
//! (placed there wherever it has every form, `placement::Binding::Always`), and the native engine's rows must be
//! SQLite's. Where each one ran, or why not, is the ledger `gate/executor-coverage/matrix.tsv`: a probe that newly
//! falls back fails the test, and so does one that newly runs where the ledger says it does not, until the ledger is
//! written again (`RERAIL_BLESS_COVERAGE=1`). The ledger is the list of work the executors lack.
//!
//! The census (`every_operation_a_shipped_rule_uses_has_a_probe`) closes the loop from the other side: every
//! function, comparison, aggregate and text test any shipped `.rr` file uses must be one the matrix probes.

use super::*;
use crate::content_store::ContentStore;
use crate::code_rule_dsl::logic::front::AggOp;
use crate::code_rule_dsl::logic::rr::parse;
use crate::code_rule_dsl::logic::strings::functions::{Kind, all as functions};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The facts every probe reads: symbols, numbers, pairs and edges (with a cycle).
const FACTS: &str = r#".decl s(a: symbol)
s("alpha"). s("Beta"). s("a.b"). s("x_1"). s("12").
.decl t(a: symbol)
t("alpha"). t("a").
.decl n(x: number)
n(1). n(2). n(5). n(-3). n(0).
.decl m(x: number)
m(2). m(0).
.decl p(a: symbol, x: number)
p("alpha", 1). p("alpha", 4). p("Beta", 2). p("a.b", -1).
.decl q(a: symbol, b: symbol)
q("alpha", "z"). q("alpha", "y"). q("Beta", "w").
.decl e(a: number, b: number)
e(1, 2). e(2, 5). e(5, 1). e(0, -3).
.lattice shortlex_min<symbol> { Lub -> @shortlex_min }
"#;

/// One probe: what it checks (the ledger's key), the operations it uses (the census's names), and its rule, whose
/// head is `{HEAD}` (the probe's own relation, declared from `cols`).
struct Probe {
    name: String,
    ops: Vec<String>,
    cols: &'static str,
    rule: String,
}

fn probe(name: impl Into<String>, ops: &[&str], cols: &'static str, rule: impl Into<String>) -> Probe {
    Probe { name: name.into(), ops: ops.iter().map(|s| (*s).to_owned()).collect(), cols, rule: rule.into() }
}

/// The comparison operators `.rr` writes.
const COMPARISONS: [&str; 6] = ["=", "!=", "<", "<=", ">", ">="];

/// `.rr`'s arithmetic, and the functor each lowers to.
const ARITHMETIC: [(&str, &str); 5] = [("+", "add"), ("-", "sub"), ("*", "mul"), ("/", "div"), ("%", "mod")];

/// Soufflé's built-in (non-`@`) functions `rr::lower` takes, by `.rr` name, with each argument's kind and the
/// functor it lowers to.
const BUILTINS: [(&str, &[Kind], &str); 7] = [
    ("strlen", &[Kind::Text], "strlen"),
    ("to_string", &[Kind::Number], "to_string"),
    ("to_number", &[Kind::Text], "to_number"),
    ("substr", &[Kind::Text, Kind::Number, Kind::Number], "substr"),
    ("max", &[Kind::Number, Kind::Number], "max"),
    ("min", &[Kind::Number, Kind::Number], "min"),
    ("cat", &[Kind::Text, Kind::Text], "concat"),
];

/// The functors `plan::sql_term` computes by an arm of its own (beyond `strings::functions`' registry), each with its
/// arguments' kinds. `doc`, `in` and `place*` read nodes and atoms, which these facts have none of; they are probed
/// by the comparisons that use them where they are written (`@doc(x) = @doc(y)`).
const SQL_FUNCTORS: [(&str, &[Kind]); 9] = [
    ("dot", &[Kind::Text, Kind::Text]),
    ("concat", &[Kind::Text, Kind::Text]),
    ("replace_all", &[Kind::Text, Kind::Text, Kind::Text]),
    ("json_str", &[Kind::Text]),
    ("nfkc", &[Kind::Text]),
    ("py_repr", &[Kind::Text]),
    ("ed25519_ok", &[Kind::Text, Kind::Text, Kind::Text]),
    ("rx_replace", &[Kind::Text, Kind::Text, Kind::Text]),
    ("rx_group", &[Kind::Text, Kind::Text, Kind::Text]),
];

/// An aggregate's probe, one per `AggOp`: the match has no wildcard, so a new aggregate needs its probe to compile.
fn aggregate(op: &AggOp) -> Probe {
    match op {
        AggOp::Count => probe("aggregate count", &["agg count"], "a: symbol, c: number", "{HEAD}(a, c) :- s(a), c = count : { p(a, _) }."),
        AggOp::Sum => probe("aggregate sum", &["agg sum"], "a: symbol, c: number", "{HEAD}(a, c) :- s(a), c = sum v : { p(a, v) }."),
        AggOp::Min => probe("aggregate min", &["agg min"], "a: symbol, c: number", "{HEAD}(a, c) :- t(a), c = min v : { p(a, v) }."),
        AggOp::Max => probe("aggregate max", &["agg max"], "a: symbol, c: number", "{HEAD}(a, c) :- t(a), c = max v : { p(a, v) }."),
        AggOp::Product => probe("aggregate product", &["agg product"], "a: symbol, c: number", "{HEAD}(a, c) :- s(a), c = product v : { p(a, v) }."),
        AggOp::Join { .. } => probe("aggregate join", &["agg join"], "a: symbol, c: symbol", "{HEAD}(a, c) :- t(a), c = join b by b sep \", \" : { q(a, b) }."),
    }
}

/// A literal form's probes, one or more per `Literal` variant: the match has no wildcard, so a new form needs its
/// probe to compile. `nearest` and `ast` read the syntax tree, which these facts have none of: their probes run in
/// `tree_forms_run_on_every_executor`.
fn form(l: &Literal) -> Vec<Probe> {
    match l {
        Literal::Atom(_) => vec![
            probe("join", &["atom"], "x: number, y: number", "{HEAD}(x, y) :- e(x, y), n(y)."),
            probe("join of three", &["atom"], "x: number, z: number", "{HEAD}(x, z) :- e(x, y), e(y, z), n(z)."),
            probe("negation", &["atom", "negation"], "x: number", "{HEAD}(x) :- n(x), !m(x)."),
            probe("negation of a pair", &["atom", "negation"], "x: number, y: number", "{HEAD}(x, y) :- n(x), n(y), !e(x, y)."),
            probe("semi-join (a column unread)", &["atom"], "x: number", "{HEAD}(x) :- n(x), e(x, _)."),
            probe("constant in an atom", &["atom"], "y: number", "{HEAD}(y) :- e(2, y)."),
            probe("constant in a head", &["atom"], "x: number, k: number", "{HEAD}(x, 7) :- n(x)."),
            probe("recursion (closure)", &["atom", "recursion"], "x: number, y: number", "{HEAD}(x, y) :- e(x, y).\n{HEAD}(x, z) :- {HEAD}(x, y), e(y, z)."),
            probe("recursion with a negation", &["atom", "recursion", "negation"], "x: number, y: number", "{HEAD}(x, y) :- e(x, y), !m(y).\n{HEAD}(x, z) :- {HEAD}(x, y), e(y, z), !m(z)."),
            probe("recursion with a comparison", &["atom", "recursion", "cmp <"], "x: number, y: number", "{HEAD}(x, y) :- e(x, y).\n{HEAD}(x, z) :- {HEAD}(x, y), e(y, z), z < 5."),
            probe("recursion with a computed term", &["atom", "recursion", "call add"], "x: number, d: number", "{HEAD}(x, 0) :- n(x).\n{HEAD}(y, d + 1) :- {HEAD}(x, d), e(x, y), d < 4."),
            probe("computed head", &["atom", "call add"], "x: number", "{HEAD}(x + 1) :- n(x)."),
            probe("computed symbol head", &["atom", "call cat"], "a: symbol", "{HEAD}(@dot(a, \"x\")) :- s(a)."),
        ],
        Literal::Cmp { .. } => Vec::new(),
        Literal::Agg { .. } => Vec::new(),
        Literal::Nearest { .. } => Vec::new(),
        Literal::Ast { .. } => Vec::new(),
    }
}

/// Every probe: the comparisons by operand kinds, the arithmetic, every function and functor, the text tests, the
/// aggregates and the literal forms.
fn probes() -> Vec<Probe> {
    let mut out = Vec::new();
    for op in COMPARISONS {
        let ops = [&format!("cmp {op}")[..]];
        out.push(probe(format!("cmp {op}: numbers"), &ops, "x: number, y: number", format!("{{HEAD}}(x, y) :- n(x), n(y), x {op} y.")));
        out.push(probe(format!("cmp {op}: number and constant"), &ops, "x: number", format!("{{HEAD}}(x) :- n(x), x {op} 2.")));
        out.push(probe(format!("cmp {op}: symbols"), &ops, "a: symbol, b: symbol", format!("{{HEAD}}(a, b) :- s(a), t(b), a {op} b.")));
        out.push(probe(format!("cmp {op}: symbol and constant"), &ops, "a: symbol", format!("{{HEAD}}(a) :- s(a), a {op} \"alpha\".")));
        out.push(probe(format!("cmp {op}: computed number"), &[ops[0], "call add"], "x: number", format!("{{HEAD}}(x) :- n(x), x + 1 {op} 3.")));
        out.push(probe(format!("cmp {op}: computed symbol"), &[ops[0], "call concat"], "a: symbol", format!("{{HEAD}}(a) :- s(a), cat(a, \"!\") {op} \"alpha!\".")));
    }
    for (op, f) in ARITHMETIC {
        out.push(probe(format!("arithmetic {op}"), &[&format!("call {f}")], "x: number, y: number, z: number", format!("{{HEAD}}(x, y, z) :- n(x), n(y), z = x {op} y.")));
    }
    out.push(probe("arithmetic of floats", &["call float_add", "call to_float"], "z: number", "{HEAD}(z) :- s(a), z = to_float(\"1.5\") + 2.25, a = \"alpha\"."));
    let arg = |k: &Kind, i: usize| match k {
        Kind::Text => ["a", "b", "c"][i % 3],
        Kind::Number => ["x", "y", "w"][i % 3],
    };
    let binds = |kinds: &[Kind]| {
        let mut b: Vec<String> = Vec::new();
        for (i, k) in kinds.iter().enumerate() {
            b.push(match k {
                Kind::Text => format!("t({})", arg(k, i)),
                Kind::Number => format!("m({})", arg(k, i)),
            });
        }
        b.join(", ")
    };
    let value = |k: Kind| if k == Kind::Text { "v: symbol" } else { "v: number" };
    for (name, kinds, rr) in BUILTINS {
        let args: Vec<&str> = kinds.iter().enumerate().map(|(i, k)| arg(k, i)).collect();
        let ret = if matches!(name, "strlen" | "to_number" | "max" | "min") { Kind::Number } else { Kind::Text };
        out.push(probe(format!("builtin {name}/{}", kinds.len()), &[&format!("call {rr}")], value(ret), format!("{{HEAD}}(v) :- {}, v = {name}({}).", binds(kinds), args.join(", "))));
    }
    for (name, kinds) in SQL_FUNCTORS {
        let args: Vec<&str> = kinds.iter().enumerate().map(|(i, k)| arg(k, i)).collect();
        let ret = if name == "ed25519_ok" { Kind::Number } else { Kind::Text };
        let op = if name == "dot" { "cat" } else { name };
        out.push(probe(format!("functor @{name}/{}", kinds.len()), &[&format!("call {op}")], value(ret), format!("{{HEAD}}(v) :- {}, v = @{name}({}).", binds(kinds), args.join(", "))));
    }
    for &(name, kinds, ret) in functions() {
        // `rr_*` are the native engine's names of `.rr`'s own arithmetic and built-ins, probed above by their spellings.
        if name.starts_with("rr_") {
            continue;
        }
        let args: Vec<&str> = kinds.iter().enumerate().map(|(i, k)| arg(k, i)).collect();
        out.push(probe(format!("function @{name}/{}", kinds.len()), &[&format!("call {name}")], value(ret), format!("{{HEAD}}(v) :- {}, v = @{name}({}).", binds(kinds), args.join(", "))));
    }
    // Every other functor a shipped rule calls, by the arity it is called with: typed by its `.functor` declaration
    // where it has one (else its arguments and value are symbols). So a rule that calls a new function is probed
    // without anyone writing its probe.
    let probed: BTreeSet<String> = out.iter().flat_map(|p| p.ops.iter().cloned()).collect();
    let decls = functor_decls();
    let tree = ["doc", "in", "place", "place_path", "place_line", "flat_bin", "id", "line", "depth", "within", "tree_digest"];
    for op in census().0.keys() {
        let Some((name, arity)) = op.strip_prefix("arity ").and_then(|r| r.split_once(' ')) else { continue };
        let Ok(arity) = arity.parse::<usize>() else { continue };
        if probed.contains(&format!("call {name}")) || tree.contains(&name) || out.iter().any(|p| p.name == format!("shipped functor @{name}/{arity}")) {
            continue;
        }
        // A data-layer builtin (`functions::catalogue`) is typed as the engine function it is.
        use crate::code_rule_dsl::logic::strings::functions as fns;
        let builtin = fns::catalogue(name, arity).and_then(|(f, _)| fns::signature(f)).filter(|(_, k, _)| k.len() == arity);
        let (kinds, ret) = match (builtin, decls.get(name)) {
            (Some((_, k, r)), _) => (k.to_vec(), r),
            (None, Some((k, r))) if k.len() == arity => (k.clone(), *r),
            _ => (vec![Kind::Text; arity], Kind::Text),
        };
        let args: Vec<&str> = kinds.iter().enumerate().map(|(i, k)| arg(k, i)).collect();
        out.push(probe(format!("shipped functor @{name}/{arity}"), &[&format!("call {name}")], value(ret), format!("{{HEAD}}(v) :- {}, v = @{name}({}).", binds(&kinds), args.join(", "))));
    }
    for (rr, engine) in crate::code_rule_dsl::logic::rr::lower::TESTS {
        let ops = [&format!("test {engine}")[..]];
        out.push(probe(format!("text test {rr}: a constant pattern"), &ops, "a: symbol", format!("{{HEAD}}(a) :- s(a), {rr}(\"a\", a).")));
        out.push(probe(format!("text test {rr}: negated"), &ops, "a: symbol", format!("{{HEAD}}(a) :- s(a), !{rr}(\"a\", a).")));
        out.push(probe(format!("text test {rr}: a set of patterns"), &ops, "a: symbol", format!("{{HEAD}}(a) :- s(a), ({rr}(\"a\", a) ; {rr}(\"B\", a)).")));
        out.push(probe(format!("text test {rr}: a bound pattern"), &ops, "a: symbol, b: symbol", format!("{{HEAD}}(a, b) :- s(a), t(b), {rr}(b, a).")));
    }
    for op in [AggOp::Count, AggOp::Sum, AggOp::Min, AggOp::Max, AggOp::Product, AggOp::Join { by: Vec::new(), sep: String::new() }] {
        out.push(aggregate(&op));
    }
    // `@concat` of more than two texts, as the data-layer programs write it: their concatenation, left to right.
    out.push(probe("functor @concat of three texts", &["call concat"], "a: symbol", "{HEAD}(v) :- t(a), t(b), v = @concat(a, \"::\", b)."));
    // `@printf` of a constant format, as the data-layer programs write it: `%d`, `%0Nd` (never truncated) and `%s`.
    out.push(probe("functor @printf of %012d", &["call printf"], "a: symbol", "{HEAD}(v) :- n(x), v = @printf(\"%03d.\", x)."));
    out.push(probe("functor @printf of %d and %s", &["call printf"], "a: symbol", "{HEAD}(v) :- p(a, x), v = @printf(\"%d:%s|%%\", x, a)."));
    out.push(probe("membership of a constant set", &["test equals"], "a: symbol", "{HEAD}(a) :- s(a), (a = \"alpha\" ; a = \"x_1\")."));
    out.push(probe("lattice relation (shortest)", &["lattice"], "a: number, d: min<>| widen 8", "{HEAD}(x, 0) :- n(x), x = 1.\n{HEAD}(y, d + 1) :- {HEAD}(x, d), e(x, y)."));
    // A lattice relation holds one row a key: a key's rows fold by its join, and its readers see the folded value only.
    out.push(probe("lattice relation (flat: a key's values differ)", &["lattice"], "a: symbol, v: flat<>", "{HEAD}(a, x) :- p(a, x)."));
    out.push(probe("lattice relation (max over a key)", &["lattice"], "a: symbol, v: max<>| widen 8", "{HEAD}(a, x) :- p(a, x)."));
    out.push(probe("lattice relation (shortlex_min over symbols)", &["lattice"], "a: number, r: shortlex_min<>| widen 8",
        "{HEAD}(x, \"\") :- n(x), x = 1.\n{HEAD}(y, r) :- {HEAD}(x, r0), e(x, y), r = cat(r0, \"z\")."));
    out.push(probe("lattice relation (a key improved after its first value)", &["lattice"], "a: number, d: min<>| widen 8",
        "{HEAD}(x, 0) :- n(x), x = 1.\n{HEAD}(x, 10) :- n(x), x = 5.\n{HEAD}(y, d + 1) :- {HEAD}(x, d), e(x, y)."));
    out.push(probe("lattice relation (max over a cycle, widened to its top)", &["lattice"], "a: number, d: max<>| widen 2",
        "{HEAD}(x, 0) :- n(x), x = 1.\n{HEAD}(y, d + 1) :- {HEAD}(x, d), e(x, y)."));
    out.push(probe("lattice relation read by a later stratum", &["lattice"], "a: number, d: number",
        ".decl {HEAD}_l(a: number, d: min<>) widen 8\n{HEAD}_l(x, 0) :- n(x), x = 1.\n{HEAD}_l(y, d + 1) :- {HEAD}_l(x, d), e(x, y).\n{HEAD}(a, d) :- {HEAD}_l(a, d)."));
    out.push(probe("aggregate count = 0 (a negation)", &["agg count"], "a: symbol", "{HEAD}(a) :- s(a), count : { p(a, _) } = 0."));
    out.push(probe("aggregate over two atoms' bag", &["agg count"], "a: symbol, c: number", "{HEAD}(a, c) :- t(a), c = count : { q(a, _) }."));
    let a = Atom { rel: "n".into(), args: Vec::new(), neg: false, gated: false };
    let t = Term::Wild;
    for l in [
        Literal::Atom(a.clone()),
        Literal::Cmp { l: t.clone(), op: "=", r: t },
        Literal::Agg { var: String::new(), op: AggOp::Count, over: None, atom: a },
        Literal::Nearest { n: String::new(), a: String::new(), kindset: String::new() },
        Literal::Ast { document: String::new(), node: String::new(), matcher: String::new(), captures: Vec::new() },
    ] {
        out.extend(form(&l));
    }
    out
}

/// Where one probe ran on each executor.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Verdict {
    native: String,
    device: String,
}

fn fresh() -> (PathBuf, ContentStore, Connection) {
    let path = crate::code_rule_dsl::fixtures::temp();
    let store = ContentStore::open(&path).unwrap();
    let db = Connection::open(&path).unwrap();
    super::super::native::register(&db).unwrap();
    super::super::native::register_symbols(&db).unwrap();
    (path, store, db)
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

/// Runs `src` (whose probe relation is `rel`) on SQLite, the native engine and the device; the verdict, or why the
/// probe did not run at all.
fn run_probe(src: &str, rel: &str) -> std::result::Result<Verdict, String> {
    let p = parse(src).map_err(|e| format!("unparsed: {e}"))?;
    // A run's rows of `rel`, whether it ran natively, and where its strata ran.
    type ProbeRun = (Vec<String>, bool, Vec<(String, placement::Ran)>);
    let lower_run = |modes: &str, binding: placement::Binding| -> std::result::Result<ProbeRun, String> {
        let (_path, store, db) = fresh();
        // `shortlex_min<>` orders the run's symbols by their text: the store's atoms, as a run declares it.
        crate::code_rule_dsl::logic::native::declare("shortlex_min", std::sync::Arc::new(crate::code_rule_dsl::logic::native::ShortlexMin(store.atoms_handle())));
        let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
        let lowered = lower::lower(&p, python, &mut |s| store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string()))).map_err(|e| format!("unlowered: {e}"))?;
        let was = placement::set_binding(binding);
        let got = run_atoms_native_over(&lowered, &db, "main", Stats::empty(&lowered.program), BTreeSet::new(), modes);
        placement::set_binding(was);
        let got = got.map_err(|e| format!("error: {e}"))?;
        let ran = placement::recorded(rel);
        Ok((sorted(got.outputs.get(rel).cloned().unwrap_or_default()), got.native.iter().any(|k| k.split('/').any(|r| r == rel)), ran))
    };
    let (want, _, _) = lower_run("", placement::Binding::Cpu).map_err(|e| format!("sqlite: {e}"))?;
    let native = match lower_run("program", placement::Binding::Cpu) {
        Err(e) => e,
        Ok((got, _, _)) if got != want => format!("WRONG ROWS: native {got:?}, sqlite {want:?}"),
        Ok((_, true, _)) => "native".to_owned(),
        Ok((_, false, ran)) => ran.iter().find(|(_, r)| r.why.starts_with("sqlite")).map_or_else(|| "sqlite (no reason recorded)".to_owned(), |(_, r)| r.why.clone()),
    };
    let device = if !metal() {
        "no Metal device".to_owned()
    } else {
        match lower_run("program", placement::Binding::Always) {
            Err(e) => e,
            Ok((got, _, _)) if got != want => format!("WRONG ROWS: device {got:?}, sqlite {want:?}"),
            Ok((_, _, ran)) => match ran.iter().find(|(_, r)| !r.why.starts_with("sqlite")) {
                Some((_, r)) => r.why.clone(),
                None if native != "native" => "not native".to_owned(),
                None => "not placed".to_owned(),
            },
        }
    };
    Ok(Verdict { native, device })
}

fn metal() -> bool {
    #[cfg(target_os = "macos")]
    return crate::code_rule_dsl::transducer::packed::metal_join::available();
    #[cfg(not(target_os = "macos"))]
    false
}

/// The second ledger, `packs.tsv`: each gap the matrix records (a probe not native, or not on the device), by how
/// many shipped programs of each pack use its operations: which gaps the packs meet, and how often. Written and
/// compared with the matrix (`RERAIL_BLESS_COVERAGE=1`).
fn packs_ledger(all: &[Probe], matrix: &str) {
    let (used, _, _) = census();
    // A file's pack: the directory under `packs/`, else the tree it is in (`data-layer`, `lib`, the engine's own).
    let pack = |f: &str| -> String {
        if let Some((_, rest)) = f.split_once("/rule-syntax/packs/") {
            return rest.split('/').next().unwrap_or("?").to_owned();
        }
        if let Some((_, rest)) = f.split_once("/rule-syntax/") {
            return rest.split('/').next().filter(|d| !d.ends_with(".rr")).unwrap_or("rule-syntax").to_owned();
        }
        if f.contains("/programs/data_layer/") { "data-layer (engine)".into() } else { "engine".into() }
    };
    let cells: BTreeMap<&str, (&str, &str)> = matrix.lines().filter(|l| !l.starts_with('#')).filter_map(|l| {
        let mut c = l.split('\t');
        Some((c.next()?, (c.next()?, c.next()?)))
    }).collect();
    let mut rows: Vec<(usize, String)> = Vec::new();
    for pr in all {
        let Some(&(native, device)) = cells.get(pr.name.as_str()) else { continue };
        if native == "native" && device == "device" {
            continue;
        }
        let files: BTreeSet<&String> = pr.ops.iter().filter_map(|op| used.get(op)).flatten().collect();
        if files.is_empty() {
            continue;
        }
        let mut by: BTreeMap<String, usize> = BTreeMap::new();
        for f in &files {
            *by.entry(pack(f)).or_default() += 1;
        }
        let mut by: Vec<(String, usize)> = by.into_iter().collect();
        by.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let gap = match (native == "native", device == "device") {
            (false, _) => format!("not native: {native}"),
            _ => format!("not on the device: {device}"),
        };
        let packs = by.iter().map(|(p, n)| format!("{p} {n}")).collect::<Vec<_>>().join(", ");
        rows.push((files.len(), format!("{}\t{}\t{}\t{packs}", files.len(), pr.name, gap)));
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut text = String::from("# Executor gaps by the shipped programs that meet them (plan/coverage.rs, from matrix.tsv; RERAIL_BLESS_COVERAGE=1 writes this).\n# programs\tprobe\tgap\tprograms by pack\n");
    for (_, r) in rows {
        text.push_str(&r);
        text.push('\n');
    }
    let path = ledger().with_file_name("packs.tsv");
    if std::env::var_os("RERAIL_BLESS_COVERAGE").is_some() {
        std::fs::write(&path, &text).unwrap();
        return;
    }
    let had = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(had == text, "the executors' gaps by pack differ from {}: RERAIL_BLESS_COVERAGE=1 writes it", path.display());
}

/// The ledger's place.
fn ledger() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../gate/executor-coverage/matrix.tsv")
}

/// One cell, on one line: the explanation's own detail (rule numbers, slots; the rule a SQL error was in) left
/// out, so the ledger names the gap and does not churn with numbering or the probe's plan.
fn cell(s: &str) -> String {
    let s = s.replace(['\t', '\n'], " ");
    let s = match [" (rule #", ", in the SQL for "].iter().filter_map(|m| s.find(m)).min() {
        Some(i) => s[..i].to_owned(),
        None => s,
    };
    if s.len() > 160 { format!("{}…", &s[..s.floor_char_boundary(160)]) } else { s }
}

#[test]
fn every_operation_runs_on_every_executor() {
    let _rec = placement::recording();
    let mut rows = String::from("# The executor coverage gate (plan/coverage.rs; RERAIL_BLESS_COVERAGE=1 writes this): per probe, where it ran.\n# probe\tnative\tdevice\trule\n");
    let mut wrong = Vec::new();
    let mut native_gaps = 0;
    let mut device_gaps = 0;
    let all = probes();
    let path = ledger();
    let had = std::fs::read_to_string(&path).unwrap_or_default();
    // Without a Metal device (Linux, and so CI) the device column is the ledger's, as a Mac last measured it: only
    // the native column is checked there, and a blessing keeps the device cells it cannot measure.
    let measured: BTreeMap<&str, &str> = if metal() {
        BTreeMap::new()
    } else {
        had.lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| {
                let mut c = l.split('\t');
                Some((c.next()?, c.nth(1)?))
            })
            .collect()
    };
    for (k, pr) in all.iter().enumerate() {
        let rel = format!("cov_{k:03}_p");
        let rule = pr.rule.replace("{HEAD}", &rel);
        // `cols` may carry the declaration's tail after a `|` (a lattice's `widen k`).
        let (cols, tail) = pr.cols.split_once('|').unwrap_or((pr.cols, ""));
        let src = format!("{FACTS}.decl {rel}({cols}){tail}\n{rule}\n.output {rel}\n");
        let (native, device) = match run_probe(&src, &rel) {
            Ok(v) => (v.native, v.device),
            Err(e) => (e.clone(), e),
        };
        if native.starts_with("WRONG") || device.starts_with("WRONG") {
            wrong.push(format!("{}: {native} / {device}", pr.name));
        }
        native_gaps += usize::from(native != "native");
        device_gaps += usize::from(device != "device");
        // The rule as written with its head named `p`, and the cells without the probe's own name: a probe added
        // renumbers the others, and the ledger does not churn with it.
        let shown = pr.rule.replace("{HEAD}", "p").replace('\n', " ");
        let unnamed = |c: &str| cell(c).replace(&rel, "p");
        let device = match measured.get(pr.name.as_str()) {
            Some(kept) if device == "no Metal device" => (*kept).to_owned(),
            _ => unnamed(&device),
        };
        let _ = writeln!(rows, "{}\t{}\t{device}\t{shown}", pr.name, unnamed(&native));
    }
    eprintln!("{} probes: {native_gaps} not native, {device_gaps} not on the device", all.len());
    assert!(wrong.is_empty(), "an executor's rows differ from SQLite's:\n{}", wrong.join("\n"));
    if std::env::var_os("RERAIL_BLESS_COVERAGE").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &rows).unwrap();
        packs_ledger(&all, &rows);
        return;
    }
    // The gaps by pack follow from the matrix: compared once the matrix is as the ledger says.
    if had == rows {
        packs_ledger(&all, &rows);
    }
    if had != rows {
        let old: BTreeSet<&str> = had.lines().collect();
        let new: BTreeSet<&str> = rows.lines().collect();
        let gone: Vec<&&str> = old.difference(&new).collect();
        let came: Vec<&&str> = new.difference(&old).collect();
        panic!(
            "the executors' coverage differs from {}. A probe that now falls back is a missing step to build; one that now runs is progress to record. RERAIL_BLESS_COVERAGE=1 writes the ledger.\n--- ledger\n{}\n+++ now\n{}",
            path.display(),
            gone.iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n"),
            came.iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n")
        );
    }
}

/// The operations a file's statements use, by the names the probes give them (`Probe::ops`): read from the `.rr`
/// syntax (`rerail_rr_syntax`), as written, so no file needs its includes or a lowering to be counted.
fn ops_of(items: &[rerail_rr_syntax::syntax::Statement], out: &mut BTreeMap<String, String>, file: &str) {
    use rerail_rr_syntax::syntax::{Item, Lit, Term as S};
    let tests: BTreeMap<&str, &str> = crate::code_rule_dsl::logic::rr::lower::TESTS.into_iter().collect();
    let mut add = |op: String| {
        out.entry(op).or_insert_with(|| file.to_owned());
    };
    fn term(t: &S, add: &mut dyn FnMut(String), tests: &BTreeMap<&str, &str>) {
        match t {
            S::Call { name, functor, args } => {
                let name = match (name.as_str(), functor) {
                    ("dot", true) => "cat",
                    ("cat", false) => "concat",
                    (n, _) => n,
                };
                add(format!("call {name}"));
                add(format!("arity {name} {}", args.len()));
                args.iter().for_each(|a| term(a, add, tests));
            }
            S::Bin { op, l, r } => {
                if let Some((_, f)) = ARITHMETIC.iter().find(|(o, _)| o == op) {
                    add(format!("call {f}"));
                }
                term(l, add, tests);
                term(r, add, tests);
            }
            S::Agg { op, over, by, body, .. } => {
                add(format!("agg {op}"));
                over.iter().for_each(|o| term(o, add, tests));
                by.iter().for_each(|b| term(b, add, tests));
                body.iter().for_each(|l| lit(l, add, tests));
            }
            S::Named(_, t) => term(t, add, tests),
            _ => {}
        }
    }
    fn lit(l: &Lit, add: &mut dyn FnMut(String), tests: &BTreeMap<&str, &str>) {
        match l {
            Lit::Atom { rel, args, neg } => {
                match rel.as_str() {
                    r if tests.contains_key(r) => add(format!("test {}", tests[r])),
                    "nearest" | "ast" => add(rel.clone()),
                    _ => add(if *neg { "negation" } else { "atom" }.to_owned()),
                }
                args.iter().for_each(|a| term(a, add, tests));
            }
            Lit::Cmp { l, op, r } => {
                add(format!("cmp {op}"));
                term(l, add, tests);
                term(r, add, tests);
            }
            Lit::Pattern { subject, .. } => {
                add("test rx".to_owned());
                term(subject, add, tests);
            }
            Lit::Group { alts, .. } => alts.iter().flatten().for_each(|l| lit(l, add, tests)),
        }
    }
    for (item, _) in items {
        match item {
            Item::Lattice { .. } => add("lattice".to_owned()),
            Item::Clause { heads, body, .. } => {
                for h in heads {
                    h.args.iter().for_each(|t| term(t, &mut add, &tests));
                }
                body.iter().for_each(|l| lit(l, &mut add, &tests));
            }
            _ => {}
        }
    }
}

/// The shipped `.rr` files: the packs and the engine's own programs.
fn shipped() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    let mut walk = vec![root.join("../docs/rule-syntax"), root.join("src/code_rule_dsl")];
    while let Some(d) = walk.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk.push(p);
            } else if p.extension().is_some_and(|x| x == "rr") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// What the shipped files use: each operation (the probes' names, `ops_of`) with the first file that uses it, and
/// each call's arities (`arity <name> <n>`); read once.
/// Every operation the shipped files use, with the files that use it; the files read, and those unparsed.
fn census() -> &'static (BTreeMap<String, BTreeSet<String>>, usize, usize) {
    static CENSUS: std::sync::OnceLock<(BTreeMap<String, BTreeSet<String>>, usize, usize)> = std::sync::OnceLock::new();
    CENSUS.get_or_init(|| {
        use rayon::prelude::*;
        let files = shipped();
        let read: Vec<Option<BTreeMap<String, String>>> = files
            .par_iter()
            .map(|f| {
                let text = std::fs::read_to_string(f).ok()?;
                let items = rerail_rr_syntax::syntax::Parser::new(&text).and_then(|mut p| p.statements()).ok()?;
                let mut used = BTreeMap::new();
                ops_of(&items, &mut used, &f.display().to_string());
                Some(used)
            })
            .collect();
        let unread = read.iter().filter(|r| r.is_none()).count();
        let mut used: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for one in read.into_iter().flatten() {
            for (op, f) in one {
                used.entry(op).or_default().insert(f);
            }
        }
        (used, files.len(), unread)
    })
}

/// The `.functor` declarations of the shipped files (`core.rr`'s and any file's own): each name's arguments' kinds
/// and its result's.
fn functor_decls() -> BTreeMap<String, (Vec<Kind>, Kind)> {
    let kind = |t: &str| if t.trim() == "number" || t.trim() == "float" { Kind::Number } else { Kind::Text };
    let mut out = BTreeMap::new();
    for f in shipped() {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        for line in text.lines().filter_map(|l| l.trim_start().strip_prefix(".functor ")) {
            let Some((name, rest)) = line.split_once('(') else { continue };
            let Some((args, ret)) = rest.split_once(')') else { continue };
            let args: Vec<Kind> = args.split(',').filter(|a| !a.trim().is_empty()).map(|a| kind(a.split_once(':').map_or(a, |(_, t)| t))).collect();
            let ret = kind(ret.trim_start_matches(':').split("//").next().unwrap_or(""));
            out.entry(name.trim().to_owned()).or_insert((args, ret));
        }
    }
    out
}

/// Every operation a shipped rule uses is one the matrix probes: a rule that starts using a new function or form
/// fails here until the matrix covers it (and so says which executors run it).
#[test]
fn every_operation_a_shipped_rule_uses_has_a_probe() {
    let probed: BTreeSet<String> = probes().into_iter().flat_map(|p| p.ops).collect();
    let (used, files, unread) = census();
    // Forms the tree probes cover (`tree_forms_run_on_every_executor`) once they exist; until then, named here.
    let tree = ["nearest", "ast", "call doc", "call in", "call place", "call place_path", "call place_line", "call flat_bin", "call id", "call line", "call depth", "call within", "call tree_digest"];
    let missing: Vec<String> = used.iter().filter(|(op, _)| !op.starts_with("arity ") && !probed.contains(*op) && !tree.contains(&op.as_str())).map(|(op, f)| format!("{op} (first in {})", f.first().map_or("", String::as_str))).collect();
    eprintln!("{files} shipped files ({unread} unparsed), {} operations used", used.len());
    assert!(missing.is_empty(), "operations shipped rules use that no coverage probe runs:\n{}", missing.join("\n"));
}
