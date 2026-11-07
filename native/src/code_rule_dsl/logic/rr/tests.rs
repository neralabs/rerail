//! The `.rr` front end: the rule-syntax files parse and pass `check_rr.py`'s checks, the spellings lower to the
//! engine's forms, and every `.rdl` program translates to `.rr` that lowers to the same program.

use super::*;
use crate::code_rule_dsl::logic::front::{self, Literal, Term};
use std::collections::BTreeSet;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn rule_syntax() -> PathBuf {
    repo().join("docs/rule-syntax")
}

fn files(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            files(&p, ext, out);
        } else if p.extension().is_some_and(|x| x == ext) {
            out.push(p);
        }
    }
}

/// The hand-written `.rr` files: the library, the data-layer rules and the spikes (`site/` is the page's code).
fn rr_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    files(&rule_syntax(), "rr", &mut out);
    out.retain(|p| !p.starts_with(rule_syntax().join("site")));
    out
}

fn short(p: &Path) -> String {
    p.strip_prefix(rule_syntax()).unwrap_or(p).display().to_string()
}

#[test]
fn rule_syntax_files_parse() {
    let files = rr_files();
    assert!(files.len() >= 20, "{} files", files.len());
    let failures: Vec<String> = files.iter().filter_map(|f| read(f).err().map(|e| format!("{}: {e}", short(f)))).collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// `check_rr.py`: every rule grounded; per program (a file, its includes and `core.rr`) no relation declared twice
/// nor a built-in constraint declared, and every cycle through `!` or an aggregate allowed by `ordered by`.
#[test]
fn rule_syntax_files_pass_check_rr() {
    let lib: Vec<PathBuf> = rr_files().into_iter().filter(|p| p.starts_with(library())).collect();
    let mut given = BTreeSet::new();
    for f in &lib {
        given.extend(check::fix_relations(&read(f).unwrap().0));
    }
    let mut problems = Vec::new();
    for f in rr_files() {
        for (line, vars) in check::grounded(&read(&f).unwrap().0, &given) {
            problems.push(format!("{}:{line}: unbound {}", short(&f), vars.join(", ")));
        }
        let mut program = program(&f).unwrap().0;
        let core = library().join("core.rr").canonicalize().unwrap();
        if !program.iter().any(|(p, _)| *p == core) {
            program.push((core.clone(), read(&core).unwrap().0));
        }
        let named: Vec<(String, Vec<syntax::Statement>)> = program.iter().map(|(p, s)| (short(p), s.clone())).collect();
        problems.extend(check::declarations(&named).into_iter().map(|p| format!("{}: {p}", short(&f))));
        let all: Vec<&syntax::Statement> = program.iter().flat_map(|(_, s)| s).collect();
        problems.extend(check::strata(&all).into_iter().map(|p| format!("{}: {p}", short(&f))));
    }
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
}

/// The checks find what they are for.
#[test]
fn check_rr_finds_problems() {
    let statements = |src: &str| syntax::Parser::new(src).unwrap().statements().unwrap();
    let ungrounded = statements(".decl a(x: number)\n.decl b(x: number)\nb(x) :- !a(x).\nb(y) :- a(x), match(\"(?<y>.*)\", x).\n");
    let bad = check::grounded(&ungrounded, &BTreeSet::new());
    assert_eq!(bad, vec![(3, vec!["x".to_owned()])]);
    let cyclic = statements(".decl a(x: number)\n.decl b(x: number)\na(x) :- b(x), !a(x).\nb(x) :- a(x).\n");
    let refs: Vec<&syntax::Statement> = cyclic.iter().collect();
    assert_eq!(check::strata(&refs).len(), 1);
    let twice = vec![("a.rr".to_owned(), statements(".decl a(x: number)")), ("b.rr".to_owned(), statements(".decl a(x: number)\n.decl glob(x: symbol)"))];
    assert_eq!(check::declarations(&twice).len(), 2);
}

fn cmp(l: Term, op: &'static str, r: Term) -> Literal {
    Literal::Cmp { l, op, r }
}

fn call(f: &str, args: Vec<Term>) -> Term {
    Term::Call(f.to_owned(), args)
}

fn s(x: &str) -> Term {
    Term::Str(x.to_owned())
}

fn v(x: &str) -> Term {
    Term::Var(x.to_owned())
}

fn set(items: &[&str]) -> Term {
    call(crate::code_rule_dsl::logic::strings::SET, items.iter().map(|i| s(i)).collect())
}

/// The Soufflé spellings lower to the engine's forms.
#[test]
fn spellings_lower_to_the_engine_forms() {
    let p = parse(
        r#"
.decl name(n: node, s: symbol)
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.input name, node
.decl scope(kind: symbol)
scope("module"). scope("class_definition").
.lattice shortlex_min<symbol> { Lub -> @shortlex_min }
.decl best(n: node, r: shortlex_min<>) widen 8
.decl cv(n: node, v: flat<>)
.decl private(n: node)
.decl kinds(n: node, k: number)
.decl owner(n: node, a: node)
.decl first(n: node, i: number)
.pragma "magic-transform" "private, cv"
private(n) :- name(n, x), prefix("_", x), !glob("*_test", x), !glob("test_*", x).
private(n) :- name(n, x), (glob("a*", x) ; glob("b*", x)).
private(n) :- name(n, x), (x = "a" ; x = "b").
private(n) :- name(n, x), match("[a-z]+", x).
private(n) :- name(n, x), (prefix("a", x), suffix("b", x) ; contains("c", x)).
owner(n, a) :- nearest(n, a, scope).
owner(n, a) :- inside(n, a, "module").
kinds(n, k) :- node(_, n, _, _, _), k = count : { node(_, c, _, _, _), inside(c, n, scope) }.
first(n, i) :- node(_, n, _, _, _), i = min j : { name(n, x), node(_, n, _, j, _) }, count : { name(n, _) } > 0.
cv(n, top) :- name(n, _).
best(n, @dot("a", x)) :- name(n, x).
"#,
    )
    .unwrap();
    assert_eq!(p.decls["name"], vec![("n".to_owned(), "node".to_owned()), ("s".to_owned(), "sym".to_owned())]);
    assert_eq!(p.kindsets["scope"], vec!["module".to_owned(), "class_definition".to_owned()]);
    assert!(!p.decls.contains_key("scope") && !p.facts.contains_key("scope"));
    let best = &p.lattice["best"];
    assert_eq!((best.keys, best.ty.as_str(), best.merge.as_deref(), best.widen), (1, "sym", Some("shortlex_min"), Some(8)));
    assert_eq!((p.lattice["cv"].ty.as_str(), p.lattice["cv"].merge.as_deref()), ("flat", None));
    assert_eq!(p.demand, vec!["private".to_owned(), "cv".to_owned()]);
    let rules: Vec<&front::Rule> = p.rules.iter().filter(|r| r.head.rel == "private").collect();
    assert_eq!(rules[0].body[1..], [cmp(call("prefix", vec![v("x"), s("_")]), "=", Term::Int(1)), cmp(call("glob", vec![v("x"), set(&["*_test", "test_*"])]), "=", Term::Int(0))]);
    assert_eq!(rules[1].body[1], cmp(call("glob", vec![v("x"), set(&["a*", "b*"])]), "=", Term::Int(1)));
    assert_eq!(rules[2].body[1], cmp(call("equals", vec![v("x"), set(&["a", "b"])]), "=", Term::Int(1)));
    assert_eq!(rules[3].body[1], cmp(call("rx", vec![v("x"), s("^(?:[a-z]+)$")]), "=", Term::Int(1)));
    // A disjunction is a rule per alternative.
    assert_eq!(rules.len(), 6);
    let owner: Vec<&front::Rule> = p.rules.iter().filter(|r| r.head.rel == "owner").collect();
    assert_eq!(owner[0].body, [Literal::Nearest { n: "n".to_owned(), a: "a".to_owned(), kindset: "scope".to_owned() }]);
    assert!(matches!(&owner[1].body[0], Literal::Atom(a) if a.rel == "inside_rr_kind_module"));
    // A count over two literals reads a helper of every variable, grouped by the ones shared (`_` for the body's
    // own); a min over two, the ones shared and its own.
    let helper = |head: &str| -> Vec<String> {
        let r = p.rules.iter().find(|r| r.head.rel == head).unwrap();
        r.body.iter().filter_map(|l| if let Literal::Agg { atom, .. } = l { Some(format!("{atom}")) } else { None }).collect()
    };
    assert_eq!(helper("kinds"), vec!["rr_agg1(_, $n)".to_owned()]);
    assert_eq!(p.decls["rr_agg1"].len(), 2);
    let first = helper("first");
    assert_eq!(first.len(), 2);
    assert!(first[0].contains("$j") && first[0].contains("$n") && !first[0].contains("$x"), "{first:?}");
    let best_rule = p.rules.iter().find(|r| r.head.rel == "best").unwrap();
    assert_eq!(best_rule.head.args[1], call("cat", vec![s("a"), v("x")]));
    assert_eq!(p.rules.iter().find(|r| r.head.rel == "cv").unwrap().head.args[1], Term::Top);
}

/// What is not lowered yet says so by name.
#[test]
fn unlowered_constructs_are_named() {
    for (src, what) in [
        (".decl a(n: node)\na(n) :- n ~ `exec($X)`.", "pattern"),
        (".decl a(x: number)\nmust a(1) :- a(2).", "must"),
        (".decl a(x: number)\na(x) :- a(y), x = mean z : { a(z) }.", "mean"),
        (".decl a(x: symbol)\na(x) :- a(x), a(y), !match(\"(?<z>.*)\", y).", "named groups"),
        (".decl t(s: symbol)\n.input t\n.violation v(at: span) \"m\"\nv(m) :- t(s), match(\"(?<m>x+)\", s).", "at a span that can be a span the engine does not make (a regex capture)"),
        (".decl first_token(n: node, t: span)\n.input first_token\n.violation v(at: loc) \"m\"\nv(t) :- first_token(_, t).", "at a loc that can be a span the engine does not make (first_token, a span)"),
    ] {
        let e = parse(src).unwrap_err().to_string();
        assert!(e.contains("not lowered yet") && e.contains(what), "{src}: {e}");
    }
}

/// `.violation` rows become the engine's findings; fixes are left out; arithmetic and Soufflé's string
/// functions are functors; a computed atom argument is bound first; `!nearest(n, _, set)` is `!inside`.
#[test]
fn directives_and_functions_lower() {
    let p = parse(concat!(
        ".policy \"data-layer:p\" revision 1\n",
        ".decl child(p: node, c: node, i: number)\n.decl text(n: node, s: symbol)\n.decl e(s: symbol)\n.input child, text, e\n",
        ".decl fdef(k: symbol)\nfdef(\"function_definition\").\n",
        ".violation BAD(at: node, s: symbol) code {s} \"{s} at {at}\"\n",
        "BAD(c, s) :- child(p, c, i), j = i + 1, child(p, _, j), text(c, t), s = cat(t, to_string(strlen(t))), !nearest(c, _, fdef).\n",
        ".edit drop(n: node)\ndrop(c) :- child(_, c, 0) => delete(c).\n",
        ".decl r(s: symbol)\nr(s) :- text(_, s), e(@dot(s, \"x\")).\n",
    ))
    .unwrap();
    assert!(p.outputs.contains(&"BAD".to_owned()) && p.outputs.contains(&"violation".to_owned()));
    let rule = |rel: &str| p.rules.iter().find(|r| r.head.rel == rel).map(ToString::to_string).unwrap_or_default();
    assert_eq!(rule("violation"), "violation(\"data-layer:p\", \"BAD\", $rr_v1, $rr_p, $rr_l) :- BAD($rr_v0, $rr_v1), node($rr_d, $rr_v0, _, _, _), doc($rr_d, $rr_p, _), line($rr_v0, $rr_l)");
    let bad = rule("BAD");
    for part in ["$j = @add($i, 1)", "@concat($t, @to_string(@strlen($t)))", "!inside_fdef($c, _)"] {
        assert!(bad.contains(part), "{part} in {bad}");
    }
    assert!(!p.rules.iter().any(|r| r.head.rel == "drop"), "a fix is not run");
    assert!(rule("r").contains("= @cat($s, \"x\"), e($rr_t"), "{}", rule("r"));
}

/// A program and what it was read from, compared field by field: the first difference.
fn difference(a: &front::Program, b: &front::Program) -> Option<String> {
    macro_rules! field {
        ($f:ident) => {
            if a.$f != b.$f {
                let (x, y) = (format!("{:#?}", a.$f), format!("{:#?}", b.$f));
                let at = x.lines().zip(y.lines()).position(|(l, r)| l != r).unwrap_or(0);
                let around = |t: &str| t.lines().skip(at.saturating_sub(3)).take(8).collect::<Vec<_>>().join("\n");
                return Some(format!("{}: .rdl\n{}\n.rr\n{}", stringify!($f), around(&x), around(&y)));
            }
        };
    }
    field!(decls);
    field!(lattice);
    field!(facts);
    field!(rules);
    field!(inputs);
    field!(outputs);
    field!(kindsets);
    field!(demand);
    field!(policies);
    None
}

/// The authored `.rdl` programs, as (name, text): T's, the pinned goldens, the data-layer programs (each after its
/// prelude) and the serverplan-quality program.
fn authored() -> Vec<(String, String, PathBuf)> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl");
    let mut out = Vec::new();
    for dir in ["typing/programs", "logic/plan/golden/programs"] {
        let mut found = Vec::new();
        files(&src.join(dir), "rdl", &mut found);
        out.extend(found.into_iter().map(|p| (format!("{dir}/{}", p.file_name().unwrap().to_string_lossy()), p)));
    }
    out.into_iter().map(|(name, path)| (name, std::fs::read_to_string(&path).unwrap(), path)).collect()
}

