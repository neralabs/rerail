//! The `.rr` front end: the rule-syntax files parse and pass `check_rr.py`'s checks, the spellings lower to the
//! engine's forms, and every `.rdl` program translates to `.rr` that lowers to the same program.

use super::*;
use crate::code_rule_dsl::logic::front::{self, Literal, Term};
use std::collections::BTreeSet;

fn finding_contract(source: &str, program: &str) -> Vec<serde_json::Value> {
    let root = crate::code_rule_dsl::prefilter::tests::temp().with_extension("rr-contract");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.py"), source).unwrap();
    let rule = root.join("rule.rr");
    std::fs::write(&rule, program).unwrap();
    let report = crate::code_rule_dsl::tools::run_rr(&rule, &root, &root.join("out"), &serde_json::Value::Null).unwrap();
    let rows = report["diagnostics"].as_array().unwrap().clone();
    std::fs::remove_dir_all(root).unwrap();
    rows
}

#[test]
fn invalid_or_unsupported_boolean_regex_fails_at_compilation() {
    for pattern in ["[", "(?<![A-Za-z_é])str"] {
        let rule = format!(".decl sample(s: symbol)\nsample(\"str\").\n.decl found(s: symbol)\nfound(s) :- sample(s), match({}, s).", serde_json::to_string(pattern).unwrap());
        let error = parse(&rule).err().expect("unsupported regex must fail, never report a clean source").to_string();
        assert!(error.contains("regex"), "{error}");
        assert!(!error.contains("named groups"), "lookbehind is not a capture: {error}");
    }
}