/// An `.rdl` program as its translation lowers it: relations named as built-ins renamed, adjacent negations of one
/// test merged (both keep the program's meaning).
fn normalized(mut p: front::Program) -> front::Program {
    let r = |name: &str| translate::relation(name);
    let rename = |a: &mut front::Atom| a.rel = r(&a.rel);
    p.decls = std::mem::take(&mut p.decls).into_iter().map(|(k, v)| (r(&k), v)).collect();
    p.lattice = std::mem::take(&mut p.lattice).into_iter().map(|(k, v)| (r(&k), v)).collect();
    p.facts = std::mem::take(&mut p.facts).into_iter().map(|(k, v)| (r(&k), v)).collect();
    for list in [&mut p.inputs, &mut p.outputs, &mut p.demand] {
        list.iter_mut().for_each(|n| *n = r(n));
    }
    for rule in &mut p.rules {
        rename(&mut rule.head);
        for l in &mut rule.body {
            if let Literal::Atom(a) | Literal::Agg { atom: a, .. } = l {
                rename(a);
            }
        }
        rule.body = lower::merge_negated_tests(std::mem::take(&mut rule.body));
    }
    p
}

fn error_name(e: &Error) -> String {
    format!("{e:?}").split(['(', ' ', '{']).next().unwrap_or_default().to_owned()
}

/// Each copied `.rdl` program (T's, until step 31 removes them) translates to `.rr` that lowers to the program the
/// `.rdl` parses to (or fails with the same named error), and its `.rr` beside it is that translation.
#[test]
fn rdl_programs_translate_to_the_same_program() {
    let mut failures = Vec::new();
    for (name, text, path) in authored() {
        let rr = match translate(&text) {
            Ok(rr) => rr,
            Err(e) => {
                failures.push(format!("{name}: translate: {e}"));
                continue;
            }
        };
        let beside = path.with_extension("rr");
        if beside.exists() && std::fs::read_to_string(&beside).unwrap() != rr {
            failures.push(format!("{name}: {} is not its translation", beside.display()));
        }
        match (front::parse(&text).map(normalized), parse(&rr)) {
            (Ok(a), Ok(b)) => {
                failures.extend(difference(&a, &b).map(|d| format!("{name}: {d}")));
                // Printed, the program reads back as itself.
                match print(&b).and_then(|text| parse(&text)) {
                    Ok(c) => failures.extend(difference(&b, &c).map(|d| format!("{name}, printed: {d}"))),
                    Err(e) => failures.push(format!("{name}, printed: {e}")),
                }
            }
            (Err(a), Err(b)) if error_name(&a) == error_name(&b) => {}
            (a, b) => failures.push(format!("{name}: .rdl {:?} but .rr {:?}", a.err().map(|e| e.to_string()), b.err().map(|e| e.to_string()))),
        }
    }
    assert!(failures.is_empty(), "{} programs differ:\n{}", failures.len(), failures.join("\n\n"));
}