#[test]
fn python_visit_order_follows_ast_fields_instead_of_source_offsets() {
    let mut rows = finding_contract(r#"
@decor()
def f(a: ann() = default(), *v: varann(), k: kwann() = kwdefault(), **kw: kwargsann()) -> result():
    body()
x = {key1(): value1(), key2(): value2()}
box(k=keyword(), *(positional(),))
"#, r#"
.policy "contract:python-visit-order" revision 1
.language python
#include "python/ast.rr"
.violation Visitation(at: node, callee: symbol, ordinal: number) "{callee}"
Visitation(c, name, i) :- call_name(c, name), visit_index(c, i).
.violation Before(at: node) code "BEFORE" "body before decorator"
Before(a) :- call_name(a, "body"), call_name(b, "decor"), earlier(a, b).
"#);
    assert_eq!(rows.iter().filter(|r| r["code"] == "BEFORE").count(), 1);
    rows.retain(|r| r["clause"] == "Visitation");
    rows.sort_by_key(|r| r["parameters"]["ordinal"].as_i64().unwrap());
    let names: Vec<_> = rows.iter().map(|r| r["parameters"]["callee"].as_str().unwrap()).collect();
    assert_eq!(names, ["ann", "varann", "kwann", "kwdefault", "kwargsann", "default", "body", "decor", "result",
        "key1", "key2", "value1", "value2", "box", "positional", "keyword"]);
}

#[test]
fn decoded_constant_positions_are_public_typed_rows() {
    let mut rows = finding_contract("x = f\"\"\"json\n{a}\njson\"\"\"\n", r#"
.language python
.violation Piece(at: span, ordinal: number, column: number) "{ordinal}:{column}"
Piece(@place(p, l), i, c) :- str_part_start(n, i, l, c), docof(n, d), doc(d, p, _).
"#);
    rows.sort_by_key(|r| r["parameters"]["ordinal"].as_i64().unwrap());
    assert_eq!(rows.iter().map(|r| (r["line"].as_i64().unwrap(), r["parameters"]["column"].as_i64().unwrap())).collect::<Vec<_>>(), [(1, 8), (2, 3)]);
}

/// Word runs are decoded code points, including non-scalar delimiters and adjacent literals.
#[test]
fn python_constant_word_runs_preserve_surrogate_boundaries() {
    let rows = finding_contract(r#"x = "\ud800json" "_ok\udfffroute_partition"
y = f"json{x}json"
z = "notjson \N{LATIN SMALL LETTER J}son"
"#, r#"
.language python
.violation Word(at: node, ordinal: number, word: symbol) "{word}"
Word(at, i, w) :- str_part_word(_, i, w, at, _).
"#);
    let mut words: Vec<_> = rows.iter().map(|r| (r["line"].as_i64().unwrap(), r["parameters"]["ordinal"].as_i64().unwrap(), r["parameters"]["word"].as_str().unwrap())).collect();
    words.sort();
    assert_eq!(words, [(1, 0, "json_ok"), (1, 0, "route_partition"), (2, 0, "json"), (2, 1, "json"), (3, 0, "json"), (3, 0, "notjson")]);
}

#[test]
fn python_constant_words_use_parsed_interpolation_boundaries() {
    let rows = finding_contract("x = f\"{a:{b:json}}\"\ny = f\"\"\"{a # } : ignored\n}json\"\"\"\nz = \"js\\\r\non\"\r\n", r#"
.language python
.violation Word(at: span, ordinal: number, word: symbol) "{word}"
Word(@place(p, l), i, w) :- str_part_word(n, i, w, _, _), str_part_start(n, i, l, _), docof(n, d), doc(d, p, _).
"#);
    let mut words: Vec<_> = rows.iter().map(|r| (r["line"].as_i64().unwrap(), r["parameters"]["word"].as_str().unwrap())).collect();
    words.sort();
    assert_eq!(words, [(1, "json"), (3, "json"), (4, "json")]);
}

/// Python-only grammar failures must gate AST rules while raw-text rules retain their input.
#[test]
fn native_python_syntax_verdict_keeps_raw_source_available() {
    for source in ["marker = 1\nprint \"legacy\"\n", "marker = 1\n\"\\x\"\n", "marker = t\"future\"\n", "\u{feff}marker = 1\n"] {
        let rows = finding_contract(source, r#"
.language python
.violation ParseFailure(at: span) code "PARSE" "invalid Python"
ParseFailure(@place(p, l)) :- doc(d, p, _), syntax_error(d, l, _).
.violation RawText(at: span) code "RAW" "raw input survives"
RawText(@place(p, 1)) :- doc(d, p, _), source(d, sp), text(sp, t), contains("marker", t).
"#);
        assert_eq!(rows.iter().filter(|r| r["code"] == "RAW").count(), 1, "{source}");
        assert_eq!(rows.iter().filter(|r| r["code"] == "PARSE").count(), 1, "{source}");
    }
}

#[test]
fn public_rr_report_preserves_compiled_policy_metadata() {
    let root = crate::code_rule_dsl::prefilter::tests::temp().with_extension("rr-metadata");
    std::fs::create_dir_all(&root).unwrap();
    let rule = root.join("rule.rr");
    std::fs::write(&rule, r#"
.policy "contract:metadata" revision 7
.mode block
.scope include "impl/**/*.py"
.violation MetadataProbe(at: span) "nothing"
MetadataProbe(@place("input.py", 1)) :- 0 = 1.
"#).unwrap();
    let report = crate::code_rule_dsl::tools::run_rr(&rule, &root, &root.join("out"), &serde_json::Value::Null).unwrap();
    std::fs::remove_dir_all(root).unwrap();
    assert_eq!(report["metadata"]["policies"][0]["id"], "contract:metadata");
    assert_eq!(report["metadata"]["policies"][0]["revision"], 7);
    let settings = report["metadata"]["settings"].as_array().expect("compiled settings");
    assert!(settings.contains(&serde_json::json!(["mode", "block"])));
    assert!(settings.contains(&serde_json::json!(["scope", "include \"impl/**/*.py\""])));
}

/// A public rule's full diagnostic survives the production planner and transport. Distinct witnesses
/// and tags at the same line do not collapse into the old five-column finding index.
#[test]
fn findings_keep_messages_tags_parameters_and_witnesses() {
    let rows = finding_contract("x = y\n", r#"
.policy "contract:diagnostics" revision 1
.language python
.violation Bad(at: node, why: symbol, count: number) code "BAD001" tag {why}
    "{why}:{count}:{at.line}"
Bad(n, "first\n\t\"", 7) :- kind(n, "identifier").
Bad(n, "second", 8) :- kind(n, "identifier").
"#);
    assert_eq!(rows.len(), 4);
    assert_eq!(rows.iter().map(|row| row["at"].as_i64().unwrap()).collect::<BTreeSet<_>>().len(), 2);
    for row in rows {
        assert_eq!(row["policy"], "contract:diagnostics");
        assert_eq!(row["clause"], "Bad");
        assert_eq!(row["code"], "BAD001");
        assert_eq!(row["path"], "a.py");
        assert_eq!(row["line"], 1);
        assert_eq!(row["tag"], row["parameters"]["why"]);
        assert_eq!(row["message"], format!("{}:{}:1", row["tag"].as_str().unwrap(), row["parameters"]["count"].as_i64().unwrap()));
    }
}

/// This producer binds a source span, not a tree node. Per-file memoization must use its document key.
#[test]
fn source_span_producers_retain_rows_through_local_planning() {
    let rows = finding_contract("# marker\nx = y\n", r#"
.policy "contract:source" revision 1
.language python
#include "python/ast.rr"
#include "python/calls.rr"
.decl selected(d: number, t: symbol)
selected(d, t) :- source(d, sp), text(sp, t), contains("marker", t).
.violation Seen(at: node, source_text: symbol) "{source_text}"
Seen(n, t) :- selected(d, t), kind(n, "identifier"), docof(n, d).
"#);
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row["message"] == "# marker\nx = y\n"));
}

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
    // `typing/programs` holds copies of the engine's typing programs (T's dialect, its own inputs): not rules.
    out.retain(|p| !p.starts_with(rule_syntax().join("site")) && !p.starts_with(rule_syntax().join("typing")));
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
    assert!(matches!(&owner[1].body[0], Literal::Atom(a) if a.rel == "rr_inside_rr_kind_module"));
    // A count over two literals reads a helper of every variable, grouped by the ones shared (`_` for the body's
    // own); a min over two, the ones shared and its own.
    let helper = |head: &str| -> Vec<String> {
        let r = p.rules.iter().find(|r| r.head.rel == head).unwrap();
        r.body.iter().filter_map(|l| if let Literal::Agg { atom, .. } = l { Some(format!("{atom}")) } else { None }).collect()
    };
    assert_eq!(helper("kinds"), vec!["rr_agg1(_, $n)".to_owned()]);
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
        (".decl a(x: symbol)\na(x) :- a(x), a(y), !match(\"(?<z>.*)\", y).", "named groups"),
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
    for part in ["$j = @add($i, 1)", "@concat($t, @to_string(@strlen($t)))", "!rr_inside_fdef($c, _)"] {
        assert!(bad.contains(part), "{part} in {bad}");
    }
    assert!(!p.rules.iter().any(|r| r.head.rel == "drop"), "a fix is not run");
    assert!(rule("r").contains("= @cat($s, \"x\"), e($rr_t"), "{}", rule("r"));
}

/// The `.violation` lines the packs use: a constant code, `suppress` lines (settings of the findings layer), a span
/// or place `at`; and an aggregate over an expression parses.
#[test]
fn violation_codes_suppressions_and_places_lower() {
    let p = parse(concat!(
        ".decl node(d: number, n: node, kind: symbol, start: number, end: number)\n.decl doc(d: number, p: symbol, m: symbol)\n",
        ".decl text(n: node, s: symbol)\n.input node, doc, text\n",
        ".violation font(at: node, why: symbol) code \"@next/next/no-page-custom-font\"\n",
        "    \"{why} is discouraged.\"\n",
        "    suppress eslint \"@next/next/no-page-custom-font\"\n",
        "    suppress phpcs \"A.B\", \"A.B.C\",\n        \"A.D\"\n",
        "font(n, w) :- text(n, w).\n",
        ".violation SEC(at: span, c: symbol) code {c} in \"SEC\\d+\" \"{c}\"\n",
        "SEC(n, c) :- text(n, c).\n",
        ".violation missing(at: loc, why: symbol) \"{why}\"\n",
        "missing(@place(p, 1), \"none\") :- doc(_, p, _).\n",
        "missing(n, \"text\") :- text(n, _).\n",
    ))
    .unwrap();
    let rules: Vec<String> = p.rules.iter().filter(|r| r.head.rel == "violation").map(ToString::to_string).collect();
    assert!(rules.iter().any(|r| r.contains("\"@next/next/no-page-custom-font\"") && r.contains("font($rr_v0, _)")), "{rules:?}");
    assert!(rules.iter().any(|r| r.contains("SEC($rr_v0, $rr_v1)") && r.contains("docof($rr_v0, $rr_d)")), "{rules:?}");
    assert!(rules.iter().any(|r| r.contains("rr_place_missing($rr_p, $rr_l, _)")), "{rules:?}");
    assert!(p.rules.iter().any(|r| r.head.rel == "rr_place_missing" && r.head.args[1] == Term::Int(1)));
    let settings: Vec<String> = p.settings.iter().map(|(k, t)| format!("{k} {t}")).collect();
    assert!(settings.contains(&"suppress font eslint \"@next/next/no-page-custom-font\"".to_owned()), "{settings:?}");
    assert!(settings.contains(&"suppress font phpcs \"A.B\", \"A.B.C\", \"A.D\"".to_owned()), "{settings:?}");
    assert!(settings.contains(&"code_in SEC \"SEC\\\\d+\"".to_owned()), "{settings:?}");
    // The printed program reads back as itself.
    let printed = print(&p).unwrap();
    assert_eq!(parse(&printed).unwrap().settings, p.settings, "{printed}");
    let sum = "len(n, k) :- node(_, n, _, _, _), k = sum strlen(x) - strlen(y) : { text(n, x), text(n, y) }.";
    let statements = syntax::Parser::new(sum).unwrap().statements().unwrap();
    assert!(format!("{statements:?}").contains("Agg { op: \"sum\", over: Some(Bin { op: \"-\""), "{statements:?}");
}

/// Soufflé's built-ins run: `cat` of any arity, `max` and `min` of two, `to_number`, `%`, floats (`to_float`,
/// decimals, their arithmetic and order), and an aggregate inside a term.
#[test]
fn souffle_builtins_run() {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("builtins");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "x = 1\ny = 2\n").unwrap();
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl doc(d: number, p: symbol, m: symbol)
.input node, doc
.decl s(t: symbol)
s("12"). s("7"). s("x").
.decl f(t: symbol)
f("1.5"). f("-2.25"). f("1e3").
.decl joined(t: symbol)
joined(cat(a, "-", b, "-", a)) :- s(a), s(b), a != b, a != "x", b != "x".
.decl num(t: symbol, n: number, r: number, big: number, small: number)
num(t, n, n % 5, max(n, 10), min(n, 10)) :- s(t), n = to_number(t).
.decl fl(t: symbol, back: symbol)
fl(t, to_string(to_float(t) * 2.0)) :- f(t), to_float(t) > 0.0.
.decl neg(t: symbol)
neg(t) :- f(t), to_float(t) < 0.
.decl assigns(d: number, k: number)
assigns(d, 1 + count : { node(d, _, "assignment", _, _) }) :- doc(d, _, _).
.decl kv(k: number, v: symbol)
kv(2, "b"). kv(1, "a"). kv(3, "c"). kv(4, "a").
.decl agg(s: symbol, t: number, l: number, e: symbol)
agg(s, t, l, e) :- kv(4, _), s = join v by k sep "," : { kv(k, v) }, t = sum k : { kv(k, _) },
    l = sum strlen(v) * 2 : { kv(_, v) }, e = join v by k sep "," : { kv(k, v), k > 9 }.
.output joined, num, fl, neg, assigns, agg
"#;
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap();
    assert_eq!(read("joined"), "12-7-12\n7-12-7\n");
    assert_eq!(read("num"), "12\t12\t2\t12\t10\n7\t7\t2\t10\t7\n");
    assert_eq!(read("fl"), "1.5\t3\n1e3\t2000\n");
    assert_eq!(read("neg"), "-2.25\n");
    assert!(read("assigns").ends_with("\t3\n"), "{}", read("assigns"));
    assert_eq!(read("agg"), "a,b,c,a\t10\t6\t\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `min`/`max` aggregates and the text functions as native steps (O85): the corpus runner's native engine takes
/// the rules the SQL executor ran before, with the same rows.
#[test]
fn native_aggregates_and_text_functions_run() {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("native-agg");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "x = 1\n").unwrap();
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl doc(d: number, p: symbol, m: symbol)
.input node, doc
.decl kv(k: number, v: number)
kv(1, 5). kv(1, 3). kv(1, 9). kv(2, 4).
.decl ks(k: number, s: symbol)
ks(1, "b"). ks(1, "a"). ks(2, "c").
.decl lo(k: number, v: number)
lo(k, v) :- kv(k, _), v = min x : { kv(k, x) }.
.decl hi(k: number, v: number)
hi(k, v) :- kv(k, _), v = max x : { kv(k, x) }.
.decl one(k: number)
one(k) :- ks(k, _), s = min t : { ks(k, t) }, ks(k, s).
.decl q(p: symbol, k: number)
q("a.b.c", 1). q("a.b.c", 2). q("a", 3).
.decl up(p: symbol, k: number, u: symbol)
up(p, k, u) :- q(p, k), u = @pkg_up(p, k).
.output lo, hi, one, up
"#;
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap();
    assert_eq!(read("lo"), "1\t3\n2\t4\n");
    assert_eq!(read("hi"), "1\t9\n2\t4\n");
    assert_eq!(read("one"), "1\n2\n");
    assert_eq!(read("up"), "a\t3\t\na.b.c\t1\ta.b\na.b.c\t2\ta\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The axes over a relation of nodes (climbs up the tree), and a negated conjunction (a helper relation).
#[test]
fn node_sets_and_negated_conjunctions_run() {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("nodesets");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "def outer():\n    for x in y:\n        def inner():\n            g()\n        f()\nh()\n").unwrap();
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl field(p: node, f: symbol, c: node)
.decl text(n: node, s: symbol)
.input node, field, text
.decl callee(c: node, s: symbol)
callee(c, s) :- node(_, c, "call", _, _), field(c, "function", x), text(x, s).
.decl fname(f: node, s: symbol)
fname(f, s) :- node(_, f, "function_definition", _, _), field(f, "name", x), text(x, s).
// The bodies of for loops, and functions: a relation of nodes.
.decl stop(n: node)
stop(b) :- node(_, l, "for_statement", _, _), field(l, "body", b).
stop(f) :- node(_, f, "function_definition", _, _).
.decl near(c: symbol, k: symbol)
near(c, k) :- callee(n, c), nearest(n, s, stop), node(_, s, k, _, _).
.decl out_of_loop(c: symbol)
out_of_loop(c) :- callee(n, c), !inside(n, _, stop).
.decl fn_with_call(f: symbol)
fn_with_call(f) :- fname(a, f), has(a, n, stop), node(_, n, "block", _, _).
.decl own(c: symbol, f: symbol)
own(c, f) :- callee(n, c), fname(a, f), within_stop(n, a, stop).
.decl lonely(c: symbol)
lonely(c) :- callee(_, c), !(callee(m, c), fname(f, c)), !(c = "h", callee(_, "f")).
.output near, out_of_loop, fn_with_call, own, lonely
"#;
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap();
    assert_eq!(read("near"), "f\tblock\ng\tfunction_definition\n");
    assert_eq!(read("out_of_loop"), "h\n");
    assert_eq!(read("fn_with_call"), "outer\n");
    assert_eq!(read("own"), "g\tinner\n");
    assert_eq!(read("lonely"), "f\ng\n");
    let _ = std::fs::remove_dir_all(&dir);
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
    // The lowering's projections (`lower::projections`, decision 0047): `kind` and `docof` over nodes, read as
    // inputs, are rules over `node`, first.
    let v = |s: &str| front::Term::Var(s.to_owned());
    let mut at = 0;
    for (rel, head, args) in [
        ("kind", [v("n"), v("k")], [front::Term::Wild, v("n"), v("k"), front::Term::Wild, front::Term::Wild]),
        ("docof", [v("n"), v("d")], [v("d"), v("n"), front::Term::Wild, front::Term::Wild, front::Term::Wild]),
    ] {
        if p.inputs.iter().any(|i| i == rel) && p.decls.contains_key("node") {
            p.inputs.retain(|i| i != rel);
            let atom = |rel: &str, args: Vec<front::Term>| front::Atom { rel: rel.to_owned(), args, neg: false, gated: false };
            p.rules.insert(at, front::Rule { head: atom(rel, head.to_vec()), body: vec![Literal::Atom(atom("node", args.to_vec()))] });
            at += 1;
        }
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

/// Writes each authored program's `.rr` beside it (run once per change of an `.rdl`).
#[test]
#[ignore = "writes files: the .rr translations of the authored .rdl programs"]
fn write_rr_translations() {
    for (name, text, path) in authored() {
        std::fs::write(path.with_extension("rr"), translate(&text).unwrap_or_else(|e| panic!("{name}: {e}"))).unwrap();
    }
}

/// Which hand-written data-layer rules lower to the engine today, and what stops the others (a report).
#[test]
#[ignore = "a report: what the hand-written rules need from the lowering"]
fn hand_written_rules_lowering_report() {
    let mut out = Vec::new();
    files(&rule_syntax().join("data-layer"), "rr", &mut out);
    files(&library(), "rr", &mut out);
    for f in out {
        let result = parse_file(&f);
        println!("{}: {}", short(&f), result.map_or_else(|e| e.to_string(), |p| format!("lowers: {} rules", p.rules.len())));
    }
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

/// `must, may` (core.rr 3): two sides, negation reading the other one, and what reads them three-valued too.
#[test]
fn must_may_relations_run() {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("mustmay");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "x = 1\n").unwrap();
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl doc(d: number, p: symbol, m: symbol)
.input node, doc
.decl base(x: symbol)
base("a"). base("b"). base("c").
.decl known(x: symbol)
known("a").
.decl p(x: symbol) must, may
must p(x) :- known(x).
may p(x) :- base(x), !known(x), x != "c".
.decl q(x: symbol)
q(x) :- base(x), !p(x).
// Read positively, p is three-valued in what reads it too (a relation the runner writes: p itself is negated).
.decl seen(x: symbol)
seen(x) :- p(x).
.output seen, q, rr_must_seen, rr_must_q
"#;
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap();
    assert_eq!(read("seen"), "a\nb\n");
    assert_eq!(read("rr_must_seen"), "a\n");
    assert_eq!(read("q"), "b\nc\n");
    assert_eq!(read("rr_must_q"), "c\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `precedes_stop(a, b, S)`: siblings in order with no child of S between them.
#[test]
fn precedes_stop_runs() {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("precstop");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "def f():\n    a()\n    b()\n    return\n    c()\n").unwrap();
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, c: node, i: number)
.decl kind(n: node, k: symbol)
.input node, child, kind
.decl stops(k: symbol)
stops("return_statement").
.decl pairs(x: number, y: number)
pairs(x, y) :- node(_, a, "expression_statement", x, _), node(_, b, "expression_statement", y, _), precedes_stop(a, b, "return_statement").
.decl by_set(x: number, y: number)
by_set(x, y) :- node(_, a, "expression_statement", x, _), node(_, b, _, y, _), precedes_stop(a, b, stops).
.output pairs, by_set
"#;
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap();
    assert_eq!(read("pairs"), "13\t21\n");
    assert_eq!(read("by_set"), "13\t21\n13\t29\n21\t29\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `!nearest(n, s, set)` with s named, two aggregates compared, and `unset` (its text is "").
#[test]
fn negated_nearest_and_compared_aggregates_run() {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("negnearest");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "def outer():\n    def inner():\n        g()\n    f()\n").unwrap();
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl kind(n: node, k: symbol)
.input node, kind
.decl fscope(k: symbol)
fscope("function_definition").
.decl other(c: number, s: number)
other(x, y) :- node(_, c, "call", x, _), node(_, s, "function_definition", y, _), !nearest(c, s, fscope).
.decl same(x: number)
same(x) :- node(d, _, "module", x, _), count : { node(d, _, "call", _, _) } = count : { node(d, _, "function_definition", _, _) }.
.decl text(n: node, s: symbol)
.decl unset(x: node)
.input text
.decl empty(x: number, t: symbol)
empty(x, t) :- node(_, _, "module", x, _), unset(v), text(v, t).
.decl given(x: number)
given(x) :- node(_, n, "call", x, _), !unset(n).
.output other, same, empty, given
"#;
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap();
    assert_eq!(read("other"), "38\t0\n46\t17\n");
    assert_eq!(read("same"), "0\n");
    assert_eq!(read("empty"), "0\t\n");
    assert_eq!(read("given"), "38\n46\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Source scanners use str.splitlines(), while AST positions use physical lines.
/// Keep both contracts, including long lines, empty lines and CRLF as one separator.
#[test]
fn python_split_lines_are_lossless_and_document_local() {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("splitlines");
    std::fs::create_dir_all(&dir).unwrap();
    let long = "x".repeat(400);
    let src = format!("{long}\u{000b}two\u{000c}\u{001c}four\u{001d}five\u{001e}six\u{0085}seven\u{2028}eight\u{2029}nine\r\nten\releven\n");
    std::fs::write(dir.join("a.py"), src).unwrap();
    std::fs::write(dir.join("b.py"), "short").unwrap();
    std::fs::write(dir.join("empty.py"), "").unwrap();
    let program = r#"
.decl seen(p: symbol, l: number, t: symbol)
seen(p, l, t) :- doc(d, p, _), split_line(d, l, t).
.output seen
"#;
    let policy = dir.join("lines.rr");
    std::fs::write(&policy, program).unwrap();
    let program = crate::code_rule_dsl::tools::read_rr(&policy).unwrap();
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let actual = std::fs::read_to_string(out.join("seen.csv")).unwrap();
    let lines = [&long, "two", "", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven"];
    let mut expected: Vec<String> = lines.iter().enumerate().map(|(i, t)| format!("a.py\t{}\t{t}", i + 1)).collect();
    expected.push("b.py\t1\tshort".to_owned());
    let mut rows: Vec<&str> = actual.lines().collect();
    rows.sort();
    expected.sort();
    assert_eq!(rows, expected);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Empty scopes still run global absence checks; they must not become a planning error or an early return.
#[test]
fn empty_corpora_run_global_absence_findings() {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("emptyrr");
    let corpus = dir.join("corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    let policy = dir.join("rule.rr");
    std::fs::write(&policy, r#"
.policy "empty-contract" revision 1
.violation Missing(at: span) code "MISSING" "required file is absent"
Missing(@place("required.py", 1)) :- !doc(_, "required.py", _).
"#).unwrap();
    let result = crate::code_rule_dsl::tools::run_rr(&policy, &corpus, &dir.join("out"), &serde_json::Value::Null).unwrap();
    assert_eq!(result["files"], 0);
    assert_eq!(result["diagnostics"].as_array().unwrap().len(), 1);
    assert_eq!(result["diagnostics"][0]["code"], "MISSING");
    let _ = std::fs::remove_dir_all(dir);
}

/// Direct scanner APIs select buffers, including hidden paths and nonstandard extensions.
#[test]
fn explicit_input_files_preserve_the_callers_document_batch() {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("rrinputs");
    let root = dir.join("corpus");
    std::fs::create_dir_all(root.join(".hidden")).unwrap();
    std::fs::write(root.join(".hidden/a.txt"), "hidden text").unwrap();
    std::fs::write(root.join("included.py"), "included text").unwrap();
    std::fs::write(root.join("ignored.py"), "ignored text").unwrap();
    let policy = dir.join("rule.rr");
    std::fs::write(&policy, r#"
.policy "explicit-inputs" revision 1
.language python
.violation Seen(at: span, t: symbol) "{t}"
Seen(@place(p, l), t) :- doc(d, p, _), split_line(d, l, t).
"#).unwrap();
    let result = crate::code_rule_dsl::tools::run_rr(&policy, &root, &dir.join("out"),
        &serde_json::json!({"inputFiles": [".hidden/a.txt", "included.py"]})).unwrap();
    let mut paths: Vec<&str> = result["diagnostics"].as_array().unwrap().iter().map(|r| r["path"].as_str().unwrap()).collect();
    paths.sort();
    assert_eq!(paths, [".hidden/a.txt", "included.py"]);
    assert_eq!(result["files"], 2);
    let _ = std::fs::remove_dir_all(dir);
}

/// async_executor uses str.split('\n'), not physical lines or Python splitlines.
#[test]
fn lf_lines_keep_other_separators_and_the_trailing_empty_line() {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("rrlf");
    let root = dir.join("corpus");
    std::fs::create_dir_all(&root).unwrap();
    let first = format!("{}\rtwo\u{2028}three\u{b}four", "x".repeat(400));
    std::fs::write(root.join("a.py"), format!("{first}\nfive\n")).unwrap();
    std::fs::write(root.join("empty.py"), "").unwrap();
    let policy = dir.join("rule.rr");
    std::fs::write(&policy, r#"
.policy "lf-lines" revision 1
.language python
.violation Seen(at: span, text: symbol) "{text}"
Seen(@place(p, l), t) :- doc(d, p, _), lf_line(d, l, t).
"#).unwrap();
    let result = crate::code_rule_dsl::tools::run_rr(&policy, &root, &dir.join("out"), &serde_json::Value::Null).unwrap();
    let mut actual: Vec<(String, u64, String)> = result["diagnostics"].as_array().unwrap().iter()
        .map(|r| (r["path"].as_str().unwrap().to_owned(), r["line"].as_u64().unwrap(), r["parameters"]["text"].as_str().unwrap().to_owned())).collect();
    actual.sort();
    assert_eq!(actual, vec![("a.py".into(),1,first), ("a.py".into(),2,"five".into()),
                           ("a.py".into(),3,"".into()), ("empty.py".into(),1,"".into())]);
}

/// Existing allowlist bytes must verify through the public reader and relational execution path.
#[test]
fn signed_allowlist_functor_runs_through_public_rules() {
    let rows = finding_contract("", r#"
.policy "contract:signature" revision 1
.decl candidate(name: symbol, message: symbol)
candidate("original", "{\"allowlist\":[],\"linter\":\"lint_admin_async_purity\",\"version\":1}").
candidate("tampered", "{\"allowlist\":[\"added.py\"],\"linter\":\"lint_admin_async_purity\",\"version\":1}").
.violation Verified(at: span, name: symbol) "{name}"
Verified(@place("allowlist.json", 1), name) :- candidate(name, message),
    @ed25519_ok("7e9776d87292ab633115bd6c07892a4bf56450fffb2dfcf13b3eba1866b9fbb6", message,
    "xN3ugqWFKXrfdfYHOWH14S9NfLPHCwnni7yf1errzrn17xHvVgd6C3y/QXR7mwfGPnFwpuq/HcCWAx9C2crzDg==") = 1.
"#);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["parameters"]["name"], "original");
}

/// One repository document can feed several typed selectors without rebuilding per rule or source file.
#[test]
fn repository_json_selectors_preserve_typed_rows_and_wildcard_keys() {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("rrjson");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("policy.json"), r#"{"counts":{"a":2,"b":3},"groups":{"g":{"entries":[{"name":"one","limit":4},{"name":"two","limit":5}]}},"title":"kept"}"#).unwrap();
    let rule = dir.join("rule.rr");
    std::fs::write(&rule, r#"
.policy "contract:json" revision 1
.decl counts(k: symbol, n: number)
.input counts(IO=json, filename = "policy.json", path="/counts")
.decl entries(group: symbol, name: symbol, limit: number)
.input entries(IO=json, filename="policy.json", path="/groups/*/entries", records=true)
.decl title(t: symbol)
.input title(IO=json, filename="policy.json", path="/title")
.violation Row(at: span, family: symbol, value: symbol, n: number) "{family}:{value}:{n}"
Row(@place("policy.json", 1), "count", k, n) :- counts(k, n).
Row(@place("policy.json", 1), g, name, n) :- entries(g, name, n).
Row(@place("policy.json", 1), "title", t, 0) :- title(t).
"#).unwrap();
    let report = crate::code_rule_dsl::tools::run_rr(&rule, &dir, &dir.join("out"), &serde_json::Value::Null).unwrap();
    let mut actual: Vec<_> = report["diagnostics"].as_array().unwrap().iter().map(|r| r["message"].as_str().unwrap()).collect();
    actual.sort();
    assert_eq!(actual, ["count:a:2", "count:b:3", "g:one:4", "g:two:5", "title:kept:0"]);
}

/// A malformed row must not leak an earlier prefix of a policy input into rule execution.
#[test]
fn repository_inputs_report_missing_and_malformed_without_partial_rows() {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("rrinputerrors");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("bad.json"), r#"[1,"wrong type"]"#).unwrap();
    let rule = dir.join("rule.rr");
    std::fs::write(&rule, r#"
.policy "contract:input-errors" revision 1
.decl bad(n: number)
.input bad(IO=json, filename="bad.json")
.decl missing(t: symbol)
.input missing(IO=lines, filename="missing.txt")
.violation Error(at: span, kind: symbol) "{kind}"
Error(@place(p, 1), kind) :- input_error(p, kind, _).
.violation Leaked(at: span) "partial row leaked"
Leaked(@place("bad.json", 1)) :- bad(_).
"#).unwrap();
    let report = crate::code_rule_dsl::tools::run_rr(&rule, &dir, &dir.join("out"), &serde_json::Value::Null).unwrap();
    let mut actual: Vec<_> = report["diagnostics"].as_array().unwrap().iter().map(|r| (r["path"].as_str().unwrap(), r["message"].as_str().unwrap())).collect();
    actual.sort();
    assert_eq!(actual, [("bad.json", "malformed"), ("missing.txt", "missing")]);
}

#[test]
fn shared_signing_library_verifies_an_existing_empty_allowlist() {
    let rows = finding_contract("", r#"
.policy "contract:shared-signature" revision 1
#include "data-layer/signing.rr"
allow_sig("original", "lint_admin_async_purity", "xN3ugqWFKXrfdfYHOWH14S9NfLPHCwnni7yf1errzrn17xHvVgd6C3y/QXR7mwfGPnFwpuq/HcCWAx9C2crzDg==").
allow_sig("tampered", "lint_admin_async_purity", "xN3ugqWFKXrfdfYHOWH14S9NfLPHCwnni7yf1errzrn17xHvVgd6C3y/QXR7mwfGPnFwpuq/HcCWAx9C2crzDg==").
allow_entry("tampered", "added.py").
allow_sig("unsigned", "lint_admin_async_purity", "UNSIGNED").
.violation Status(at: span, name: symbol, status: symbol) "{name}:{status}"
Status(@place("allowlist.json", 1), name, status) :- allow_status(name, status).
"#);
    let mut actual: Vec<_> = rows.iter().map(|r| r["message"].as_str().unwrap()).collect();
    actual.sort();
    assert_eq!(actual, ["original:valid", "tampered:invalid", "unsigned:unsigned"]);
}

/// `@ed25519_ok` through a rule: an existing signature verifies, a tampered message does not.
#[test]
fn signatures_verify_through_rules() {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("signatures");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "x = 1\n").unwrap();
    let program = r#"
.decl candidate(name: symbol, message: symbol)
candidate("original", "{\"allowlist\":[],\"linter\":\"lint_admin_async_purity\",\"version\":1}").
candidate("tampered", "{\"allowlist\":[\"added.py\"],\"linter\":\"lint_admin_async_purity\",\"version\":1}").
.decl verified(name: symbol)
verified(name) :- candidate(name, message),
    @ed25519_ok("7e9776d87292ab633115bd6c07892a4bf56450fffb2dfcf13b3eba1866b9fbb6", message,
    "xN3ugqWFKXrfdfYHOWH14S9NfLPHCwnni7yf1errzrn17xHvVgd6C3y/QXR7mwfGPnFwpuq/HcCWAx9C2crzDg==") = 1.
.output verified
"#;
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    assert_eq!(std::fs::read_to_string(out.join("verified.csv")).unwrap(), "original\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `@tree_digest(n)` compares structural trees: no positions, comments or punctuation, a
/// parenthesized expression as what it holds, literals by value. Equal right-hand sides pair up; others do not.
#[test]
fn tree_digests_compare_subtrees_without_positions() {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("digest");
    std::fs::create_dir_all(&dir).unwrap();
    let source = "a = (1, 2)\nb = (1, 2,)\nc = 'x'\nd = \"x\"\ne = 16\nf = 0x10\ng = (h)\nh = h  # note\ni = [1, 2]\n";
    std::fs::write(dir.join("a.py"), source).unwrap();
    let rule = dir.join("digest.rr");
    std::fs::write(&rule, r#"
.decl rhs(l: symbol, r: node)
rhs(l, r) :- field(a, "left", x), text(x, l), field(a, "right", r), kind(a, "assignment").
.decl same(l: symbol, m: symbol)
same(l, m) :- rhs(l, r), rhs(m, s), l < m, @tree_digest(r) = @tree_digest(s).
.output same
"#).unwrap();
    let program = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    assert_eq!(std::fs::read_to_string(out.join("same.csv")).unwrap(), "a\tb\nc\td\ne\tf\ng\th\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `match_all` and `match` with captures (`core.rr`): over a `source` or `src_line` span each group is a span, with
/// its text and line (`rx_span`); over a symbol, `match_all` gives a row per match with its groups' text.
#[test]
fn match_all_captures_spans_over_source_text_and_symbols_over_symbols() {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("matchall");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "x = 1  # SEC1 SEC22\nclass Foo:\n    pass\nclass Bar: pass\n").unwrap();
    let rule = dir.join("m.rr");
    std::fs::write(&rule, r#"
.decl sec(l: number, c: symbol, k: number)
sec(l, c, k) :- src_line(d, l, sp), match_all("(?<m>SEC\\d+)", sp), text(m, c), line(m, k).
.decl cls(x: symbol, k: number)
cls(x, k) :- source(d, sp), match_all("(?m)^class\\s+(?<c>\\w+)", sp), text(c, x), line(c, k).
.decl whole(x: symbol, k: number)
whole(x, k) :- src_line(d, k, sp), match("x = (?<v>\\d+).*", sp), text(v, x).
.decl sv(s: symbol)
sv("a=1 b=22 c=x").
sv("none here").
.decl kv(k: symbol, v: symbol)
kv(k, v) :- sv(s), match_all("(?<k>[a-z]+)=(?<v>[0-9]+)", s).
.decl hasb(s: symbol)
hasb(s) :- sv(s), match_all("b=", s).
.decl nob(s: symbol)
nob(s) :- sv(s), !match_all("b=", s).
.decl st(d: number, sp: span)
st(d, sp) :- source(d, sp).
.decl via(x: symbol)
via(x) :- st(_, sp), match_all("(?m)^class\\s+(?<c>\\w+)", sp), text(c, x).
.output sec, cls, whole, kv, hasb, nob, via
.decl hit(m: span, c: symbol)
hit(m, c) :- src_line(d, _, sp), match_all("(?<m>SEC\\d+)", sp), text(m, c).
.violation NQ(at: span, c: symbol) code {c} "{c} suppression"
NQ(m, c) :- hit(m, c).
"#).unwrap();
    let program = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| {
        let mut rows: Vec<String> = std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap().lines().map(str::to_owned).collect();
        rows.sort();
        rows.join("\n")
    };
    assert_eq!(read("sec"), "1\tSEC1\t1\n1\tSEC22\t1");
    assert_eq!(read("cls"), "Bar\t4\nFoo\t2");
    assert_eq!(read("whole"), "1\t1");
    assert_eq!(read("kv"), "a\t1\nb\t22");
    assert_eq!(read("hasb"), "a=1 b=22 c=x");
    assert_eq!(read("nob"), "none here");
    assert_eq!(read("via"), "Bar\nFoo");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `data-layer/noqa-sec-ratchet.rr`, whose `match_all` over each line's span is what lowering refused (O80): every
/// SEC code on a suppression line counts, inside the comment or not.
#[test]
fn noqa_sec_ratchet_counts_every_sec_code_on_suppression_lines() {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("noqa");
    std::fs::create_dir_all(dir.join("impl")).unwrap();
    std::fs::write(dir.join("impl/a.py"), "x = \"SEC1\"  # noqa: SEC2\ny = 1  # noqa: E501, SEC3 SEC44\nz = \"SEC9\"\n").unwrap();
    let rule = crate::code_rule_dsl::logic::rr::library().join("../data-layer/noqa-sec-ratchet.rr");
    let program = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    assert_eq!(std::fs::read_to_string(out.join("annotations.csv")).unwrap(), "noqa_sec_annotations\t4\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Error directives report rejected syntax while raw-source consumers keep their document views.
#[test]
fn syntax_error_directives_preserve_raw_source_relations() {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("withheld");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("good.py"), "x = 1\n").unwrap();
    std::fs::write(dir.join("bad.py"), "y = 2\ndef (:\n").unwrap();
    let rule = dir.join("w.rr");
    std::fs::write(&rule, r#"
.policy "t:withheld" revision 1
.on syntax_error report BAD000 at line 1
.violation ASSIGN(at: node) "an assignment"
ASSIGN(n) :- kind(n, "assignment"), docof(n, d), !syntax_error(d, _, _).
.decl lines(d: number, k: number)
lines(d, k) :- line_count(d, k).
.decl err(d: number)
err(d) :- syntax_error(d, _, _).
.output lines, err
"#).unwrap();
    let program = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap();
    let mut v: Vec<String> = read("violation").lines().map(str::to_owned).collect();
    v.sort();
    assert_eq!(v, ["t:withheld\tASSIGN\tASSIGN\tgood.py\t1", "t:withheld\tsyntax_error\tBAD000\tbad.py\t1"]);
    assert_eq!(read("lines").lines().count(), 2, "raw-text policies retain both documents");
    assert_eq!(read("err").lines().count(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A run's outputs do not depend on how its files are chunked: one chunk, or chunks of a few files whose mix (small
/// files or large ones) changes from chunk to chunk, each mix measured for itself, give the same rows, cross-file
/// ones too.
#[test]
fn outputs_do_not_depend_on_the_chunk_size() {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("chunks");
    std::fs::create_dir_all(&dir).unwrap();
    for i in 0..6 {
        std::fs::write(dir.join(format!("m{i}.py")), format!("def f{i}(x):\n    return g{}(x) + len(x)\n\ndef g{i}(y):\n    return f{i}(y)\n", (i + 1) % 6)).unwrap();
    }
    for j in 0..2 {
        let body: String = (1..30).map(|k| format!("def b{j}_{k}():\n    return b{j}_{}()\n\n", k - 1)).collect();
        std::fs::write(dir.join(format!("big{j}.py")), format!("def b{j}_0():\n    return 0\n\n{body}")).unwrap();
    }
    let rule = dir.join("c.rr");
    std::fs::write(&rule, r#"
.decl defined(x: symbol)
defined(x) :- kind(n, "function_definition"), field(n, "name", m), text(m, x).
.decl called(x: symbol, p: symbol)
called(x, p) :- kind(c, "call"), field(c, "function", f), text(f, x), defined(x), docof(c, d), doc(d, p, _).
.violation UNKNOWN(at: node, x: symbol) "call of {x}"
UNKNOWN(c, x) :- kind(c, "call"), field(c, "function", f), text(f, x), !defined(x).
.output called
"#).unwrap();
    let program = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
    let run = |chunk_nodes: usize| {
        let out = dir.join(format!("out{chunk_nodes}"));
        crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, chunk_nodes).unwrap();
        ["called", "violation"].map(|rel| {
            let mut rows: Vec<String> = std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap().lines().map(str::to_owned).collect();
            rows.sort();
            rows
        })
    };
    let whole = run(1 << 18);
    assert_eq!(whole[0].len(), 12 + 2 * 29, "every call of a defined function: {:?}", whole[0]);
    assert_eq!(whole[1].len(), 6, "len, once per Python file: {:?}", whole[1]);
    for chunk_nodes in [60, 400] {
        assert_eq!(run(chunk_nodes), whole, "chunks of {chunk_nodes} nodes");
    }
    // Every chunk's batch (its facts) is released when the run returns: no binding keeps one.
    let mut files: Vec<String> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()?.file_name().into_string().ok()).filter(|f| f.ends_with(".py")).collect();
    files.sort();
    let keys: std::collections::BTreeSet<u32> = crate::code_rule_dsl::logic::plan::corpus::memo::doc_keys(&files).into_iter().map(|k| k as u32).collect();
    let batches = crate::code_rule_dsl::facts::vtab::syntax::BATCHES.lock();
    let ours: Vec<_> = batches.iter().filter(|(k, _)| k.iter().any(|k| keys.contains(k))).collect();
    assert!(ours.len() >= 6, "the runs' batches were recorded: {}", ours.len());
    assert!(ours.iter().all(|(_, b)| b.upgrade().is_none()), "a released batch is still held");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `dfs_first` and `dfs_first_memo` through rules: the first target by edge order from each root, the ord of the
/// root's edge that starts the path, and the memo variant over its roots in order.
#[test]
fn dfs_first_runs_through_rules() {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("dfs");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "x = 1\n").unwrap();
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl doc(d: number, p: symbol, m: symbol)
.input node, doc
.decl e(a: number, b: number, o: number)
e(10, 20, 1). e(20, 30, 5). e(10, 40, 0). e(40, 10, 2). e(50, 60, 0).
.decl t(x: number)
t(30).
.decl roots(r: number, k: number)
roots(40, 1). roots(10, 2). roots(50, 3).
.decl hit(r: number, h: number, f: number)
hit(r, h, f) :- dfs_first(r, h, f, e, t).
.decl hm(r: number, h: number, f: number)
hm(r, h, f) :- dfs_first_memo(r, h, f, e, t, roots).
.decl none(r: number)
none(r) :- e(r, _, _), !dfs_first(r, _, _, e, t).
.output hit, hm, none
"#;
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap();
    assert_eq!(read("hit"), "10\t30\t1\n20\t30\t5\n40\t30\t2\n");
    assert_eq!(read("hm"), "10\t30\t1\n40\t30\t2\n");
    assert_eq!(read("none"), "50\n");
    let _ = std::fs::remove_dir_all(&dir);
}


#[test]
fn error_directives_survive_public_report_transport() {
    let rows = finding_contract("def (:\n", r#"
.policy "contract:error-transport" revision 1
.language python
.on syntax_error report BAD000 at line 1
"#);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["code"], "BAD000");
    assert_eq!(rows[0]["path"], "a.py");
    assert_eq!(rows[0]["line"], 1);
}


#[test]
fn input_error_directives_survive_public_report_transport() {
    let rows = finding_contract("", r#"
.policy "contract:input-error-transport" revision 1
.decl items(s: symbol)
.input items(IO=lines, filename="missing.txt")
.on input_error report NEED
"#);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["code"], "NEED");
    assert_eq!(rows[0]["path"], "missing.txt");
    assert_eq!(rows[0]["line"], 1);
}

#[test]
fn nullary_relations_preserve_boolean_set_semantics() {
    let rows = finding_contract("import example\n", r#"
.language python
.decl enabled()
enabled() :- doc(_, _, _).
.decl absent()
absent() :- kind(_, "not_a_real_kind").
.decl always()
always().
.decl marker(x: number)
marker(1).
marker(2).
.decl many()
many() :- marker(_).
.violation Boolean(at: span) code "BOOLEAN" "true"
Boolean(@place(p, 1)) :- doc(_, p, _), enabled(), !absent(), always(), many().
.violation Bad(at: span) code "BAD" "false"
Bad(@place(p, 1)) :- doc(_, p, _), absent().
.violation Count(at: span, n: number) code "COUNT" "{n}"
Count(@place("a.py", 1), n) :- n = count : { many() }.
"#);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows.iter().filter(|row| row["code"] == "BOOLEAN").count(), 1);
    assert_eq!(rows.iter().find(|row| row["code"] == "COUNT").unwrap()["parameters"]["n"], 1);
}

#[test]
fn dynamic_regex_patterns_preserve_whole_string_matching_and_roundtrip() {
    let rule = r#"
.language python
.decl pattern(id: number, re: symbol)
pattern(0, "cache").
pattern(1, "(?i)cache").
.decl sample(id: number, s: symbol)
sample(0, "cache").
sample(1, "notcache").
sample(2, "CACHE").
.violation Found(at: span, pattern_id: number, sample_id: number) code "MATCH" "found"
Found(@place("a.py", 1), p, i) :- pattern(p, re), sample(i, s), match(re, s).
.violation Missing(at: span, pattern_id: number, sample_id: number) code "MISS" "missing"
Missing(@place("a.py", 1), p, i) :- pattern(p, re), sample(i, s), !match(re, s).
"#;
    let parsed = parse(rule).unwrap();
    let printed = print(&parsed).unwrap();
    let roundtrip = parse(&printed).unwrap();
    assert_eq!(roundtrip.rules, parsed.rules, "{printed}");
    assert_eq!(roundtrip.facts, parsed.facts, "{printed}");
    let rows = finding_contract("", rule);
    let mut matches: Vec<_> = rows.iter().filter(|row| row["code"] == "MATCH")
        .map(|row| (row["parameters"]["pattern_id"].as_i64().unwrap(), row["parameters"]["sample_id"].as_i64().unwrap())).collect();
    matches.sort();
    assert_eq!(matches, [(0, 0), (1, 0), (1, 2)]);
    assert_eq!(rows.iter().filter(|row| row["code"] == "MISS").count(), 3);
}

#[test]
fn requested_source_and_line_text_remain_exact_without_copying_large_subtrees() {
    let source = format!("marker = 1 # {}\npass\n", "é".repeat(5000));
    let rows = finding_contract(&source, r#"
.language python
.decl source_copy(sp: span)
source_copy(sp) :- source(_, sp).
.violation Whole(at: span, body: symbol) code "SOURCE" "source"
Whole(sp, body) :- source_copy(sp), text(sp, body).
.violation Physical(at: span, body: symbol) code "LINE" "line"
Physical(sp, body) :- src_line(_, _, sp), text(sp, body).
.violation Subtree(at: node) code "TREE" "whole AST text"
Subtree(n) :- kind(n, "module"), text(n, _).
"#);
    assert_eq!(rows.iter().filter(|row| row["code"] == "SOURCE").count(), 1);
    assert_eq!(rows.iter().find(|row| row["code"] == "SOURCE").unwrap()["parameters"]["body"], source);
    let mut lines: Vec<_> = rows.iter().filter(|row| row["code"] == "LINE")
        .map(|row| (row["line"].as_i64().unwrap(), row["parameters"]["body"].as_str().unwrap())).collect();
    lines.sort();
    assert_eq!(lines, [(1, source.split('\n').next().unwrap()), (2, "pass")]);
    assert!(!rows.iter().any(|row| row["code"] == "TREE"), "a requested source span must not enable all large AST texts");
}

/// Decision 0055's guests: a run of `rule` over `files`, each output's sorted rows.
fn guests_run(name: &str, files: &[(&str, &str)], rule: &str, outputs: &[&str]) -> Vec<Vec<String>> {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension(name);
    std::fs::create_dir_all(&dir).unwrap();
    for (path, text) in files {
        std::fs::write(dir.join(path), text).unwrap();
    }
    let path = dir.join("g.rr");
    std::fs::write(&path, rule).unwrap();
    let program = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&path).unwrap()).unwrap();
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let rows = outputs
        .iter()
        .map(|rel| {
            let mut rows: Vec<String> = std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap_or_default().lines().map(str::to_owned).collect();
            rows.sort();
            rows
        })
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    rows
}

/// A string annotation is parsed where it stands: its names have the file's lines and columns when the string is
/// plain, and the string's place when it is escaped or concatenated; a string inside a parsed one is parsed in the
/// next round. Bytes give no guest.
#[test]
fn string_annotations_are_guests_at_their_host_lines() {
    let py = "x: \"List[int]\" = []\n\ndef f(y: 'Dict[str, Foo]') -> \"Bar\": ...\nz: \"A\\x42\" = 1\nw: (\"Li\" \"st\") = 2\nq: \"Opt['Baz']\" = 3\nb: b\"Nope\" = 4\n";
    let rule = r#"
.decl ident(x: symbol, l: number, c: number)
ident(x, l, c) :- kind(s, "string"), parsed(s, "annotation", _), guest(g, _, s, _), node(g, i, "identifier", _, _), text(i, x), line(i, l), col(i, c).
.decl whole(x: symbol, l: number, c: number)
whole(x, l, c) :- kind(s, "concatenated_string"), parsed(s, "annotation", _), guest(g, _, s, _), node(g, i, "identifier", _, _), text(i, x), line(i, l), col(i, c).
.decl roots(k: symbol)
roots(k) :- kind(s, "string"), parsed(s, "annotation", r), kind(r, k).
.output ident, whole, roots
"#;
    let [names, whole, roots] = <[Vec<String>; 3]>::try_from(guests_run("annotations", &[("a.py", py)], rule, &["ident", "whole", "roots"])).unwrap();
    assert!(names.contains(&"List\t1\t4".to_owned()), "{names:?}");
    assert!(names.contains(&"int\t1\t9".to_owned()), "{names:?}");
    assert!(names.contains(&"Foo\t3\t20".to_owned()), "{names:?}");
    assert!(names.contains(&"Bar\t3\t31".to_owned()), "{names:?}");
    // "A\x42" is AB, at the string.
    assert!(names.contains(&"AB\t4\t3".to_owned()), "{names:?}");
    // ("Li" "st"): each part is a string of its own, and the concatenation is List, at the concatenation.
    assert!(names.contains(&"Li\t5\t5".to_owned()), "{names:?}");
    assert_eq!(whole, ["List\t5\t4"]);
    assert_eq!(roots, ["identifier", "subscript"]);
    // 'Baz' inside "Opt['Baz']": round 2.
    assert!(names.contains(&"Baz\t6\t9".to_owned()), "{names:?}");
    assert!(!names.iter().any(|n| n.starts_with("Nope")), "{names:?}");
}

/// `.on guest_error`: a guest that does not parse is withheld (skip, the default) or keeps its partial tree, and
/// `report` gives a finding at its line in the host; the host's own `.on syntax_error` does not see it.
#[test]
fn guests_that_do_not_parse_follow_the_policy() {
    let py = "x: \"int\" = 1\ny: \"List[\" = 2\n";
    let rule = |on: &str| {
        format!(
            r#"
.policy "t:guest" revision 1
.on syntax_error report BAD000
{on}
.decl roots(l: number)
roots(l) :- kind(s, "string"), parsed(s, "annotation", r), line(r, l).
.decl kept(l: number)
kept(l) :- kind(s, "string"), guest(_, _, s, "annotation"), line(s, l).
.output roots, kept
"#
        )
    };
    let [v, roots, kept] = <[Vec<String>; 3]>::try_from(guests_run("gskip", &[("a.py", py)], &rule(".on guest_error annotation report GBAD"), &["violation", "roots", "kept"])).unwrap();
    assert_eq!(v, ["t:guest\tguest_error\tGBAD\ta.py\t2"]);
    assert_eq!(roots, ["1"], "the broken guest has no tree");
    assert_eq!(kept, ["1", "2"], "both are guests");
    let [v, roots] = <[Vec<String>; 2]>::try_from(guests_run("gpartial", &[("a.py", py)], &rule(".on guest_error partial"), &["violation", "roots"])).unwrap();
    assert!(v.is_empty(), "{v:?}");
    assert_eq!(roots, ["1", "2"], "the broken guest keeps its partial tree");
}

/// A library's `embed` and `embed_value` declare guests of a role; `.guests off` leaves them out. `sub_tape` gives
/// a guest's tape: equal texts share one. A rule's own request is made either way.
#[test]
fn embeds_declare_guests_and_guests_off_leaves_them_out() {
    let py = "a = \"import os\"\nb = \"import os\"\nc = 1\n";
    let rule = |guests: &str| {
        format!(
            r#"
{guests}
embed(d, s, "python", "code") :- kind(s, "string"), docof(s, d).
embed_value(d, n, "python", "made", "import sys") :- kind(n, "integer"), docof(n, d).
.decl imp(r: symbol, x: symbol, l: number)
imp(r, x, l) :- doc_role(g, r), node(g, i, "import_statement", _, _), text(i, x), line(i, l).
.decl tapes(t: symbol)
tapes(t) :- kind(s, "string"), sub_tape(s, "python", t).
.output imp, tapes
"#
        )
    };
    let [imp, tapes] = <[Vec<String>; 2]>::try_from(guests_run("embed", &[("a.py", py)], &rule(""), &["imp", "tapes"])).unwrap();
    assert_eq!(imp, ["code\timport os\t1", "code\timport os\t2", "made\timport sys\t3"]);
    assert_eq!(tapes.len(), 1, "{tapes:?}");
    let [imp] = <[Vec<String>; 1]>::try_from(guests_run("embedoff", &[("a.py", py)], &rule(".guests off"), &["imp"])).unwrap();
    // The rule's own `sub_tape` still asks for the strings, as fragments.
    assert_eq!(imp, ["fragment\timport os\t1", "fragment\timport os\t2"]);
}

/// Python-style finditer consumers need every occurrence and code-point offsets, even for equal captures.
#[test]
fn match_all_symbol_positions_keep_duplicate_matches_and_unicode_offsets() {
    let rule = r#"
.language python
.decl sample(s: symbol)
sample("é xx xx").
.violation Found(at: span, ordinal: number, start: number, end: number, capture: symbol) code "MATCH" "found"
Found(@place("a.py", 1), i, a, b, capture) :- sample(s), match_all("(?<capture>xx)", s, i, a, b).
.violation Bare(at: span, ordinal: number, start: number, end: number) code "BARE" "found"
Bare(@place("a.py", 1), i, a, b) :- sample(s), match_all("x", s, i, a, b).
"#;
    let wanted = |rows: &[serde_json::Value], code: &str| {
        let mut positions: Vec<_> = rows.iter().filter(|r| r["code"] == code).map(|r| {
            let p = &r["parameters"];
            (p["ordinal"].as_i64().unwrap(), p["start"].as_i64().unwrap(), p["end"].as_i64().unwrap())
        }).collect();
        positions.sort();
        positions
    };
    let rows = finding_contract("", rule);
    assert_eq!(wanted(&rows, "MATCH"), [(0, 2, 4), (1, 5, 7)]);
    assert_eq!(wanted(&rows, "BARE"), [(0, 2, 3), (1, 3, 4), (2, 5, 6), (3, 6, 7)]);
    assert!(rows.iter().filter(|r| r["code"] == "MATCH").all(|r| r["parameters"]["capture"] == "xx"));
    let printed = complete_rule(rule);
    let roundtrip = lowered_findings(&printed);
    assert_eq!(wanted(&roundtrip, "MATCH"), wanted(&rows, "MATCH"));
    assert_eq!(wanted(&roundtrip, "BARE"), wanted(&rows, "BARE"));
}

/// Coalescing may rename a shared context, but a native producer must still read that context's rows.
#[test]
fn native_regex_inputs_follow_program_namespaces_and_shared_contexts() {
    let rule = |code: &str, sample: &str, pattern: &str| format!(r#"
.language python
.decl sample(s: symbol)
sample("{sample}").
.violation Found(at: span, ordinal: number) code "{code}" "found"
Found(@place("a.py", 1), i) :- sample(s), match_all("{pattern}", s, i, _, _).
"#);
    let a = complete_rule(&rule("A", "xx xx", "xx"));
    let b = complete_rule(&rule("B", "yy yy", "yy"));
    let combined = crate::code_rule_dsl::logic::plan::corpus::combine(&[&a, &b], "").unwrap();
    assert_eq!(combined.natives.len(), 2);
    for native in combined.natives.values() {
        for input in native.reads() { assert!(combined.decls.contains_key(input), "{input}"); }
    }
    let rows = lowered_findings(&print(&combined).unwrap());
    assert_eq!(rows.iter().filter(|r| r["code"] == "A").count(), 2, "{rows:?}");
    assert_eq!(rows.iter().filter(|r| r["code"] == "B").count(), 2, "{rows:?}");
}

/// Printed engine programs use reserved helper names; run through the internal text boundary used by run_rr.
fn complete_rule(program: &str) -> String {
    let path = crate::code_rule_dsl::prefilter::tests::temp().with_extension("public.rr");
    std::fs::write(&path, program).unwrap();
    let (program, _) = read_file(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    print(&program).unwrap()
}

fn lowered_findings(program: &str) -> Vec<serde_json::Value> {
    let root = crate::code_rule_dsl::prefilter::tests::temp().with_extension("lowered-findings");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.py"), "").unwrap();
    let out = root.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(program, &root, None, &out, "program",
        crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let rows = std::fs::read_to_string(out.join("rr_diagnostic.csv")).unwrap().lines()
        .map(|line| serde_json::from_str(line).unwrap()).collect();
    std::fs::remove_dir_all(root).unwrap();
    rows
}