/// The engine's own programs (the data-layer programs on their prelude, the serverplan quality program), printed,
/// read back as themselves.
#[test]
fn rr_programs_print_as_themselves() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl/logic/programs/data_layer");
    let prelude = std::fs::read_to_string(dir.join("prelude.rr")).unwrap();
    let mut found = Vec::new();
    files(&dir, "rr", &mut found);
    let mut programs: Vec<(String, String)> = found
        .into_iter()
        .filter(|p| !p.ends_with("prelude.rr"))
        .map(|p| (short(&p), format!("{prelude}\n{}", std::fs::read_to_string(&p).unwrap())))
        .collect();
    let quality = repo().join("gate/data-layer/final-shape/data-layer-serverplan-quality.rr");
    programs.push((short(&quality), std::fs::read_to_string(&quality).unwrap()));
    let mut failures = Vec::new();
    let mut lowered = 0;
    for (name, text) in programs {
        let Ok(b) = parse(&text) else { continue };
        lowered += 1;
        match print(&b).and_then(|text| parse(&text)) {
            Ok(c) => failures.extend(difference(&b, &c).map(|d| format!("{name}, printed: {d}"))),
            Err(e) => failures.push(format!("{name}, printed: {e}")),
        }
    }
    assert!(lowered >= 20, "{lowered} programs lowered");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// A regex test prints back as written: `match("X", s)` lowers to the engine's search of `^(?:X)$` and prints as
/// `match("X", s)` again, never under a leading `(?s)` that would give X's own `.` newlines; a search built another
/// way prints with the flag on its dots only.
#[test]
fn regex_tests_print_back_exactly() {
    let src = r#"
.decl t(s: symbol)
.input t
.decl a(s: symbol)
a(s) :- t(s), match("WHEN.*BEGIN", s).
.decl b(s: symbol)
b(s) :- t(s), !match("(?i)x(y)", s).
.output a, b
"#;
    let prog = parse(src).unwrap();
    let text = print(&prog).unwrap();
    assert!(text.contains(r#"match("WHEN.*BEGIN", s)"#), "{text}");
    assert!(text.contains(r#"!match("(?i)x(y)", s)"#), "{text}");
    assert!(!text.contains("(?s)"), "{text}");
    assert_eq!(difference(&prog, &parse(&text).unwrap()), None);
    assert_eq!(super::print::search("rx", "a.b"), "(?s:.*)(?:a.b)(?s:.*)");
    assert_eq!(super::print::search("irx", "a"), "(?i:(?s:.*)(?:a)(?s:.*))");
    assert_eq!(super::print::search("rx", "^(?:a)|(?:b)$"), "(?s:.*)(?:^(?:a)|(?:b)$)(?s:.*)");
    assert_eq!(super::print::search("rx", r"^(?:a\)[)]b)$"), r"a\)[)]b");
}

/// Writes each authored program's `.rr` beside it (run once per change of an `.rdl`).
#[test]
#[ignore = "writes files: the .rr translations of the authored .rdl programs"]
fn write_rr_translations() {
    for (name, text, path) in authored() {
        std::fs::write(path.with_extension("rr"), translate(&text).unwrap_or_else(|e| panic!("{name}: {e}"))).unwrap();
    }
}

/// Which hand-written data-layer rules lower to the engine today, and what stops the others (a report).
/// `RERAIL_RR_REPORT_EXTRA=<dir>[:<dir>…]` reports the `.rr` files under those directories too (another branch's
/// packs, say), with `#include` still read from this library.
#[test]
#[ignore = "a report: what the hand-written rules need from the lowering"]
fn hand_written_rules_lowering_report() {
    let mut out = Vec::new();
    files(&rule_syntax().join("data-layer"), "rr", &mut out);
    files(&library(), "rr", &mut out);
    for dir in std::env::var("RERAIL_RR_REPORT_EXTRA").iter().flat_map(|dirs| dirs.split(':')).filter(|d| !d.is_empty()) {
        files(Path::new(dir), "rr", &mut out);
    }
    for f in out {
        let result = parse_file(&f);
        println!("{}: {}", short(&f), result.map_or_else(|e| e.to_string(), |p| format!("lowers: {} rules", p.rules.len())));
    }
}

// ---- aggregates ----

/// `sum` and `join` over one atom read it in place: the body's own variables are `_`, and join's keys are variables
/// of the atom. A value or key that is an expression, or an atom that repeats a body variable, is a helper relation
/// (sum's keeps every variable, a multiset), with the computed value typed by what computes it. Without `sep`, join
/// joins the texts as they are.
#[test]
fn sum_and_join_lower() {
    let p = parse(concat!(
        ".decl arg(c: node, t: symbol, i: number)\n.input arg\n",
        ".decl s(c: node, n: number)\ns(c, n) :- arg(c, _, _), n = sum i : { arg(c, t, i) }.\n",
        ".decl j(c: node, x: symbol)\nj(c, x) :- arg(c, _, _), x = join t by i sep \", \" : { arg(c, t, i) }.\n",
        ".decl k(c: node, x: symbol)\nk(c, x) :- arg(c, _, _), x = join t by (n, i) sep \"\" : { arg(c, t, i), n = 0 - i }.\n",
        ".decl g(c: node, n: number)\ng(c, n) :- arg(c, _, _), n = sum strlen(t) - 1 : { arg(c, t, _) }.\n",
        ".decl r(c: node, n: number)\nr(c, n) :- arg(c, _, _), n = count : { arg(c, t, t) }.\n",
    ))
    .unwrap();
    let agg = |rel: &str| {
        let r = p.rules.iter().find(|r| r.head.rel == rel).unwrap();
        r.body.iter().find_map(|l| if let Literal::Agg { op, over, atom, .. } = l { Some((op.clone(), over.clone(), atom.clone())) } else { None }).unwrap()
    };
    let (op, over, atom) = agg("s");
    assert_eq!((op, over.as_deref(), atom.to_string()), (front::AggOp::Sum, Some("i"), "arg($c, _, $i)".to_owned()));
    let (op, over, atom) = agg("j");
    assert_eq!((op, over.as_deref(), atom.to_string()), (front::AggOp::Join { by: vec!["i".to_owned()], sep: ", ".to_owned() }, Some("t"), "arg($c, $t, $i)".to_owned()));
    // Keys (n, i): the helper's columns in order c, i, n, t.
    let (op, _, atom) = agg("k");
    assert_eq!(op, front::AggOp::Join { by: vec!["n".to_owned(), "i".to_owned()], sep: String::new() });
    let types: Vec<&str> = p.decls[&atom.rel].iter().map(|(_, t)| t.as_str()).collect();
    assert_eq!(types, ["node", "int", "int", "sym"], "{atom}");
    let (op, over, atom) = agg("g");
    let over = over.unwrap();
    assert!(op == front::AggOp::Sum && over.starts_with("rr_over"), "{over}");
    let cols = &p.decls[&atom.rel];
    assert_eq!(cols.len(), 3, "c, t and the value: {cols:?}");
    assert!(cols.contains(&(over.clone(), "int".to_owned())), "{cols:?}");
    let (_, _, atom) = agg("r");
    assert_ne!(atom.rel, "arg", "arg(c, t, t) tests t = t: a helper relation");
    let e = ".decl a(x: number)\na(x) :- a(y), x = sum z sep \",\" : { a(z) }.";
    assert!(matches!(parse(e), Err(front::Error::Syntax(_))), "{e}");
    let p = parse(".decl b(z: symbol)\n.input b\n.decl a(x: symbol)\na(x) :- x = join z : { b(z) }.\n").unwrap();
    let r = p.rules.iter().find(|r| r.head.rel == "a").unwrap();
    assert!(r.body.iter().any(|l| matches!(l, Literal::Agg { op: front::AggOp::Join { by, sep }, .. } if by.is_empty() && sep.is_empty())), "{r}");
}

/// An aggregate inside a term, in a head or a comparison, is an aggregate of its own whose value the term uses.
#[test]
fn aggregates_inside_terms_lower() {
    let p = parse(concat!(
        ".decl d(f: node, n: node)\n.input d\n",
        ".decl cc(f: node, k: number)\ncc(f, 1 + count : { d(f, n) }) :- d(f, _).\n",
        ".decl two(f: node, k: number)\ntwo(f, k) :- d(f, _), k = count : { d(f, n) } + count : { d(n, f) }, k > 1.\n",
    ))
    .unwrap();
    let cc = p.rules.iter().find(|r| r.head.rel == "cc").unwrap();
    let Some(Literal::Agg { var, op: front::AggOp::Count, .. }) = cc.body.iter().find(|l| matches!(l, Literal::Agg { .. })) else { panic!("{cc}") };
    assert_eq!(cc.head.args[1], Term::Call("add".to_owned(), vec![Term::Int(1), Term::Var(var.clone())]), "{cc}");
    let two = p.rules.iter().find(|r| r.head.rel == "two").unwrap();
    assert_eq!(two.body.iter().filter(|l| matches!(l, Literal::Agg { .. })).count(), 2, "{two}");
}

/// The aggregates print as `.rr` that reads back as the same program.
#[test]
fn aggregates_print_as_themselves() {
    let b = parse(concat!(
        ".decl arg(c: node, t: symbol, i: number)\n.input arg\n",
        ".decl j(c: node, x: symbol, y: symbol, n: number, m: number)\n",
        "j(c, x, y, n, 2 * count : { arg(c, _, _) }) :- arg(c, _, _), x = join t by i sep \"\\\", \" : { arg(c, t, i) },\n",
        "    y = join t by (k, i) sep \"\" : { arg(c, t, i), k = strlen(t) }, n = sum strlen(t) : { arg(c, t, _) }, m = sum i : { arg(c, _, i) }.\n",
        ".output j\n",
    ))
    .unwrap();
    let c = parse(&print(&b).unwrap()).unwrap_or_else(|e| panic!("{e}\n{}", print(&b).unwrap()));
    assert_eq!(difference(&b, &c), None, "{}", print(&b).unwrap());
}

// ---- functions ----

/// Soufflé's `cat` of any arity is `concat` folded from the left; `max`, `min`, `to_number` and `%` (`mod`) are the
/// engine's functions of those names; each prints and reads back as itself. Floats stay refused by name: the engine
/// has no float values.
#[test]
fn souffle_functions_lower() {
    let p = parse(concat!(
        ".decl t(s: symbol, n: number)\n.input t\n",
        ".decl a(s: symbol, n: number)\n",
        "a(cat(s, \"-\", s, \".\"), max(n, 0) + min(n, 9) % 4) :- t(s, _), t(_, n).\n",
        "a(@lower(s), to_number(s)) :- t(s, _).\n",
    ))
    .unwrap();
    let heads: Vec<String> = p.rules.iter().filter(|r| r.head.rel == "a").map(|r| r.head.to_string()).collect();
    assert_eq!(
        heads,
        [
            "a(@concat(@concat(@concat($s, \"-\"), $s), \".\"), @add(@max($n, 0), @mod(@min($n, 9), 4)))",
            "a(@lower($s), @to_number($s))",
        ]
    );
    let back = parse(&print(&p).unwrap()).unwrap();
    assert_eq!(difference(&p, &back), None);
    for (src, what) in [
        (".decl a(x: number)\na(x) :- a(y), x = to_float(\"1\").", "to_float/1"),
        (".decl a(x: number)\na(x) :- a(y), x = y + 1.5.", "decimal number 1.5"),
        (".decl a(x: symbol)\na(x) :- a(y), x = cat(y).", "cat/1"),
    ] {
        let e = parse(src).unwrap_err().to_string();
        assert!(e.contains("not lowered yet") && e.contains(what), "{src}: {e}");
    }
}

// ---- violations ----

/// A `.violation`'s constant code is the finding's code; its `suppress` and `fix` lines are settings of the
/// findings layer; a `loc` column holding nodes and `@place`s gives each its finding, a node at its document and
/// line, a place at its path and line; and the program prints as itself.
#[test]
fn violations_at_places_and_their_lines_lower() {
    let p = parse(concat!(
        ".policy \"p\" revision 1\n",
        ".decl doc(d: number, path: symbol, module: symbol)\n.decl node(d: number, n: node, kind: symbol, start: number, end: number)\n",
        ".decl line(n: node, l: number)\n.input doc, node, line\n",
        ".violation V(at: loc, x: symbol) code \"@scope/a-b\" \"{x}\"\n",
        "    suppress eslint \"@scope/a-b\", \"a-b\" over {x}\n",
        "    suppress oxlint \"a-b\"\n",
        "    fix safe drop \"Remove it\"\n",
        "V(n, \"call\") :- node(_, n, \"call\", _, _).\n",
        "V(@place(p, 0), p) :- doc(_, p, _).\n",
        ".decl spot(at: loc)\nspot(@place(\"a.json\", 3)) :- doc(_, _, _).\n",
        ".violation W(at: span) \"w\"\nW(s) :- spot(s).\n",
    ))
    .unwrap();
    let settings: Vec<(&str, &str)> = p.settings.iter().map(|(k, t)| (k.as_str(), t.as_str())).collect();
    assert_eq!(settings, [("policy", "\"p\" revision 1"), ("suppress", "V eslint \"@scope/a-b\", \"a-b\" over {x}"), ("suppress", "V oxlint \"a-b\""), ("fix", "V safe drop \"Remove it\"")]);
    let rules = |rel: &str| p.rules.iter().filter(|r| r.head.rel == "violation" && r.head.args[1] == s(rel)).map(ToString::to_string).collect::<Vec<_>>();
    assert_eq!(
        rules("V"),
        [
            "violation(\"p\", \"V\", \"@scope/a-b\", $rr_p, $rr_l) :- V($rr_v0, _), node($rr_d, $rr_v0, _, _, _), doc($rr_d, $rr_p, _), line($rr_v0, $rr_l)",
            "violation(\"p\", \"V\", \"@scope/a-b\", @place_path($rr_v0), @place_line($rr_v0)) :- V($rr_v0, _), $rr_v0 < -1",
        ]
    );
    // A span column that only places reach has no node finding.
    assert_eq!(rules("W"), ["violation(\"p\", \"W\", \"W\", @place_path($rr_v0), @place_line($rr_v0)) :- W($rr_v0), $rr_v0 < -1"]);
    let printed = print(&p).unwrap();
    let again = parse(&printed).unwrap();
    assert_eq!(difference(&p, &again), None, "{printed}");
    assert_eq!(p.settings, again.settings);
}

// ---- tree axes and node sets ----

/// `has(a, n, set)` is the descendant axis, the set tested on n; a set of nodes is read as rules over `child`,
/// not a kind set; the forms print as themselves.
#[test]
fn axes_lower_and_print_as_themselves() {
    let p = parse(
        r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, c: node, i: number)
.decl docof(n: node, d: number)
.decl next_sibling(a: node, b: node)
.input node, child, docof, next_sibling
.decl fn(kind: symbol)
fn("function_definition").
.decl blk(n: node)
blk(n) :- node(_, n, "block", _, _), child(n, _, 0).
.decl r(a: node, n: node)
r(a, n) :- has(a, n, fn).
r(a, n) :- node(_, a, _, _, _), !has(a, _, fn), nearest(n, a, blk).
r(a, n) :- node(_, a, _, _, _), node(_, n, _, _, _), !nearest(n, a, fn), !within_stop(n, a, blk), !earlier(a, n), precedes_stop(a, n, blk), next_sibling(a, n).
.decl s(q: symbol)
s(q) :- s(x), split_dot(x, q, _), !split_dot(x, q, "y").
"#,
    )
    .unwrap();
    let rules: Vec<&front::Rule> = p.rules.iter().filter(|r| r.head.rel == "r").collect();
    let v = |s: &str| Term::Var(s.to_owned());
    assert_eq!(rules[0].body, [Literal::Atom(front::Atom { rel: "inside_rr_has_fn".to_owned(), args: vec![v("n"), v("a")], neg: false, gated: false })]);
    assert!(!p.kindsets.contains_key("blk") && p.kindsets.contains_key("fn"));
    // The nearest block: down from each member of blk to the next, reading blk negated.
    let near: Vec<String> = p.rules.iter().filter(|r| r.head.rel == "rr_near_blk").map(ToString::to_string).collect();
    assert_eq!(near, ["rr_near_blk($n, $a) :- child($a, $n, _), blk($a)", "rr_near_blk($n, $a) :- child($p, $n, _), !blk($p), rr_near_blk($p, $a)"]);
    assert!(rules[1].body.iter().any(|l| l.to_string() == "rr_near_blk($n, $a)"), "{}", rules[1]);
    // `next_sibling` is core.rr's, written out over `child` in the rule (`rr::lower::core`): no relation of the program.
    assert!(!p.rules.iter().any(|r| r.head.rel == "next_sibling") && !p.inputs.contains(&"next_sibling".to_owned()));
    let again = parse(&print(&p).unwrap()).unwrap();
    assert_eq!(difference(&p, &again), None);
}

/// What the axes still refuse says so by name; core.rr's `within` and `earlier` negate with a node unnamed, through
/// their relations (`rr::lower::core`): n has no ancestor, n has nothing after it.
#[test]
fn axes_refuse_by_name() {
    let decls = ".decl node(d: number, n: node, kind: symbol, start: number, end: number)\n.decl tape(n: number, par: number, size: number, depth: number)\n.input node, tape\n.decl a(n: node, m: node)\n.decl q(s: symbol)\n.decl fn(kind: symbol)\nfn(\"f\").\n";
    for (body, what) in [
        ("a(n, n) :- a(n, _), !within_stop(n, _, fn).", "`!within_stop` with its ancestor unnamed"),
        ("q(s) :- q(s), !split_dot(s, _, \"x\").", "`!split_dot` with a column unnamed"),
        ("a(n, n) :- a(n, _), !precedes_stop(n, _, fn).", "`!precedes_stop` with a node unnamed"),
    ] {
        let e = parse(&format!("{decls}{body}")).unwrap_err().to_string();
        assert!(e.contains("not lowered yet") && e.contains(what), "{body}: {e}");
    }
    for body in ["a(n, n) :- a(n, _), !within(n, _).", "a(n, n) :- a(n, _), !earlier(n, _)."] {
        parse(&format!("{decls}{body}")).unwrap_or_else(|e| panic!("{body}: {e}"));
    }
}

// ---- recursion ----

/// A negated conjunction is a helper relation of the variables it shares with the rule, negated; `!(a ; b)` is one
/// such helper per alternative; a shared variable the conjunction only tests is bound in the helper by the rule's
/// other literals; a conjunction sharing none has one constant column. Printed, the program reads back as itself.
#[test]
fn negated_conjunctions_lower_to_helpers() {
    let p = parse(concat!(
        ".decl e(x: number)\n.decl f(x: number, y: number)\n.decl g(y: number)\n.decl o(k: symbol, v: symbol)\n.input e, f, g, o\n",
        ".decl a(x: number)\na(x) :- e(x), !(f(x, y), g(y)).\n",
        ".decl b(x: number)\nb(x) :- e(x), !(f(x, _) ; g(x)).\n",
        ".decl c(x: number)\nc(x) :- e(x), !(x > 2, o(\"k\", \"on\")).\n",
        ".decl d(x: number)\nd(x) :- e(x), !(o(\"k\", \"on\"), g(_)).\n",
    ))
    .unwrap();
    let rules = |rel: &str| -> Vec<&front::Rule> { p.rules.iter().filter(|r| r.head.rel == rel).collect() };
    // The rule's negated helpers, and each helper's rules as text with its name as `H`.
    let helpers = |rel: &str| -> Vec<(String, Vec<String>)> {
        let [rule] = rules(rel)[..] else { panic!("{rel}: one rule") };
        rule.body
            .iter()
            .filter_map(|l| match l {
                Literal::Atom(a) if a.neg => Some((
                    a.to_string().replace(&a.rel, "H"),
                    rules(&a.rel).iter().map(|r| r.to_string().replace(&a.rel, "H")).collect(),
                )),
                _ => None,
            })
            .collect()
    };
    assert_eq!(helpers("a"), [("!H($x)".to_owned(), vec!["H($x) :- f($x, $y), g($y)".to_owned()])]);
    assert_eq!(helpers("b"), [("!H($x)".to_owned(), vec!["H($x) :- f($x, _)".to_owned()]), ("!H($x)".to_owned(), vec!["H($x) :- g($x)".to_owned()])]);
    let c = helpers("c");
    assert_eq!(c.len(), 1);
    let [body] = &c[0].1[..] else { panic!("{c:?}") };
    assert!(body.starts_with("H($x) :- rr_ctx") && body.ends_with("($x), $x > 2, o(\"k\", \"on\")"), "{body}");
    assert_eq!(helpers("d"), [("!H(0)".to_owned(), vec!["H(0) :- o(\"k\", \"on\"), g(_)".to_owned()])]);
    let printed = print(&p).and_then(|text| parse(&text)).unwrap();
    assert_eq!(difference(&p, &printed), None);
}

/// Decision 0044 §4: a program is a derivation of the library files it read. Reading it again
/// reuses it; editing a file it read, or creating one where an include looked, reads it again.
#[test]
fn programs_are_kept_by_the_hashes_of_what_they_read() {
    let dir = std::env::temp_dir().join(format!("rr-library-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (main, helper) = (dir.join("main.rr"), dir.join("zz_helper_0044.rr"));
    std::fs::write(&helper, ".decl h(x: number)\nh(1).\n").unwrap();
    std::fs::write(&main, "#include \"zz_helper_0044.rr\"\n.decl r(x: number)\nr(x) :- h(x).\n.output r\n").unwrap();
    let (first, files) = read_file(&main).unwrap();
    let names: Vec<String> = files.iter().map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(names.first().map(String::as_str), Some("core.rr"), "core.rr first: {names:?}");
    // In reading order: each file, then what it includes; the helper twice, as the library probe
    // that found nothing and as the file beside main.rr.
    assert_eq!(names, ["core.rr", "main.rr", "zz_helper_0044.rr", "zz_helper_0044.rr"]);
    assert!(files.iter().any(|f| f.sha256.is_none() && f.path.starts_with(library())), "the include's library probe is recorded");
    assert!(files.iter().filter_map(LibraryFile::fact).all(|f| f.starts_with("library_file(") && f.contains("\"sha256:")));
    let (again, files_again) = read_file(&main).unwrap();
    assert_eq!((format!("{first:?}"), &files), (format!("{again:?}"), &files_again), "unchanged files: the same program");
    std::fs::write(&helper, ".decl h(x: number)\nh(2).\n").unwrap();
    let (edited, edited_files) = read_file(&main).unwrap();
    assert_ne!(format!("{first:?}"), format!("{edited:?}"), "an edited include is read again");
    assert_ne!(files, edited_files);
    std::fs::remove_dir_all(&dir).unwrap();
}
