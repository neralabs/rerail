//! The `.rr` front end: the rule-syntax files parse and pass `check_rr.py`'s checks, the spellings lower to the
//! engine's forms, and every `.rdl` program translates to `.rr` that lowers to the same program.

use super::*;
use crate::code_rule_dsl::logic::front::{self, Literal, Term};
use std::collections::BTreeSet;

#[test]
fn include_seed_ignores_quoted_directives_and_preserves_real_includes() {
    let quoted = parse(r##".decl x(s:symbol)
x("#include \"absent.rr\"").
// #include "absent.rr"
"##).unwrap();
    assert_eq!(quoted.facts["x"].len(),1);
    let included = parse("#include \"core.rr\"\n.decl answer(n:node)\nanswer(n) :- kind(n, \"call\").\n").unwrap();
    assert!(included.decls.contains_key("doc"), "a real include still supplies its declarations");
    assert!(parse("#unknown \"x.rr\"\n").is_err());
    assert!(parse(".decl broken(\n").is_err(), "the lexical shortcut does not bypass compilation errors");
}

fn finding_contract(source: &str, program: &str) -> Vec<serde_json::Value> {
    let root = crate::code_rule_dsl::fixtures::temp().with_extension("rr-contract");
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
    let root = crate::code_rule_dsl::fixtures::temp().with_extension("rr-metadata");
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

/// O111: `{{` and `}}` are braces of a message's text, as the converter writes them; a lone `}` stays text.
#[test]
fn doubled_braces_in_a_message_are_text() {
    let rows = finding_contract("x = y\n", r#"
.policy "contract:braces" revision 1
.language python
.violation Lit(at: node, why: symbol) "use `{{k: v for ...}}` for {why} }"
Lit(n, "dicts") :- kind(n, "identifier").
"#);
    assert!(!rows.is_empty());
    assert!(rows.iter().all(|row| row["message"] == "use `{k: v for ...}` for dicts }"), "{rows:?}");
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
    // `packs/python-main` is generated from all of main's rules (`site/regen_main_pack.sh`), the ones the reader
    // refuses too, so a test over every rule of main sees them all; its own check is `reader-check.txt`.
    let generated = rule_syntax().join("packs/python-main");
    out.retain(|p| !p.starts_with(rule_syntax().join("site")) && !p.starts_with(rule_syntax().join("typing")) && !p.starts_with(&generated));
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
    // Phase 0 resolves every file's includes in one run of `bootstrap.rr`, not one run each.
    let seeds: Vec<_> = rr_files().into_iter().map(|f| crate::code_rule_dsl::logic::plan::corpus::phase0::Seed::File(f.canonicalize().unwrap())).collect();
    crate::code_rule_dsl::logic::plan::corpus::phase0::graphs(&seeds).unwrap();
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
        // A relation read and declared nowhere in the file's program, as check-rr and a run refuse it.
        problems.extend(check::undeclared(&all).into_iter().map(|p| format!("{}: {p}", short(&f))));
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
    // `m = @dot(q, x)` splits a bound `m` at each dot, binding both parts; an unbound `m` is named.
    let split = statements(".decl a(m: symbol)\n.decl b(q: symbol)\nb(q) :- a(m), m = @dot(q, x).\nb(q) :- m = @dot(q, x), a(m).\nb(q) :- a(q), m = @dot(q, x).\n");
    assert_eq!(check::grounded(&split, &BTreeSet::new()), vec![(5, vec!["m".to_owned()])]);
    // So does `m = @cat(q, x)`, at each position (the front end's `cmp_binds`).
    let cat = statements(".decl a(m: symbol)\n.decl b(q: symbol)\nb(q) :- a(m), m = @cat(q, x).\nb(q) :- a(q), m = @cat(q, x).\n");
    assert_eq!(check::grounded(&cat, &BTreeSet::new()), vec![(4, vec!["m".to_owned()])]);
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
    // A count over two literals reads a helper of every variable, its `_`s named (each row counts), grouped by the
    // ones shared (`_` for the body's own); a min over two, the ones shared and its own.
    let helper = |head: &str| -> Vec<String> {
        let r = p.rules.iter().find(|r| r.head.rel == head).unwrap();
        r.body.iter().filter_map(|l| if let Literal::Agg { atom, .. } = l { Some(format!("{atom}")) } else { None }).collect()
    };
    assert_eq!(helper("kinds"), vec!["rr_agg5(_, $n, _, _, _, _)".to_owned()]);
    assert_eq!(p.decls["rr_agg5"].len(), 6);
    let first = helper("first");
    assert_eq!(first.len(), 2);
    assert!(first[0].contains("$j") && first[0].contains("$n") && !first[0].contains("$x"), "{first:?}");
    let best_rule = p.rules.iter().find(|r| r.head.rel == "best").unwrap();
    assert_eq!(best_rule.head.args[1], call("cat", vec![s("a"), v("x")]));
    assert_eq!(p.rules.iter().find(|r| r.head.rel == "cv").unwrap().head.args[1], Term::Top);
}

/// An aggregate's context leaves out the clause's own head where the rest binds its variables: through it, the
/// aggregate would sit in the head's recursion (lazy-analyzer.rr's `la_cb`).
#[test]
fn a_context_leaves_out_the_clause_head() {
    let p = parse(concat!(
        ".decl e(n: node, x: symbol, s: number)\n.decl seed(n: node)\n.input e, seed\n",
        ".decl cb(n: node)\n.output cb\n",
        "cb(n) :- seed(n).\n",
        "cb(n) :- e(n, x, s), cb(n), count : { e(n, x, s2), s2 > s } = 0.\n",
    ))
    .unwrap();
    front::strata(&p).unwrap_or_else(|e| panic!("{e}"));
    let ctx: Vec<String> = p.rules.iter().filter(|r| r.head.rel.starts_with("rr_ctx")).map(|r| r.to_string()).collect();
    assert!(ctx.iter().all(|r| !r.contains("cb(")), "{ctx:?}");
}

/// A context keeps a group of literals that ties two asked variables, even where other groups bind each:
/// pyi047's `unread_comp_var`, asked for c, v and x, binds c by `in_stub(a), within(c, a)` and v, x by `kind(v, …),
/// text(v, x)`. Only `child(c, fc, _), field(fc, "left", t), (v = t ; within(v, t))` ties c to v. Left out, the
/// context is every node of a stub file times every identifier (122 M rows on the ruff fixtures).
#[test]
fn a_context_keeps_the_group_tying_its_variables() {
    let p = parse(concat!(
        ".decl within(n: node, a: node)\n.decl kind(n: node, k: symbol)\n.decl text(n: node, x: symbol)\n",
        ".decl child(p: node, c: node, i: number)\n.decl field(p: node, f: symbol, c: node)\n.decl in_stub(n: node)\n",
        ".input within, kind, text, child, field, in_stub\n",
        ".decl unread(a: node, v: node, x: symbol)\n.output unread\n",
        "unread(a, v, x) :- in_stub(a), within(c, a), child(c, fc, _), kind(fc, \"for_in_clause\"),\n",
        "    field(fc, \"left\", t), (v = t ; within(v, t)), kind(v, \"identifier\"), text(v, x),\n",
        "    count : { within(r, c), kind(r, \"identifier\"), text(r, x), r != v } = 0.\n",
    ))
    .unwrap();
    let ctx: Vec<String> = p.rules.iter().filter(|r| r.head.rel.starts_with("rr_ctx")).map(|r| r.to_string()).collect();
    assert!(!ctx.is_empty(), "{:#?}", p.rules.iter().map(|r| r.to_string()).collect::<Vec<_>>());
    // Every rule of the context reaches v from c through the comprehension's target t.
    assert!(ctx.iter().all(|r| r.contains("child($c, $fc") && r.contains("field($fc, \"left\", $t)")), "{ctx:#?}");
}

/// A helper's context (here the demand of strings `parsed` parses) keeps only the literals tied to its variables:
/// `root(a)`, tied to `s` only through a disjunction the context leaves out, would test that some root exists in any document (`ann_root(_)`), making
/// every row depend on every file.
#[test]
fn a_context_leaves_out_literals_not_tied_to_its_variables() {
    let p = parse(concat!(
        ".decl root(t: node)\n.decl within(n: node, a: node)\n.decl kind(n: node, k: symbol)\n",
        ".input root, within, kind\n",
        ".decl hit(e: node, s: node)\n.output hit\n",
        "hit(r, s) :- root(a), (s = a ; within(s, a)), kind(s, \"string\"), parsed(s, \"annotation\", r).\n",
    ))
    .unwrap();
    front::strata(&p).unwrap_or_else(|e| panic!("{e}"));
    let ctx: Vec<String> = p.rules.iter().filter(|r| r.head.rel.starts_with("rr_ctx")).map(|r| r.to_string()).collect();
    assert!(!ctx.is_empty(), "the parse's demand has a context");
    assert!(ctx.iter().all(|r| !r.contains("root(")), "{ctx:?}");
}

/// A context's second group binding only what the first binds still narrows it, joined once a binding: as a
/// relation of the asked variable, not left out (every row the first group gives) and not joined whole (every pair
/// of the two groups' rows, pyi041's `within(c1, n), within(c2, n)`).
#[test]
fn a_context_joins_a_later_group_by_what_it_binds() {
    let p = parse(concat!(
        ".decl within(n: node, a: node)\n.decl kind(n: node, k: symbol)\n",
        ".input within, kind\n",
        ".decl hit(e: node, s: node)\n.output hit\n",
        "hit(r, s) :- within(c1, s), kind(c1, \"call\"), within(c2, s), kind(c2, \"name\"), parsed(s, \"annotation\", r).\n",
    ))
    .unwrap();
    front::strata(&p).unwrap_or_else(|e| panic!("{e}"));
    let shown = |stem: &str| -> Vec<String> { p.rules.iter().filter(|r| r.head.rel.starts_with(stem)).map(|r| r.to_string()).collect() };
    let (ctx, cover) = (shown("rr_ctx"), shown("rr_cover"));
    assert!(!ctx.is_empty(), "the parse's demand has a context");
    assert!(ctx.iter().all(|r| r.contains("\"call\"") && r.contains("rr_cover") && !r.contains("\"name\"")), "{ctx:?}");
    assert!(!cover.is_empty() && cover.iter().all(|r| r.contains("\"name\"") && !r.contains("\"call\"")), "{cover:?}");
    assert!(p.decls.iter().any(|(rel, cols)| rel.starts_with("rr_cover") && cols.len() == 1), "a relation of s alone");
}

/// A group that only computes from an asked variable (`jb(p - 1, 92)`) does not bind it: covered alone it would be
/// `rr_cover(p) :- t = @sub(p, 1), jb(t, 92)`, which cannot place `p` (serverplan-state-tripwires' `jbs_odd`).
#[test]
fn a_context_covers_no_group_that_only_computes_from_an_asked_variable() {
    let p = parse(concat!(
        ".decl jb(p: number, c: number)\n.input jb\n",
        ".decl odd(p: number)\n.output odd\n",
        "odd(p) :- jb(p, _), jb(p - 1, 92), m = max j : { jb(j, 92), j < p }, m > 0.\n",
    ))
    .unwrap();
    front::strata(&p).unwrap_or_else(|e| panic!("{e}"));
    let cover: Vec<String> = p.rules.iter().filter(|r| r.head.rel.starts_with("rr_cover")).map(|r| r.to_string()).collect();
    assert!(cover.iter().all(|r| r.contains("jb(p,")), "a cover binds what it is a relation of: {cover:?}");
}

/// A later group tying two asked variables through one not asked (c414's `same_keywords`: `double(c, s), call_nkwargs(c,
/// n), call_nkwargs(s, n)`, counted) is a cover narrowed by the group that binds them, not every pair with the same `n`.
#[test]
fn a_cover_of_two_variables_is_narrowed_by_what_binds_them() {
    let p = parse(concat!(
        ".decl double(c: node, s: node)\n.decl nk(c: node, n: number)\n.decl kw(s: node, x: symbol)\n.decl km(c: node, s: node, x: symbol)\n",
        ".input double, nk, kw, km\n",
        ".decl same(c: node, s: node)\n.output same\n",
        "same(c, s) :- double(c, s), nk(c, n), nk(s, n), count : { kw(s, x), !km(c, s, x) } = 0.\n",
    ))
    .unwrap();
    front::strata(&p).unwrap_or_else(|e| panic!("{e}"));
    let cover: Vec<String> = p.rules.iter().filter(|r| r.head.rel.starts_with("rr_cover")).map(|r| r.to_string()).collect();
    assert!(!cover.is_empty() && cover.iter().all(|r| r.contains("double(")), "{cover:?}");
}

/// Tests of one span in a disjunction, folded into one set test, test its text as a single test does
/// (bootstrap-engine-invariants' `source(d, sp), (contains("a", sp) ; contains("b", sp))`).
#[test]
fn a_set_of_tests_on_a_span_tests_its_text() {
    let p = parse(concat!(
        ".decl source(d: number, sp: span)\n.decl text(n: loc, s: symbol)\n.input source, text\n",
        ".decl hit(d: number)\n.output hit\n",
        "hit(d) :- source(d, sp), (contains(\"a\", sp) ; contains(\"b\", sp)).\n",
    ))
    .unwrap();
    let rule = p.rules.iter().find(|r| r.head.rel == "hit").unwrap().to_string();
    assert!(rule.contains("text($sp, "), "{rule}");
    assert!(!rule.contains("contains($sp") && !rule.contains("@contains($sp"), "{rule}");
}

/// A negated match with named groups binds nothing, so it lowers with its groups non-capturing (it was the last
/// construct refused as not lowered yet).
#[test]
fn negated_named_groups_lower() {
    let src = ".decl a(x: symbol)\na(x) :- a(x), a(y), !match(\"(?<z>.*)\", y).";
    parse(src).expect("a negated match with named groups lowers");
}

/// A negated match's named groups are plain groups.
#[test]
fn a_negated_match_reads_its_named_groups_as_groups() {
    // A negated match binds nothing, so its named groups are plain groups: `!match("(?<z>.*)", y)` is
    // `!match("(?:.*)", y)` (it was refused as "named groups … not lowered yet").
    let p = parse(".decl a(x: symbol)\na(x) :- a(x), a(y), !match(\"(?<z>.*)\", y).").unwrap();
    let text = format!("{p:?}");
    assert!(text.contains("(?:.*)") && !text.contains("(?<z>"), "{text}");
}

/// What is not lowered yet says so by name. A `must` head needs its relation declared `must, may`; a `.violation`
/// at a span or loc whose values can be spans the engine does not make lowers (such a row has no `docof`, and so no
/// finding).
#[test]
fn unlowered_constructs_are_named() {
    for (src, what) in [
        (".decl a(x: number)\na(x) :- a(y), x = mean z : { a(z) }.", "mean"),
    ] {
        let e = parse(src).unwrap_err().to_string();
        assert!(e.contains("not lowered yet") && e.contains(what), "{src}: {e}");
    }
    let e = parse(".decl a(x: number)\nmust a(1) :- a(2).").unwrap_err().to_string();
    assert!(e.contains("`must` head a") && e.contains("not declared `must, may`"), "{e}");
    for src in [
        ".decl t(s: symbol)\n.input t\n.violation v(at: span) \"m\"\nv(m) :- t(s), match(\"(?<m>x+)\", s).",
        ".decl first_token(n: node, t: span)\n.input first_token\n.violation v(at: loc) \"m\"\nv(t) :- first_token(_, t).",
    ] {
        parse(src).unwrap_or_else(|e| panic!("{src}: {e}"));
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
    assert_eq!(rule("violation"), "violation(\"data-layer:p\", \"BAD\", $rr_v1, $rr_p, $rr_l, $rr_c) :- BAD($rr_v0, $rr_v1), node($rr_d, $rr_v0, _, _, _), doc($rr_d, $rr_p, _), line($rr_v0, $rr_l), col($rr_v0, $rr_c)");
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
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("builtins");
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
.decl prod(p: number, none: number)
prod(p, e) :- kv(4, _), p = product k : { kv(k, _) }, e = product k : { kv(k, _), k > 9 }.
.output joined, num, fl, neg, assigns, agg, prod
"#;
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap();
    assert_eq!(read("joined"), "12-7-12\n7-12-7\n");
    assert_eq!(read("num"), "12\t12\t2\t12\t10\n7\t7\t2\t10\t7\n");
    assert_eq!(read("fl"), "1.5\t3\n1e3\t2000\n");
    assert_eq!(read("neg"), "-2.25\n");
    assert!(read("assigns").ends_with("\t3\n"), "{}", read("assigns"));
    // `sum strlen(v) * 2 : { kv(_, v) }` sums over kv's four rows, a twice (Soufflé's tuples): 8.
    assert_eq!(read("agg"), "a,b,c,a\t10\t8\t\n");
    assert_eq!(read("prod"), "24\t1\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `min`/`max` aggregates and the text functions as native steps (O90): the corpus runner's native engine takes
/// the rules the SQL executor ran before, with the same rows.
#[test]
fn native_aggregates_and_text_functions_run() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("native-agg");
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
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("nodesets");
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
    let rename = |a: &mut front::Atom| a.rel = r(&a.rel).into();
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
            let atom = |rel: &str, args: Vec<front::Term>| front::Atom { rel: rel.into(), args, neg: false, gated: false };
            p.rules.insert(at, front::Rule { head: atom(rel, head.to_vec()), body: vec![Literal::Atom(atom("node", args.to_vec()))] });
            at += 1;
        }
    }
    p
}

/// A lowered `.rr` program without the requests its lowering derives (`rr_embed_req` for `parsed`, decision 0055;
/// `rr_parse_req`, `rr_splice_req` for tape operations, 0058;
/// `rr_dir_req`, `rr_file_req` for the file system, t21) and the context relations only they read (a context's
/// `rr_ctx` and the `rr_cover` groups it joins): an `.rdl` program
/// asks for nothing, and the requests change no relation the program derives.
fn without_requests(mut p: front::Program) -> front::Program {
    let mut gone: BTreeSet<String> = ["rr_embed_req", "rr_parse_req", "rr_splice_req"].into_iter().map(str::to_owned).chain(super::WORLD.iter().map(|(_, d)| (*d).to_owned())).collect();
    loop {
        p.rules.retain(|r| !gone.contains(r.head.rel.as_str()));
        let read: BTreeSet<&str> = p.rules.iter().flat_map(|r| &r.body).filter_map(|l| l.atom()).map(|a| a.rel.as_str()).collect();
        let more: Vec<String> = p.rules.iter().map(|r| r.head.rel.to_string()).filter(|h| (h.starts_with("rr_ctx") || h.starts_with("rr_cover")) && !read.contains(h.as_str()) && !gone.contains(h.as_str())).collect();
        if more.is_empty() {
            break;
        }
        gone.extend(more);
    }
    p.decls.retain(|k, _| !gone.contains(k));
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
        // The program's own statements: an `#include` passes through the translation as it is (checked above), and
        // what it names is read with the file (`parse_file`), not here.
        let own: String = rr.lines().map(|l| if l.starts_with("#include") { "" } else { l }).collect::<Vec<_>>().join("\n");
        match (front::parse(&text).map(normalized), parse(&own)) {
            (Ok(a), Ok(b)) => {
                failures.extend(difference(&a, &without_requests(b.clone())).map(|d| format!("{name}: {d}")));
                // Printed, the program reads back as itself.
                match print(&b).and_then(|text| parse(&text)) {
                    Ok(c) => failures.extend(difference(&without_requests(b), &without_requests(c)).map(|d| format!("{name}, printed: {d}"))),
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
    let types: Vec<&str> = p.decls[atom.rel.as_str()].iter().map(|(_, t)| t.as_str()).collect();
    assert_eq!(types, ["node", "int", "int", "sym"], "{atom}");
    let (op, over, atom) = agg("g");
    let over = over.unwrap();
    assert!(op == front::AggOp::Sum && over.starts_with("rr_val"), "{over}");
    let cols = &p.decls[atom.rel.as_str()];
    assert_eq!(cols.len(), 4, "c, t, the named `_` and the value: {cols:?}");
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
/// engine's functions of those names; each prints and reads back as itself. Floats are floats' keys (`float_key`):
/// `to_float` and a decimal number lower; `cat` of one stays refused by name.
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
    let (src, what) = (".decl a(x: symbol)\na(x) :- a(y), x = cat(y).", "cat/1");
    let e = parse(src).unwrap_err().to_string();
    assert!(e.contains("not lowered yet") && e.contains(what), "{src}: {e}");
    // Floats lower; `y + 1.5` over `a` itself would be an unbounded value cycle (`front::cycles`), so `y` is `t`'s.
    for src in [".decl a(x: number)\na(x) :- a(y), x = to_float(\"1\").", ".decl t(y: number)\n.decl a(x: number)\na(x) :- t(y), x = y + 1.5."] {
        parse(src).unwrap_or_else(|e| panic!("{src}: {e}"));
    }
}

// ---- violations ----

/// A `.violation`'s constant code is the finding's code; its `suppress` and `fix` lines are settings of the
/// findings layer; a `loc` column gives each row its finding: a node or span at its `docof` document and line, a
/// `@place` value at its path and line, and a head `V(@place(p, l), …)` through V's place relation; and the program
/// prints as itself.
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
            "violation(\"p\", \"V\", \"@scope/a-b\", $rr_p, $rr_l, $rr_c) :- V($rr_v0, _), docof($rr_v0, $rr_d), doc($rr_d, $rr_p, _), line($rr_v0, $rr_l), col($rr_v0, $rr_c)",
            "violation(\"p\", \"V\", \"@scope/a-b\", @place_path($rr_v0), @place_line($rr_v0), 0) :- V($rr_v0, _), $rr_v0 < -1",
            "violation(\"p\", \"V\", \"@scope/a-b\", $rr_p, $rr_l, 0) :- rr_place_V($rr_p, $rr_l, _)",
        ]
    );
    // A span column: its rows' documents, and the `@place` values a rule made.
    assert_eq!(
        rules("W"),
        [
            "violation(\"p\", \"W\", \"W\", $rr_p, $rr_l, $rr_c) :- W($rr_v0), docof($rr_v0, $rr_d), doc($rr_d, $rr_p, _), line($rr_v0, $rr_l), col($rr_v0, $rr_c)",
            "violation(\"p\", \"W\", \"W\", @place_path($rr_v0), @place_line($rr_v0), 0) :- W($rr_v0), $rr_v0 < -1",
        ]
    );
    let printed = print(&p).unwrap();
    let again = parse(&printed).unwrap();
    assert_eq!(difference(&p, &again), None, "{printed}");
    assert_eq!(p.settings, again.settings);
}

// ---- tree axes and node sets ----

/// `has(a, n, set)` is the descendant axis, the set tested on n; a set of nodes is read as climbs up the tree
/// (`node_axis`), not a kind set; the forms print as themselves.
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
    assert_eq!(rules[0].body, [Literal::Atom(front::Atom { rel: "rr_has_fn".to_owned().into(), args: vec![v("n"), v("a")], neg: false, gated: false })]);
    assert!(!p.kindsets.contains_key("blk") && p.kindsets.contains_key("fn"));
    // The nearest block: a climb from n that stops at the first member of blk, which is a.
    assert!(rules[1].body.iter().any(|l| matches!(l, Literal::Atom(x) if x.rel.starts_with("rr_up") && x.args == [v("n"), v("a")])), "{}", rules[1]);
    assert!(rules[1].body.iter().any(|l| l.to_string() == "blk($a)"), "{}", rules[1]);
    // `next_sibling` is core.rr's, written out over `child` in the rule (`rr::lower::core`): no relation of the program.
    assert!(!p.rules.iter().any(|r| r.head.rel == "next_sibling") && !p.inputs.contains(&"next_sibling".to_owned()));
    let again = parse(&print(&p).unwrap()).unwrap();
    assert_eq!(difference(&p, &again), None);
}

/// What the axes still refuse says so by name; core.rr's `within`, `earlier` and `precedes_stop` negate with a node
/// unnamed, through their relations: n has no ancestor, n has nothing after it, n has no later sibling before a stop.
#[test]
fn axes_refuse_by_name() {
    let decls = ".decl node(d: number, n: node, kind: symbol, start: number, end: number)\n.decl tape(n: number, par: number, size: number, depth: number)\n.input node, tape\n.decl a(n: node, m: node)\n.decl q(s: symbol)\n.decl fn(kind: symbol)\nfn(\"f\").\n";
    for (body, what) in [
        ("a(n, n) :- a(n, _), !within_stop(n, _, fn).", "`!within_stop` with its ancestor unnamed"),
        ("q(s) :- q(s), !split_dot(s, _, \"x\").", "`!split_dot` with a column unnamed"),
    ] {
        let e = parse(&format!("{decls}{body}")).unwrap_err().to_string();
        assert!(e.contains("not lowered yet") && e.contains(what), "{body}: {e}");
    }
    for body in ["a(n, n) :- a(n, _), !within(n, _).", "a(n, n) :- a(n, _), !earlier(n, _).", "a(n, n) :- a(n, _), !precedes_stop(n, _, fn)."] {
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
                    a.to_string().replace(&*a.rel, "H"),
                    rules(&a.rel).iter().map(|r| r.to_string().replace(&*a.rel, "H")).collect(),
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
    // The edit comes in a later run: a run reads its files as they were when it first looked (decision 0105).
    std::fs::write(&helper, ".decl h(x: number)\nh(2).\n").unwrap();
    crate::code_rule_dsl::logic::plan::corpus::observe(|o| o.flush());
    let (edited, edited_files) = read_file(&main).unwrap();
    assert_ne!(format!("{first:?}"), format!("{edited:?}"), "an edited include is read again");
    assert_ne!(files, edited_files);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// `must, may` (core.rr 3): two sides, negation reading the other one, and what reads them three-valued too.
#[test]
fn must_may_relations_run() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("mustmay");
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
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("precstop");
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
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("negnearest");
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
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("splitlines");
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
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("emptyrr");
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
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("rrinputs");
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

/// A policy's run holds the program `run-rr` read (`corpus::run_read`), unless the root's settings add `env` facts to
/// its text: then the run holds that text's program, which reads them. The same policy, without settings and then
/// with them, in one process.
#[test]
fn a_policy_read_once_still_reads_the_roots_settings() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("rrenv");
    let root = dir.join("corpus");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.py"), "x = 1\n").unwrap();
    let policy = dir.join("rule.rr");
    std::fs::write(&policy, r#"
.policy "settings-read" revision 1
.language python
.violation Seen(at: span, m: symbol) "{m}"
Seen(@place(p, l), m) :- doc(d, p, _), split_line(d, l, _), env("mark", m).
"#).unwrap();
    let marks = |out: &str| -> Vec<String> {
        let result = crate::code_rule_dsl::tools::run_rr(&policy, &root, &dir.join(out), &serde_json::Value::Null).unwrap();
        result["diagnostics"].as_array().unwrap().iter().map(|r| r["parameters"]["m"].as_str().unwrap().to_owned()).collect()
    };
    assert!(marks("plain").is_empty(), "no settings, no mark");
    std::fs::write(root.join("rerail.env"), "mark = here\n").unwrap();
    assert!(!marks("set").is_empty() && marks("again").iter().all(|m| m == "here"), "the setting read");
    let _ = std::fs::remove_dir_all(dir);
}

/// async_executor uses str.split('\n'), not physical lines or Python splitlines.
#[test]
fn lf_lines_keep_other_separators_and_the_trailing_empty_line() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("rrlf");
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
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("rrjson");
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

/// `.language typescript` names a language of three dialects (`.ts`, `.tsx`, `.js`): run-rr runs the policy over
/// each one's files in scope and reports them together. It read the `.ts` files alone before.
#[test]
fn a_language_of_several_dialects_runs_over_each() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("rrdialects");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.ts"), "let a = 1;\n").unwrap();
    std::fs::write(dir.join("b.js"), "\nlet b = 2;\n").unwrap();
    std::fs::write(dir.join("c.tsx"), "\n\nlet c = <p/>;\n").unwrap();
    std::fs::write(dir.join("d.py"), "d = 1\n").unwrap();
    let rule = dir.join("rule.rr");
    std::fs::write(&rule, r#"
.policy "contract:dialects" revision 1
.language typescript
.scope include "**/*"
.violation Let(at: node, name: symbol) "{name}"
Let(d, x) :- kind(d, "variable_declarator"), field(d, "name", n), text(n, x).
"#).unwrap();
    let report = crate::code_rule_dsl::tools::run_rr(&rule, &dir, &dir.join("out"), &serde_json::Value::Null).unwrap();
    let mut actual: Vec<_> = report["diagnostics"].as_array().unwrap().iter().map(|r| r["message"].as_str().unwrap()).collect();
    actual.sort();
    assert_eq!(actual, ["a", "b", "c"]);
    assert_eq!(report["files"], 3);
    // Scoped to `.js` alone, the one dialect with files is JavaScript, not the TypeScript the policy names.
    let js = dir.join("js.rr");
    std::fs::write(&js, std::fs::read_to_string(&rule).unwrap().replace(".scope include \"**/*\"", ".scope include \"**/*.js\"")).unwrap();
    let report = crate::code_rule_dsl::tools::run_rr(&js, &dir, &dir.join("out-js"), &serde_json::Value::Null).unwrap();
    let actual: Vec<_> = report["diagnostics"].as_array().unwrap().iter().map(|r| r["message"].as_str().unwrap()).collect();
    assert_eq!(actual, ["b"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A malformed row must not leak an earlier prefix of a policy input into rule execution.
#[test]
fn repository_inputs_report_missing_and_malformed_without_partial_rows() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("rrinputerrors");
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
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("signatures");
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
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("digest");
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

/// `def_digest(n, h)` (decision 0083 §0) hashes each definition unit's own code: the module, each class and each
/// function, a nested unit standing for its kind and name. A comment or a moved line changes nothing; a method's body
/// changes the method's digest alone, not its class's or its module's.
#[test]
fn def_digests_hash_each_definitions_own_code() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("defdigest");
    let digests = |source: &str| -> std::collections::BTreeMap<String, String> {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.py"), source).unwrap();
        let rule = dir.join("d.rr");
        std::fs::write(&rule, r#"
.decl unit(k: symbol, s: symbol)
unit("<module>", s) :- def_digest(n, s), kind(n, "module").
unit(x, s) :- def_digest(n, s), field(n, "name", i), text(i, x).
.output unit
"#).unwrap();
        let program = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
        let out = dir.join("out");
        crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        std::fs::read_to_string(out.join("unit.csv")).unwrap().lines().map(|l| { let (k, s) = l.split_once('\t').unwrap(); (k.to_owned(), s.to_owned()) }).collect()
    };
    let base = digests("import os\n\nclass C:\n    def m(self):\n        return 1\n\n    def n(self):\n        return 2\n\ndef f():\n    pass\n");
    assert_eq!(base.keys().map(String::as_str).collect::<Vec<_>>(), ["<module>", "C", "f", "m", "n"]);
    // A comment and blank lines: nothing.
    let commented = digests("# top\nimport os\n\n\nclass C:\n    def m(self):  # here\n        return 1\n\n    def n(self):\n        return 2\n\ndef f():\n    pass\n");
    assert_eq!(commented, base);
    // A body edit in `m`: only `m`.
    let edited = digests("import os\n\nclass C:\n    def m(self):\n        x = 1\n        return x\n\n    def n(self):\n        return 2\n\ndef f():\n    pass\n");
    let changed: Vec<&str> = base.iter().filter(|(k, v)| edited.get(*k) != Some(*v)).map(|(k, _)| k.as_str()).collect();
    assert_eq!(changed, ["m"]);
    // A new function: the module's own code names it, so the module and the new unit.
    let added = digests("import os\n\nclass C:\n    def m(self):\n        return 1\n\n    def n(self):\n        return 2\n\ndef f():\n    pass\n\ndef g():\n    pass\n");
    let changed: Vec<&str> = added.iter().filter(|(k, v)| base.get(*k) != Some(*v)).map(|(k, _)| k.as_str()).collect();
    assert_eq!(changed, ["<module>", "g"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Node ids per definition (decision 0083 §3, `RERAIL_NODE_IDS=definition`): a line added in one function's body
/// renumbers that function's nodes only, where preorder ids renumber every node after it. Counted over `kind` and
/// `parent` rows as output (`path:id`); run once under each scheme (the scheme is the process's).
#[test]
fn definition_ids_keep_other_definitions_rows_across_a_body_edit() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("defids");
    let rows = |source: &str| -> std::collections::BTreeSet<String> {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.py"), source).unwrap();
        let rule = dir.join("d.rr");
        std::fs::write(&rule, ".decl k(n: node, s: symbol)\nk(n, s) :- kind(n, s).\n.decl p(n: node, q: node, i: number)\np(n, q, i) :- child(n, q, i).\n.output k, p\n").unwrap();
        let program = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
        let out = dir.join("out");
        crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        ["k", "p"].iter().flat_map(|r| std::fs::read_to_string(out.join(format!("{r}.csv"))).unwrap().lines().map(|l| format!("{r} {l}")).collect::<Vec<_>>()).collect()
    };
    let body = |extra: &str| format!("import os\n\ndef f(a):\n    {extra}return a + 1\n\nclass C:\n    def m(self, x):\n        return [x, x * 2]\n\n    def n(self):\n        return {{1: 2}}\n\ndef g():\n    return os.path.join('a', 'b')\n");
    let before = rows(&body(""));
    let after = rows(&body("y = 2\n    "));
    let changed = before.symmetric_difference(&after).count();
    let definition = crate::code_rule_dsl::facts::vtab::syntax::definition_ids();
    eprintln!("node ids {}: {} rows before, {} after, {changed} differ", if definition { "per definition" } else { "preorder" }, before.len(), after.len());
    if definition {
        // `f`'s own nodes from the new statement on, and nothing of `C`, `m`, `n`, `g` or the module's import.
        assert!(changed <= 30, "{changed} rows differ");
    } else {
        assert!(changed > before.len() / 2, "{changed} rows differ");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// `match_all` and `match` with captures (`core.rr`): over a `source` or `src_line` span each group is a span, with
/// its text and line (`rx_span`); over a symbol, `match_all` gives a row per match with its groups' text.
#[test]
fn match_all_captures_spans_over_source_text_and_symbols_over_symbols() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("matchall");
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

/// A text test of a capture in the body that captured it tests the capture's text: `match_all("(?<w>…)", t),
/// match("…", w)` over a string's content (RUF001's words). It once tested the span's id as a text, and never held.
#[test]
fn a_text_test_of_a_capture_in_its_own_body_tests_the_captured_text() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("capturetest");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "a = \"x\u{211d}y plain\"\nb = \"2\u{211d}6789\"\n").unwrap();
    let rule = dir.join("m.rr");
    std::fs::write(&rule, r#"
.decl piece(t: loc)
piece(p) :- kind(p, "string_content").
.decl wide(w: symbol)
wide(x) :- piece(t), match_all("(?<w>[\\p{Alphabetic}\\p{N}]+)", t), match("[\\s\\S]*[^\\x00-\\x7f][\\s\\S]*", w), text(w, x).
.decl odd(w: symbol)
odd(x) :- piece(t), match_all("(?<w>[a-z0-9ℝ]+)", t), !match(".*ℝ.*", w), text(w, x).
.output wide, odd
"#).unwrap();
    let program = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| {
        let mut rows: Vec<String> = std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap().lines().map(str::to_owned).collect();
        rows.sort();
        rows.join("\n")
    };
    assert_eq!(read("wide"), "2\u{211d}6789\nx\u{211d}y");
    assert_eq!(read("odd"), "plain");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Decision 0081: `match_all` over nodes the rules bind to a kind, through derived columns that join kinds, and over
/// another pattern's captures; a distinct text is scanned once, and containers with equal text each get their spans.
/// A capture inside `count : { … }` reads its text from the rule.
#[test]
fn match_all_over_nodes_of_a_kind_and_over_captures_gives_spans() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("matchkinds");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "a = \"/x/{id}/{name}\"\nb = \"/x/{id}/{name}\"\nc = 'k9 zz'\n# note AB12 CD3\n").unwrap();
    let rule = dir.join("m.rr");
    std::fs::write(&rule, r#"
.decl str_or_comment(s: node)
str_or_comment(s) :- kind(s, "string").
str_or_comment(s) :- kind(s, "comment").
.decl param(k: symbol, l: number, c: number)
param(x, l, c) :- str_or_comment(s), match_all("\\{(?<k>[a-z]+)\\}", s), text(k, x), line(k, l), col(k, c).
.decl wd(w: span)
wd(w) :- kind(c, "comment"), match_all("(?<w>[A-Z]+[0-9]+)", c).
.decl digit(x: symbol)
digit(x) :- wd(w), match_all("(?<d>[0-9])", w), text(d, x).
.decl ndig(t: symbol, n: number)
ndig(t, n) :- wd(w), text(w, t), n = count : { match_all("(?<d>[0-9])", w) }.
.output param, digit, ndig
"#).unwrap();
    let program = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| {
        let mut rows: Vec<String> = std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap().lines().map(str::to_owned).collect();
        rows.sort();
        rows.join("\n")
    };
    // Two strings with one text: each its own spans, at its own line and column.
    assert_eq!(read("param"), "id\t1\t9\nid\t2\t9\nname\t1\t14\nname\t2\t14");
    assert_eq!(read("digit"), "1\n2\n3");
    assert_eq!(read("ndig"), "AB12\t2\nCD3\t1");
}

/// `match_all` over a node's own text, and over an earlier match's captures (decision 0081): each group is a span
/// placed in the file, with its text and line. Strings and comments in one relation give both; the same text in two
/// strings is matched once and placed at each.
#[test]
fn match_all_captures_spans_over_a_nodes_text_and_over_captures() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("matchnode");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "x = 'ab1 cd22'\ny = 'ab1 cd22'  # ef3\n").unwrap();
    let rule = dir.join("m.rr");
    std::fs::write(&rule, r#"
.decl piece(t: loc)
piece(t) :- kind(t, "string_content").
piece(t) :- kind(t, "comment").
.decl piece_word(t: loc, w: span)
piece_word(t, w) :- piece(t), match_all("(?<w>[a-z]+[0-9]+)", t).
.decl digits(x: symbol, k: number, c: number)
digits(x, k, c) :- piece_word(_, w), match_all("(?<d>[0-9]+)", w), text(d, x), line(d, k), col(d, c).
.decl per(x: symbol, n: number)
per(x, n) :- piece_word(_, w), text(w, x), n = count : { match_all("(?<d>[0-9])", w) }.
.output digits, per
"#).unwrap();
    let program = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| {
        let mut rows: Vec<String> = std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap().lines().map(str::to_owned).collect();
        rows.sort();
        rows.join("\n")
    };
    assert_eq!(read("digits"), "1\t1\t7\n22\t1\t11\n1\t2\t7\n22\t2\t11\n3\t2\t20".split('\n').collect::<std::collections::BTreeSet<_>>().into_iter().collect::<Vec<_>>().join("\n"));
    assert_eq!(read("per"), "ab1\t1\ncd22\t2\nef3\t1");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `match_all("re", n, m)` over a node gives each match m as a span (`rx_span` over the node), and a `count` whose
/// body has a `_` counts every row it ranges over: each match of a node, each occurrence in a symbol (Soufflé's count
/// of tuples). Both gave one row or none before.
#[test]
fn match_all_over_a_node_gives_each_match_and_count_ranges_over_wildcards() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("matchcount");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "f(a, b, c)
g(d)
").unwrap();
    let rule = dir.join("m.rr");
    std::fs::write(&rule, r#"
.decl comma(l: number, c: number)
comma(l, c) :- kind(n, "argument_list"), match_all(",", n, m), line(m, l), col(m, c).
.decl in_node(t: symbol, k: number)
in_node(t, k) :- kind(n, "argument_list"), text(n, t), k = count : { match_all(",", n, _) }.
.decl in_text(t: symbol, k: number)
in_text(t, k) :- kind(n, "argument_list"), text(n, t), k = count : { match_all(",", t, _, _, _) }.
.output comma, in_node, in_text
"#).unwrap();
    let program = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| {
        let mut rows: Vec<String> = std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap().lines().map(str::to_owned).collect();
        rows.sort();
        rows.join("\n")
    };
    assert_eq!(read("comma"), "1\t3\n1\t6");
    assert_eq!(read("in_node"), "(a, b, c)\t2\n(d)\t0");
    assert_eq!(read("in_text"), "(a, b, c)\t2\n(d)\t0");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `data-layer/noqa-sec-ratchet.rr`, whose `match_all` over each line's span is what lowering refused (O80): every
/// SEC code on a suppression line counts, inside the comment or not.
#[test]
fn noqa_sec_ratchet_counts_every_sec_code_on_suppression_lines() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("noqa");
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
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("withheld");
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
    assert_eq!(v, ["t:withheld\tASSIGN\tASSIGN\tgood.py\t1\t0", "t:withheld\tsyntax_error\tBAD000\tbad.py\t1\t0"]);
    assert_eq!(read("lines").lines().count(), 2, "raw-text policies retain both documents");
    assert_eq!(read("err").lines().count(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A run's outputs do not depend on how its files are chunked: one chunk, or chunks of a few files whose mix (small
/// files or large ones) changes from chunk to chunk, each mix measured for itself, give the same rows, cross-file
/// ones too.
#[test]
fn outputs_do_not_depend_on_the_chunk_size() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("chunks");
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
    // Each run's files end in a comment of its own (this test's directory and the chunk size), so no run reads
    // another's rows from the per-file memo (decision 0052): every run evaluates its chunks.
    let originals: Vec<(std::path::PathBuf, String)> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "py")).map(|p| { let s = std::fs::read_to_string(&p).unwrap(); (p, s) }).collect();
    let run = |chunk_nodes: usize| {
        for (p, s) in &originals {
            std::fs::write(p, format!("{s}# chunks of {chunk_nodes} in {}\n", dir.display())).unwrap();
        }
        let out = dir.join(format!("out{chunk_nodes}"));
        // Every file evaluated in every run: a memo hit would not be chunked (decision 0063 memoises base rows too).
        let options = serde_json::json!({ "memo": "off" });
        crate::code_rule_dsl::logic::plan::corpus::run_with_options(&[program.as_str()], &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, chunk_nodes, &Default::default(), &options).unwrap();
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
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("dfs");
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


/// `components` through rules: a cycle is one component named by its smallest document, and keys are equal only
/// over equal closures.
#[test]
fn components_run_through_rules() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("components");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "x = 1\n").unwrap();
    let program = r#"
.decl e(a: number, b: number)
e(1, 2). e(2, 3). e(3, 2). e(4, 2).
.decl own(d: number, h: symbol)
own(1, "a"). own(2, "b"). own(3, "c"). own(4, "a").
.decl comp(d: number, c: number, k: symbol)
comp(d, c, k) :- components(d, c, k, e, own).
.decl same(a: number, b: number)
same(a, b) :- comp(a, _, k), comp(b, _, k), a < b.
.decl named(d: number, c: number)
named(d, c) :- comp(d, c, _).
.output named, same
"#;
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap();
    assert_eq!(read("named"), "1\t1\n2\t2\n3\t2\n4\t4\n");
    // 1 and 4 have the same digest and the same closure; 2 and 3 share their component.
    assert_eq!(read("same"), "1\t4\n2\t3\n");
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
    // Decision 0108: a kind the rule reads by `kind` has its text at any length (the module's here, named by
    // `Subtree`); the source span and the lines are exact either way.
    assert_eq!(rows.iter().filter(|row| row["code"] == "TREE").count(), 1, "the module, a kind the rule reads, has its whole text");
}

/// Decision 0055's guests: a run of `rule` over `files`, each output's sorted rows.
fn guests_run(name: &str, files: &[(&str, &str)], rule: &str, outputs: &[&str]) -> Vec<Vec<String>> {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension(name);
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

/// A string annotation is parsed where it stands: its names have the file's lines and columns, through the escapes
/// and the parts of a concatenation (the normalized text's map); a string inside a parsed one is parsed in the next
/// round, for a rule that reaches it through the annotation (`ann`): an annotation is not shown to rules that do not
/// name it, so `ident`'s `kind(s, "string")` does not see it. Bytes give no guest.
#[test]
fn string_annotations_are_guests_at_their_host_lines() {
    let py = "x: \"List[int]\" = []\n\ndef f(y: 'Dict[str, Foo]') -> \"Bar\": ...\nz: \"A\\x42\" = 1\nw: (\"Li\" \"st\") = 2\nq: \"Opt['Baz']\" = 3\nb: b\"Nope\" = 4\n";
    let rule = r#"
.decl ident(x: symbol, l: number, c: number)
ident(x, l, c) :- kind(s, "string"), parsed(s, "annotation", _), guest(g, _, s, _), node(g, i, "identifier", _, _), text(i, x), line(i, l), col(i, c).
.decl ann(n: node)
ann(r) :- kind(s, "string"), parsed(s, "annotation", r).
ann(c) :- ann(p), child(p, c, _).
.decl nested(x: symbol, l: number, c: number)
nested(x, l, c) :- ann(s), kind(s, "string"), parsed(s, "annotation", _), guest(g, _, s, _), node(g, i, "identifier", _, _), text(i, x), line(i, l), col(i, c).
.decl whole(x: symbol, l: number, c: number)
whole(x, l, c) :- kind(s, "concatenated_string"), parsed(s, "annotation", _), guest(g, _, s, _), node(g, i, "identifier", _, _), text(i, x), line(i, l), col(i, c).
.decl roots(k: symbol)
roots(k) :- kind(s, "string"), parsed(s, "annotation", r), kind(r, k).
.output ident, nested, whole, roots
"#;
    let [names, nested, whole, roots] = <[Vec<String>; 4]>::try_from(guests_run("annotations", &[("a.py", py)], rule, &["ident", "nested", "whole", "roots"])).unwrap();
    assert!(names.contains(&"List\t1\t4".to_owned()), "{names:?}");
    assert!(names.contains(&"int\t1\t9".to_owned()), "{names:?}");
    assert!(names.contains(&"Foo\t3\t20".to_owned()), "{names:?}");
    assert!(names.contains(&"Bar\t3\t31".to_owned()), "{names:?}");
    // "A\x42" is AB: its normalized text, mapped (decision 0058 §3), so AB starts at the A.
    assert!(names.contains(&"AB\t4\t4".to_owned()), "{names:?}");
    // ("Li" "st"): each part is a string of its own, and the concatenation is List, from its first part's text.
    assert!(names.contains(&"Li\t5\t5".to_owned()), "{names:?}");
    assert_eq!(whole, ["List\t5\t5"]);
    assert_eq!(roots, ["identifier", "subscript"]);
    // 'Baz' inside "Opt['Baz']": round 2, through the annotation; `ident` does not see inside it.
    assert_eq!(nested, ["Baz\t6\t9"]);
    assert!(!names.iter().any(|n| n.starts_with("Baz")), "{names:?}");
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
    assert_eq!(v, ["t:guest\tguest_error\tGBAD\ta.py\t2\t0"]);
    assert_eq!(roots, ["1"], "the broken guest has no tree");
    assert_eq!(kept, ["1", "2"], "both are guests");
    let [v, roots] = <[Vec<String>; 2]>::try_from(guests_run("gpartial", &[("a.py", py)], &rule(".on guest_error partial"), &["violation", "roots"])).unwrap();
    assert!(v.is_empty(), "{v:?}");
    assert_eq!(roots, ["1", "2"], "the broken guest keeps its partial tree");
}

/// A library's `embed` and `embed_value` declare guests of a role. `sub_tape` gives a guest's tape: equal texts share
/// one. Guests in the run's language are shown to rules that do not name them (`top`) unless `.guests off`; a rule
/// that reaches them through a guest relation (`doc_role`) sees them either way (decision 0058 §1).
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
.decl tapes(t: tape)
tapes(t) :- kind(s, "string"), sub_tape(s, "python", t).
.decl top(x: symbol, l: number)
top(x, l) :- kind(i, "import_statement"), text(i, x), line(i, l).
.output imp, tapes, top
"#
        )
    };
    let guests = ["code\timport os\t1", "code\timport os\t2", "made\timport sys\t3"];
    let [imp, tapes, top] = <[Vec<String>; 3]>::try_from(guests_run("embed", &[("a.py", py)], &rule(""), &["imp", "tapes", "top"])).unwrap();
    assert_eq!(imp, guests);
    assert_eq!(tapes.len(), 1, "{tapes:?}");
    assert_eq!(top, ["import os\t1", "import os\t2", "import sys\t3"], "Python guests are shown in a Python run");
    let [imp, top] = <[Vec<String>; 2]>::try_from(guests_run("embedoff", &[("a.py", py)], &rule(".guests off"), &["imp", "top"])).unwrap();
    assert_eq!(imp, guests, "`.guests off` hides guests; it does not stop a library's embeds");
    assert!(top.is_empty(), "{top:?}");
}

/// SQL in a Python string (decision 0058 §1–2): a library's `embed` makes an SQL guest on an SQL tape (tag 10). A
/// rule that reaches it through `guest` names SQL kinds, and they print as SQL's; a rule that does not name a guest
/// sees only the Python file, even for the qualified `"sql:…"` kind.
#[test]
fn sql_in_python_is_an_sql_tape_hidden_from_unscoped_rules() {
    let py = "def f(db):\n    db.execute(\"SELECT name FROM users WHERE id = 1\")\n    db.execute(\"SELECT \\\"id\\\" FROM orders\")\n";
    let rule = r#"
embed(d, s, "sql", "query") :- kind(c, "call"), field(c, "arguments", a), child(a, s, _), kind(s, "string"), docof(s, d).
.decl sqlkinds(k: symbol)
sqlkinds(k) :- guest(g, _, _, "sql"), node(g, _, k, _, _).
.decl docs(g: number)
docs(g) :- guest(g, _, _, "sql").
.decl objects(x: symbol, l: number, c: number)
objects(x, l, c) :- guest(g, _, _, "sql"), node(g, n, "object_reference", _, _), text(n, x), line(n, l), col(n, c).
.decl unscoped(k: symbol)
unscoped(k) :- node(_, _, k, _, _).
.decl qualified(x: symbol)
qualified(x) :- kind(n, "sql:object_reference"), text(n, x).
.decl counted(c: number)
counted(c) :- c = count : { kind(_, "sql:object_reference") }.
.output sqlkinds, docs, objects, unscoped, qualified, counted
"#;
    let [sqlkinds, docs, objects, unscoped, qualified, counted] =
        <[Vec<String>; 6]>::try_from(guests_run("sqlguest", &[("a.py", py)], rule, &["sqlkinds", "docs", "objects", "unscoped", "qualified", "counted"])).unwrap();
    assert!(sqlkinds.contains(&"object_reference".to_owned()), "{sqlkinds:?}");
    assert!(!sqlkinds.contains(&"call".to_owned()), "SQL nodes are SQL's kinds: {sqlkinds:?}");
    assert!(docs.len() == 2 && docs.iter().all(|g| g.parse::<i64>().unwrap() >= i64::from(crate::code_rule_dsl::facts::vtab::syntax::HIDDEN_KEYS)), "an SQL guest in a Python run is a hidden document: {docs:?}");
    assert!(objects.contains(&"users\t2\t33".to_owned()), "{objects:?}");
    // `"SELECT \"id\" FROM orders"` parses as its normalized text, and `orders` keeps its column past the escapes.
    assert!(objects.contains(&"orders\t3\t35".to_owned()), "{objects:?}");
    assert!(unscoped.contains(&"call".to_owned()), "{unscoped:?}");
    assert!(!unscoped.contains(&"object_reference".to_owned()), "SQL nodes are hidden: {unscoped:?}");
    assert!(qualified.is_empty(), "{qualified:?}");
    // An aggregate's atom is bounded too.
    assert!(counted.iter().all(|c| c == "0"), "{counted:?}");
}

/// Tape operations (decision 0058 §4): `@parse(g, x)` makes a value document from a text (here a string's
/// normalized text, `str`), with its own lines; `@unparse(t)` gives a tape's text back; `@splice(t, n, t2)` is t with
/// node n's text replaced by t2's, parsed again. A value document is hidden from rules that do not reach it.
#[test]
fn parse_splice_and_unparse_make_value_documents() {
    let py = "def f(db):\n    db.execute(\"SELECT name \\\n FROM users\")\n";
    let rule = r#"
.decl q(s: node, x: symbol)
q(s, x) :- kind(c, "call"), field(c, "arguments", a), child(a, s, _), kind(s, "string"), str(s, x).
.decl tables(x: symbol, l: number)
tables(x, l) :- q(_, x0), t = @parse("sql", x0), tape_of(d, t), node(d, n, "object_reference", _, _), text(n, x), line(n, l).
.decl back(x: symbol)
back(x) :- q(_, x0), t = @parse("sql", x0), x = @unparse(t).
.decl fixed(x: symbol)
fixed(x) :- q(_, x0), t = @parse("sql", x0), tape_of(d, t), node(d, n, "object_reference", _, _), text(n, "users"), t2 = @parse("sql", "accounts"), t3 = @splice(t, n, t2), x = @unparse(t3).
.decl seen(x: symbol)
seen(x) :- kind(n, "object_reference"), text(n, x).
.decl roles(r: symbol)
roles(r) :- doc_role(_, r).
.output tables, back, fixed, seen, roles
"#;
    let [tables, back, fixed, seen, roles] = <[Vec<String>; 5]>::try_from(guests_run("tapeops", &[("a.py", py)], rule, &["tables", "back", "fixed", "seen", "roles"])).unwrap();
    // `"SELECT name \<newline> FROM users"`: the escaped newline is gone from the text that is parsed.
    assert_eq!(back, ["SELECT name  FROM users"]);
    assert_eq!(tables, ["users\t1"], "a value's places are its own text's");
    assert_eq!(fixed, ["SELECT name  FROM accounts"]);
    assert_eq!(roles, ["primary", "value"]);
    assert!(seen.is_empty(), "{seen:?}");
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
    let path = crate::code_rule_dsl::fixtures::temp().with_extension("public.rr");
    std::fs::write(&path, program).unwrap();
    let (program, _) = read_file(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    print(&program).unwrap()
}

fn lowered_findings(program: &str) -> Vec<serde_json::Value> {
    let root = crate::code_rule_dsl::fixtures::temp().with_extension("lowered-findings");
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


#[test]
fn printed_guest_demand_rules_are_not_generated_again() {
    for source in [r#"
.decl context(n: node, x: symbol)
.input context
.decl parsed(s: node, grammar: symbol, r: node)
.input parsed
.decl result(r: node)
result(r) :- context(n, _), parsed(n, "annotation", r).
.output result
"#,r#"
.decl context(n: node, x: symbol)
.input context
.decl result(t: tape)
result(t) :- context(n, x), t = @parse("sql", x).
.output result
"#] {
        let original = parse(source).unwrap();
        assert!(original.rules.iter().any(|r|r.head.rel.starts_with("rr_")));
        let mut current = original.clone();
        for _ in 0..3 {
            current = parse(&print(&current).unwrap()).unwrap();
            assert!(difference(&original,&current).is_none(), "{:?}",difference(&original,&current));
        }
    }
}

/// A value document is its askers' files' (O96): two files asking for one text share a value, and each keeps its
/// rows in the memo. A second run over the same files reads every file from the memo and gives the first run's rows.
#[test]
fn value_documents_are_memoised_with_the_files_that_asked() {
    let shared = "def f(db):\n    db.execute(\"SELECT name FROM users\")\n";
    let own = "def g(db):\n    db.execute(\"SELECT id FROM orders\")\n";
    // A text no other test parses, so the first run cannot read these files from an earlier run's memo.
    let marker = format!("# {:?}\n", std::time::SystemTime::now());
    let files = [("a.py", format!("{marker}{shared}")), ("b.py", format!("{marker}{shared}")), ("c.py", format!("{marker}{own}"))];
    let rule = r#"
.decl q(s: node, x: symbol)
q(s, x) :- kind(c, "call"), field(c, "arguments", a), child(a, s, _), kind(s, "string"), str(s, x).
.decl tables(s: node, x: symbol)
tables(s, x) :- q(s, x0), t = @parse("sql", x0), tape_of(d, t), node(d, n, "object_reference", _, _), text(n, x).
.decl objects(n: node, x: symbol)
objects(n, x) :- q(s, x0), t = @parse("sql", x0), tape_of(d, t), node(d, n, "object_reference", _, _), text(n, x).
.output tables, objects
"#;
    let run = |name: &str| {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension(name);
        std::fs::create_dir_all(&dir).unwrap();
        for (path, text) in &files {
            std::fs::write(dir.join(path), text).unwrap();
        }
        let path = dir.join("g.rr");
        std::fs::write(&path, rule).unwrap();
        let program = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&path).unwrap()).unwrap();
        let out = dir.join("out");
        let report = crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        let rows: Vec<Vec<String>> = ["tables", "objects"]
            .iter()
            .map(|rel| {
                let mut rows: Vec<String> = std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap_or_default().lines().map(str::to_owned).collect();
                rows.sort();
                rows
            })
            .collect();
        let _ = std::fs::remove_dir_all(&dir);
        (report.memo, rows)
    };
    let (memo, first) = run("valmemo1");
    assert_eq!(memo, (0, 0), "every chunk's rows are attributed to files");
    assert_eq!(first[0].len(), 3, "{first:?}");
    assert_eq!(first[1].iter().map(|r| r.split('\t').nth(1).unwrap().to_owned()).collect::<BTreeSet<_>>(), BTreeSet::from(["orders".to_owned(), "users".to_owned()]), "{first:?}");
    let (memo, second) = run("valmemo2");
    assert_eq!(memo, (3, 0), "every file is read from the memo");
    assert_eq!(second, first);
}

/// A combined run's programs each see the guests their own `.guests` shows (decision 0058 §1, O97): one batch, one
/// set of embeds, and each program's rules bounded by its own setting.
#[test]
fn combined_programs_see_guests_by_their_own_policy() {
    let py = "a = \"import os\"\nb = \"import sys\"\n";
    let program = |guests: &str| {
        format!(
            r#"
{guests}
embed(d, s, "python", "code") :- kind(s, "string"), docof(s, d).
.decl top(x: symbol, l: number)
top(x, l) :- kind(i, "import_statement"), text(i, x), line(i, l).
.decl through(x: symbol)
through(x) :- doc_role(g, "code"), node(g, i, "import_statement", _, _), text(i, x).
.output top, through
"#
        )
    };
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("combined_guests");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), py).unwrap();
    let printed: Vec<String> = [program(".guests off"), program("")]
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let path = dir.join(format!("p{i}.rr"));
            std::fs::write(&path, text).unwrap();
            crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&path).unwrap()).unwrap()
        })
        .collect();
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run_all(&printed.iter().map(String::as_str).collect::<Vec<_>>(), &dir, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| {
        let mut rows: Vec<String> = std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap_or_default().lines().map(str::to_owned).collect();
        rows.sort();
        rows
    };
    let (off, on, off_through, on_through) = (read("p0_top"), read("p1_top"), read("p0_through"), read("p1_through"));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(off.is_empty(), "`.guests off` hides the guests from its own rules: {off:?}");
    assert_eq!(on, ["import os\t1", "import sys\t2"], "the other program shows them");
    assert_eq!(off_through, ["import os", "import sys"], "a guest relation reaches them under `.guests off`");
    assert_eq!(on_through, off_through);
}

/// A kind set read by `nearest` over a scoped SQL node is resolved in SQL's grammar (decision 0058 §2, O98): the
/// set is copied as `clauses@sql`, its kinds qualified. The same set over an unscoped node stays the run's.
#[test]
fn kind_sets_over_a_guest_are_its_grammars() {
    let py = "def f(db):\n    db.execute(\"SELECT name FROM users\")\n";
    let rule = r#"
embed(d, s, "sql", "query") :- kind(c, "call"), field(c, "arguments", a), child(a, s, _), kind(s, "string"), docof(s, d).
.decl clauses(kind: symbol)
clauses("relation"). clauses("function_definition").
.decl enclosing(x: symbol, k: symbol)
enclosing(x, k) :- guest(g, _, _, "sql"), node(g, n, "object_reference", _, _), text(n, x), nearest(n, a, clauses), node(_, a, k, _, _).
.decl outer(k: symbol)
outer(k) :- kind(n, "call"), nearest(n, a, clauses), node(_, a, k, _, _).
.output enclosing, outer
"#;
    let [within, outer] = <[Vec<String>; 2]>::try_from(guests_run("sqlkindset", &[("a.py", py)], rule, &["enclosing", "outer"])).unwrap();
    assert_eq!(within, ["users\trelation"], "the set names SQL's `relation` over the guest");
    assert_eq!(outer, ["function_definition"], "and Python's kinds over the file");
}

/// `.scope` selects a repository's files (`core.rr`): a finding in a document outside `dirs`, or under a `tests`
/// part with `tests false`, is dropped; `.scope given` (files a test gives) keeps them all.
#[test]
fn scope_drops_findings_outside_the_policy_files() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("scope");
    for p in ["impl/a.py", "impl/tests/test_a.py", "other/b.py"] {
        std::fs::create_dir_all(dir.join(p).parent().unwrap()).unwrap();
        std::fs::write(dir.join(p), "f()\n").unwrap();
    }
    let program = r#"
.policy "t:scope" revision 1
.language python
.scope dirs "other",   // a comment, then the line goes on
    "impl"
.scope exclude "other/**"
.scope tests false
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl line(n: node, l: number)
.decl doc(d: number, path: symbol, module: symbol)
.input node, line, doc
.violation Call(at: node) "a call"
Call(c) :- node(_, c, "call", _, _).
"#;
    let paths = |program: &str| {
        let out = dir.join("out");
        crate::code_rule_dsl::logic::plan::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        let mut got: Vec<String> = std::fs::read_to_string(out.join("violation.csv")).unwrap().lines().map(|l| l.split('\t').nth(3).unwrap().to_owned()).collect();
        got.sort();
        got
    };
    assert_eq!(paths(program), ["impl/a.py"]);
    assert_eq!(paths(&format!("{program}.scope given\n")), ["impl/a.py", "impl/tests/test_a.py", "other/b.py"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The other `.scope` forms (O117): `dirs option("…")` takes its directories from the run's options, `text` reads
/// documents of no grammar as text only (lines, no tree), `report dirs` drops findings outside its directories while
/// facts still come from the whole scope, and `embedded false` is `.guests off`.
#[test]
fn scope_takes_option_dirs_text_documents_and_report_dirs() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("scope-forms");
    for (p, text) in [("impl/a.py", "f()\n"), ("app/x/c.py", "g()\n"), ("other/b.py", "h()\n"), ("docs/d.md", "intro\nTODO one\n"), ("docs/e.txt", "TODO two\n"), ("tools/t.py", "k()\n")] {
        std::fs::create_dir_all(dir.join(p).parent().unwrap()).unwrap();
        std::fs::write(dir.join(p), text).unwrap();
    }
    let program = r#"
.policy "t:scope-forms" revision 1
.language python
#include "python/ast.rr"
.scope include "impl/**", "docs/**", "tools/**"
.scope dirs option("roots")
.scope text "**/*.md"
.scope report dirs "impl", "app", "docs"
.violation Call(at: node) "a call"
Call(c) :- kind(c, "call").
.violation Todo(at: loc) "a todo"
Todo(@place(p, l)) :- doc(d, p, _), src_line(d, l, sp), contains("TODO", sp).
"#;
    // The program with its includes, as the corpus reads it.
    let rule = dir.with_extension("scope-forms.rr");
    std::fs::write(&rule, program).unwrap();
    let flat = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
    std::fs::remove_file(&rule).unwrap();
    let paths = |options: serde_json::Value| {
        let out = dir.join("out");
        crate::code_rule_dsl::logic::plan::corpus::run_with_options(&[flat.as_str()], &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18, &Default::default(), &options).unwrap();
        let mut got: Vec<String> = std::fs::read_to_string(out.join("violation.csv")).unwrap().lines().map(|l| format!("{}:{}", l.split('\t').nth(3).unwrap(), l.split('\t').nth(4).unwrap())).collect();
        got.sort();
        got
    };
    // `docs/d.md` is read as text (its TODO line); `docs/e.txt` matches no text glob; `tools/` is read but reports
    // nothing; `app/` only with the option.
    assert_eq!(paths(serde_json::json!({})), ["docs/d.md:2", "impl/a.py:1"]);
    assert_eq!(paths(serde_json::json!({"roots": ["app"]})), ["app/x/c.py:1", "docs/d.md:2", "impl/a.py:1"]);
    let lowered = crate::code_rule_dsl::logic::rr::parse(&format!("{flat}\n.scope embedded false\n")).unwrap();
    assert_eq!(lowered.text_globs, ["**/*.md"]);
    let off = lowered.facts.get(crate::code_rule_dsl::facts::vtab::syntax::GUEST_POLICY).into_iter().flatten().any(|r| r == &vec![crate::code_rule_dsl::logic::front::Value::Str("library".into()), crate::code_rule_dsl::logic::front::Value::Str("off".into())]);
    assert!(off, "embedded false is guests off");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A finding is keyed by its column too: two on one line are two rows (main reports each). And an annotation's
/// guest is not shown to rules that do not name it: `tctx`'s demand asks for every string its inputs hold, `"--host"`
/// (an argument) too, and shown, that guest was code to `Neg`: `--host`, a doubled minus (B002 on 157 argparse
/// lines). A type expression is not code; the rules that read one reach it through `parsed`.
#[test]
fn findings_keep_their_columns_and_an_annotation_is_not_code() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("finding-columns");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "p.add(\"--host\")\nq = --a + --b\nv: \"--k\" = 1\n").unwrap();
    let program = r#"
.policy "t:finding-columns" revision 1
.language python
#include "python/ast.rr"
.violation Neg(at: node) "a doubled minus"
Neg(u) :- kind(u, "unary_operator"), op(u, "-"), field(u, "argument", a), kind(a, "unary_operator").
.decl tctx(n: node)
tctx(t) :- kind(t, "type").
tctx(r) :- tctx(s), str_node(s), parsed(s, "annotation", r).
tctx(c) :- tctx(p), child(p, c, _).
"#;
    let rule = dir.with_extension("finding-columns.rr");
    std::fs::write(&rule, program).unwrap();
    let flat = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
    std::fs::remove_file(&rule).unwrap();
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(&flat, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let mut got: Vec<String> = std::fs::read_to_string(out.join("violation.csv")).unwrap().lines().map(|l| l.split('\t').skip(3).collect::<Vec<_>>().join(":")).collect();
    got.sort();
    assert_eq!(got, ["a.py:2:10", "a.py:2:4"], "both of line 2's, and none in the argument's string");
    let _ = std::fs::remove_dir_all(&dir);
}

/// An AST pattern literal (O103): ``n ~ `$O[:-$K]` `` holds at each match, and `$K` binds K to the node it captured;
/// a slice with a start is another pattern's, and a negated pattern holds where none matches.
#[test]
fn pattern_literals_match_and_bind_their_metavariables() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("pattern");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/a.py"), "t = name[:-4]\nu = name[1:-7]\nv = other[:-13]\n").unwrap();
    let rule = dir.join("rule.rr");
    std::fs::write(&rule, r#"
.policy "t:pattern" revision 1
.language python
.violation Cut(at: node, k: symbol) "{k}"
Cut(n, k) :- n ~ `$O[:-$K]`, text(K, k).
.violation Plain(at: node) "no step"
Plain(n) :- kind(n, "subscript"), !(n ~ `$O[:-$K]`).
"#).unwrap();
    let program = crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap();
    assert_eq!(program.ast_patterns.len(), 1);
    let text = crate::code_rule_dsl::logic::rr::print(&program).unwrap();
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(&text, &dir.join("src"), None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let mut got: Vec<String> = std::fs::read_to_string(out.join("violation.csv")).unwrap().lines().map(|l| {
        let f: Vec<&str> = l.split('\t').collect();
        format!("{}:{}", f[1], f[4])
    }).collect();
    got.sort();
    assert_eq!(got, ["Cut:1", "Cut:3", "Plain:2"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// O101: a program whose reads make demands (`parsed`, `@parse`, `@splice`, `file_line`) prints back as itself.
/// The printed text declares the demands it was lowered with, so reading it makes none again.
#[test]
fn a_program_with_demands_prints_back_as_itself() {
    let text = r#"
.decl q(s: node, x: symbol)
q(s, x) :- kind(c, "call"), field(c, "arguments", a), child(a, s, _), kind(s, "string"), str(s, x).
.decl ann(s: node, r: node)
ann(s, r) :- kind(s, "string"), parsed(s, "annotation", r).
.decl fixed(x: symbol)
fixed(x) :- q(_, x0), t = @parse("sql", x0), tape_of(d, t), node(d, n, "object_reference", _, _), t2 = @parse("sql", "accounts"), t3 = @splice(t, n, t2), x = @unparse(t3).
.decl lines(i: number)
lines(i) :- file_line("setup.cfg", i, _).
.output ann, fixed, lines
"#;
    let once = parse(text).unwrap();
    let printed = print(&once).unwrap();
    let twice = parse(&printed).unwrap_or_else(|e| panic!("{e:?} reading back:\n{printed}"));
    assert!(difference(&once, &twice).is_none(), "{:?}", difference(&once, &twice));
    assert_eq!(print(&twice).unwrap(), printed);
    for demand in ["rr_embed_req", "rr_parse_req", "rr_splice_req", "rr_file_req"] {
        assert!(once.decls.contains_key(demand), "{demand} is made");
    }
}

/// O106: a relation may be named like a SQL keyword. Every statement that names it quotes it (`sql::ident`), through
/// recursion, negation and an aggregate.
#[test]
fn relations_named_like_sql_keywords_run() {
    let rule = r#"
.decl values(x: number)
values(1).
values(2).
values(3).
.decl order(x: number, y: number)
order(x, y) :- values(x), values(y), y = x + 1.
order(x, z) :- order(x, y), order(y, z).
.decl select(x: number)
select(x) :- values(x), !order(x, _).
.decl group(n: number)
group(n) :- n = count : { order(_, _) }.
.output values, order, select, group
"#;
    let [values, order, select, group] = <[Vec<String>; 4]>::try_from(guests_run("sqlkeywords", &[("a.py", "x = 1\n")], rule, &["values", "order", "select", "group"])).unwrap();
    assert_eq!(values, ["1", "2", "3"]);
    assert_eq!(order, ["1\t2", "1\t3", "2\t3"]);
    assert_eq!(select, ["3"]);
    assert_eq!(group, ["3"]);
}

/// The native steps a string rule takes: a concatenation (`q = cat("m", ".", x)`), a split read back
/// (`split_dot`'s `$s = @cat($m, $x)`), a set of constant patterns (`contains(…, {…})` once they are symbols) and
/// a set membership (`x = "a" ; x = "b"` as `@equals`). Their rows are the SQL backend's.
#[test]
fn string_steps_give_the_rows_sql_gives() {
    let rule = r#"
.decl ident(n: node, x: symbol)
ident(n, x) :- kind(n, "identifier"), text(n, x).
.decl dotted(n: node, q: symbol)
dotted(n, q) :- ident(n, x), q = cat("m", ".", x).
.decl last(n: node, x: symbol)
last(n, x) :- dotted(n, q), split_dot(q, m, x), m != "".
.decl hit(x: symbol)
hit(x) :- ident(_, x), (contains("ol", x) ; contains("et", x)).
.decl one(x: symbol)
one(x) :- ident(_, x), (x = "alpha" ; x = "beta").
.output last, hit, one
"#;
    let [last, hit, one] = <[Vec<String>; 3]>::try_from(guests_run("stringsteps", &[("a.py", "alpha = beta + gamma\nwolf = 1\n")], rule, &["last", "hit", "one"])).unwrap();
    let mut words: Vec<String> = last.iter().map(|l| l.split('\t').nth(1).unwrap().to_owned()).collect();
    words.sort();
    assert_eq!(words, ["alpha", "beta", "gamma", "wolf"]);
    assert_eq!(hit, ["beta", "wolf"]);
    assert_eq!(one, ["alpha", "beta"]);
}

/// Decision 0091: in a combined run, an included library's relations are one copy for the run, made over every
/// document read, and each policy's rules see only its own scope's documents. Policy B's cross-file rule (two
/// definitions of one name) would join a document only A's scope reads if the library's rows reached it unguarded.
/// `.global` shares a policy's own relation when the definitions are alike, and refuses it when they differ.
#[test]
fn library_relations_are_shared_and_each_policy_reads_its_own_scope() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("fact-spaces");
    for (p, text) in [("a/x.py", "def f():\n    pass\n"), ("b/y.py", "def f():\n    pass\n"), ("b/z.py", "def g():\n    pass\n")] {
        std::fs::create_dir_all(dir.join("src").join(p).parent().unwrap()).unwrap();
        std::fs::write(dir.join("src").join(p), text).unwrap();
    }
    std::fs::write(dir.join("fact-spaces-lib.rr"), r#"
.decl def_at(f: node, s: symbol)
def_at(f, s) :- kind(f, "function_definition"), field(f, "name", n), text(n, s).
"#).unwrap();
    let policy = |name: &str, scope: &str, body: &str| -> String {
        let rule = dir.join(format!("{name}.rr"));
        std::fs::write(&rule, format!(".policy \"t:{name}\" revision 1\n.language python\n#include \"python/ast.rr\"\n#include \"fact-spaces-lib.rr\"\n.scope dirs \"{scope}\"\n{body}\n")).unwrap();
        crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap()
    };
    let a = policy("a", "a", ".violation Fn(at: node) \"a def\"\nFn(f) :- def_at(f, _).");
    let b = policy("b", "b", ".violation Dup(at: node) \"two defs\"\nDup(f) :- def_at(f, s), def_at(g, s), f != g.");
    let run = |programs: &[&str], out: &str| -> Vec<String> {
        let out = dir.join(out);
        crate::code_rule_dsl::logic::plan::corpus::run_all(programs, &dir.join("src"), &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        let mut got: Vec<String> = std::fs::read_to_string(out.join("violation.csv")).unwrap_or_default().lines().map(|l| format!("{}:{}", l.split('\t').nth(1).unwrap(), l.split('\t').nth(3).unwrap())).collect();
        got.sort();
        got
    };
    let alone: Vec<String> = [run(&[&a], "out-a"), run(&[&b], "out-b")].concat();
    assert_eq!(alone, ["Fn:a/x.py"]);
    assert_eq!(run(&[&a, &b], "out-ab"), alone, "the pack's findings are the policies' alone");
    // The library's relation is unguarded in both policies (so the simplifier makes it one); B's rule is guarded.
    let combined = crate::code_rule_dsl::logic::plan::corpus::combine(&[&a, &b], "").unwrap();
    let reads_scope = |head: &str| combined.rules.iter().filter(|r| r.head.rel == head).any(|r| r.body.iter().filter_map(|l| l.atom()).any(|x| x.rel.contains("rr_scope_")));
    assert!(!reads_scope("p0_def_at") && !reads_scope("p1_def_at"), "the library's rules stay as written");
    assert!(combined.rules.iter().any(|r| r.head.rel.starts_with("p1_") && r.body.iter().filter_map(|l| l.atom()).any(|x| x.neg && x.rel == "p1_rr_scope_not")), "B's reads are guarded");
    // `.global`: alike (up to variable names) is shared; different is refused.
    let named = |name: &str, scope: &str, rule: &str| policy(name, scope, &format!(".global named\n.decl named(f: node)\n{rule}\n.violation N(at: node) \"n\"\nN(f) :- named(f)."));
    let c = named("c", "a", "named(f) :- kind(f, \"function_definition\").");
    let d = named("d", "b", "named(x) :- kind(x, \"function_definition\").");
    let e = named("e", "b", "named(f) :- kind(f, \"class_definition\").");
    assert!(crate::code_rule_dsl::logic::plan::corpus::combine(&[&c, &d], "").is_ok());
    let refused = crate::code_rule_dsl::logic::plan::corpus::combine(&[&c, &e], "").err().map(|e| e.to_string()).unwrap_or_default();
    assert!(refused.contains(".global named"), "{refused}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Decision 0091 with the libraries held once (0089): the library is a part of its own, under its own names, made
/// over every document read; each policy's rules still see only its scope. Policy B's cross-file rule (two calls of
/// one name) would join a call only A's scope reads if the library's rows reached it unguarded.
#[test]
fn a_library_held_once_is_read_by_each_policy_in_its_own_scope() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("held-once");
    for (p, text) in [("a/x.py", "f()\n"), ("b/y.py", "f()\n"), ("b/z.py", "g()\n")] {
        std::fs::create_dir_all(dir.join("src").join(p).parent().unwrap()).unwrap();
        std::fs::write(dir.join("src").join(p), text).unwrap();
    }
    let policy = |name: &str, scope: &str, body: &str| format!(".policy \"t:{name}\" revision 1\n.language python\n#include \"python/ast.rr\"\n.scope dirs \"{scope}\"\n{body}\n");
    let a = policy("a", "a", ".violation Fn(at: node) \"a call\"\nFn(c) :- call_name(c, _).");
    let b = policy("b", "b", ".violation Dup(at: node) \"two calls\"\nDup(c) :- call_name(c, s), call_name(d, s), c != d.");
    let run = |programs: &[&str], out: &str| -> Vec<String> {
        let out = dir.join(out);
        crate::code_rule_dsl::logic::plan::corpus::run_all(programs, &dir.join("src"), &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        let mut got: Vec<String> = std::fs::read_to_string(out.join("violation.csv")).unwrap_or_default().lines().map(|l| format!("{}:{}", l.split('\t').nth(1).unwrap(), l.split('\t').nth(3).unwrap())).collect();
        got.sort();
        got
    };
    let alone: Vec<String> = [run(&[&a], "out-a"), run(&[&b], "out-b")].concat();
    assert_eq!(alone, ["Fn:a/x.py"]);
    assert_eq!(run(&[&a, &b], "out-ab"), alone, "the pack's findings are the policies' alone");
    let combined = crate::code_rule_dsl::logic::plan::corpus::combine(&[&a, &b], "").unwrap();
    let rules_of = |head: &str| combined.rules.iter().filter(|r| r.head.rel == head).collect::<Vec<_>>();
    assert!(!rules_of("call_name").is_empty() && rules_of("p0_call_name").is_empty(), "the library is held once, under its own names");
    assert!(rules_of("call_name").iter().all(|r| r.body.iter().filter_map(|l| l.atom()).all(|x| !x.rel.contains("rr_scope_"))), "the library's rules are unguarded");
    assert!(combined.rules.iter().any(|r| r.head.rel.starts_with("p1_") && r.body.iter().filter_map(|l| l.atom()).any(|x| x.neg && x.rel == "p1_rr_scope_not")), "B's rule is guarded");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A grammar a rule names is one the language registry has (decision 0082): a dialect's label or a fragment.
#[test]
fn a_grammar_the_registry_does_not_have_is_refused() {
    let lower = |text: &str| {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("grammar");
        std::fs::create_dir_all(&dir).unwrap();
        let rule = dir.join("g.rr");
        std::fs::write(&rule, text).unwrap();
        let got = crate::code_rule_dsl::logic::rr::parse_file(&rule).map(|_| ()).map_err(|e| e.to_string());
        let _ = std::fs::remove_dir_all(&dir);
        got
    };
    let rule = |g: &str| format!(".decl hit(s: node)\nhit(s) :- kind(s, \"string\"), parsed(s, \"{g}\", _).\n.output hit\n");
    assert!(lower(&rule("annotation")).is_ok(), "a fragment");
    assert!(lower(&rule("sql")).is_ok(), "a dialect's label");
    let refused = lower(&rule("postgres")).unwrap_err();
    assert!(refused.contains("no grammar \"postgres\""), "{refused}");
}

/// bootstrap-engine-invariants' BEI027: `count : { bei_identity(x), count : { def_walk(n, f), bei_ref(n, x) } = 0 } > 0`.
/// The inner count is per (f, x), f the rule's: the outer helper is a relation of f and x, f bound by the context.
/// It lost f, and planning failed with "unplaced literals … in rr_agg72($x)".
#[test]
fn a_nested_aggregate_keeps_the_rules_variables_it_reads() {
    let p = parse(".decl ident(x: symbol)\nident(\"a\").\n.decl fn(f: number)\n.decl ref(n: number, f: number, x: symbol)\n.input fn, ref\n.decl hit(f: number)\n.output hit\nhit(f) :- fn(f), count : { ident(x), count : { ref(n, f, x) } = 0 } > 0.\n").unwrap();
    let shown: Vec<String> = p.rules.iter().map(ToString::to_string).collect();
    // The inner count-zero lowers to anti-existence. Its helper still carries both outer keys.
    let helper = p.rules.iter().find(|r| r.body.iter().any(|l|
        matches!(l, Literal::Atom(a) if a.neg && a.rel == "ref"))).expect("the inner anti-existence rule");
    assert!(helper.head.rel.starts_with("rr_agg") && helper.head.vars() == BTreeSet::from(["f".into(), "x".into()]), "{shown:#?}");
    assert!(helper.body.iter().filter_map(Literal::atom).any(|a| a.rel.starts_with("rr_ctx")), "{shown:#?}");
    let hit = shown.iter().find(|r| r.starts_with("hit(")).unwrap();
    assert!(hit.contains("($f, _)"), "the outer count groups by f: {hit}");
}

/// Rules that order nodes by id mean what they did under preorder ids (decision 0094 §2, `lower::order_nodes`):
/// nodes in different granules (module statements, functions, classes) compare and aggregate by their preorder
/// keys. Each pair `x` before `x` and the last `x` are by line, under either numbering.
#[test]
fn ordering_nodes_follows_preorder_across_granules() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("nodeorder");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "x = 1\nprint(x)\ndef f():\n    return x\nclass C:\n    y = x\nz = [x]\nx = 2\ndef g():\n    return x\n").unwrap();
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl text(n: node, s: symbol)
.decl line(n: node, l: number)
.input node, text, line
.decl xs(n: node)
xs(n) :- node(_, n, "identifier", _, _), text(n, "x").
.decl before(a: number, b: number)
before(la, lb) :- xs(a), xs(b), a < b, line(a, la), line(b, lb).
.decl last(l: number)
last(l) :- m = max k : { xs(k) }, line(m, l).
.decl first(l: number)
first(l) :- m = min k : { xs(k) }, line(m, l).
.output before, last, first
"#;
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap();
    let lines = [1, 2, 4, 6, 7, 8, 10];
    let want: String = lines.iter().enumerate().flat_map(|(i, a)| lines[i + 1..].iter().map(move |b| format!("{a}\t{b}\n"))).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    let before = read("before");
    let mut got: Vec<&str> = before.lines().collect::<Vec<_>>();
    got.sort_unstable();
    let mut want_sorted: Vec<&str> = want.lines().collect();
    want_sorted.sort_unstable();
    assert_eq!(got, want_sorted);
    assert_eq!(read("last"), "10\n");
    assert_eq!(read("first"), "1\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A text read once (`read_text`) gives the modules `modules` gives and the programs `parse` and `parse_own` give.
#[test]
fn a_text_read_once_is_read_as_each_reader_reads_it() {
    let text = "#include \"python/ast.rr\"\n#include \"python/calls.rr\"\n.decl hit(c: node)\nhit(c) :- kind(c, \"call\").\n.output hit\n";
    let shown = |p: &super::Program| -> Vec<String> { p.rules.iter().map(ToString::to_string).collect() };
    let modules = super::modules(text).unwrap().expect("modules");
    let once = super::read_text(text).unwrap().modules().expect("modules");
    assert_eq!((once.files, once.heads, once.declared), (modules.files, modules.heads, modules.declared));
    let (whole, own) = (super::parse(text).unwrap(), super::parse_own(text).unwrap());
    assert_eq!(shown(&super::read_text(text).unwrap().lower(true).unwrap()), shown(&whole));
    let read_own = super::read_text(text).unwrap().lower(false).unwrap();
    assert_eq!((shown(&read_own), read_own.decls.clone()), (shown(&own), own.decls.clone()));
    assert!(own.rules.len() < whole.rules.len(), "own reads the libraries' declarations, not their rules");
}

/// A string with nothing to escape is quoted whole; one with something, char by char, as before.
#[test]
fn quoting_escapes_only_what_needs_it() {
    assert_eq!(super::quote("plain text é"), "\"plain text é\"");
    assert_eq!(super::quote("a\"b\\c\nd\te\u{1}"), "\"a\\\"b\\\\c\\nd\\te\\u{1}\"");
}

// ---------------------------------------------------------------- tests in rule files (decision 0130)

/// `text` with each `.test` … `.end` block's lines blank: its program, were tests part of it, would differ.
fn without_tests(text: &str) -> String {
    let mut inside = false;
    text.lines()
        .map(|l| {
            let t = l.trim_start();
            let blank = inside || t.starts_with(".test ");
            if t.starts_with(".test ") {
                inside = true;
            } else if t == ".end" {
                inside = false;
            }
            if blank { "" } else { l }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn files_with_tests() -> Vec<PathBuf> {
    let files: Vec<PathBuf> = rr_files().into_iter().filter(|f| super::testing::may_hold_tests(&std::fs::read_to_string(f).unwrap())).collect();
    assert!(files.len() >= 2, "the B101 pack rule and the spike carry tests: {files:?}");
    files
}

/// Every test a rule file carries passes (`rerail-engine test-rr` over `docs/rule-syntax/`).
#[test]
fn rule_files_pass_their_own_tests() {
    let work = crate::code_rule_dsl::prefilter::tests::temp().with_extension("rr-tests");
    let mut failed = Vec::new();
    let mut ran = 0;
    for (k, file) in files_with_tests().iter().enumerate() {
        for (j, test) in super::testing::tests(file).unwrap().iter().enumerate() {
            ran += 1;
            match super::testing::run(file, test, &work.join(format!("{k}-{j}"))) {
                Ok(outcome) if outcome.passed() => {}
                Ok(outcome) => failed.push(format!("{}::{}: {:?} (findings {:?})", file.display(), test.name, outcome.failures, outcome.findings)),
                Err(e) => failed.push(format!("{}::{}: {e}", file.display(), test.name)),
            }
        }
    }
    let _ = std::fs::remove_dir_all(&work);
    assert!(ran >= 6, "{ran} tests");
    assert!(failed.is_empty(), "{failed:#?}");
}

/// A test is no part of its file's program (decision 0130): the program, and so every key made of it, is the one the
/// file has with its tests' lines blank.
#[test]
fn a_rule_files_tests_are_no_part_of_its_program() {
    for file in files_with_tests() {
        let text = std::fs::read_to_string(&file).unwrap();
        let with = super::parse(&text).unwrap();
        let without = super::parse(&without_tests(&text)).unwrap();
        assert_eq!(format!("{with:?}"), format!("{without:?}"), "{}", file.display());
        assert!(!super::testing::tests(&file).unwrap().is_empty());
    }
}

/// A failed `.expect` names its line and what the run had instead; `exactly` names the findings no `.expect` lists.
#[test]
fn failed_expectations_say_what_the_run_had() {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("rr-test-fail");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.rr");
    std::fs::write(&file, r#".policy "t:fail" revision 1
.language python
.scope include "**/*.py"
.violation Call(at: node) code "C1" "a call"
Call(c) :- kind(c, "call").
.test wrong exactly
    .fixture "a.py" ```
        f()
        g()
        ```
    .decl called(l: number)
    called(l) :- Call(c), line(c, l).
    .expect C1 at "a.py":1
    .expect called(7)
    .expect count C1 = 3
.end
"#).unwrap();
    let tests = super::testing::tests(&file).unwrap();
    let outcome = super::testing::run(&file, &tests[0], &dir.join("work")).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(outcome.failures, [
        "line 14: .expect called(7): called has 2 rows: (1), (2)",
        "line 15: .expect count C1 = 3: counted 2",
        "line 6: exactly: also reported C1 at \"a.py\":2",
    ], "{outcome:?}");
}

/// A test's block is checked as it is parsed: closed by `.end`, not nested, and no `.end` outside one.
#[test]
fn test_blocks_parse_only_whole() {
    for (text, why) in [
        (".test a\n.expect X\n", "`.end`"),
        (".test a\n.test b\n.end\n.end\n", "do not nest"),
        (".fixtures s\n.test b\n.end\n.end\n", "do not nest"),
        (".end\n", "a `.test` or `.fixtures` before its `.end`"),
    ] {
        let error = super::parse(text).map(|_| ()).unwrap_err().to_string();
        assert!(error.contains(why), "{text:?}: {error}");
    }
    assert_eq!(super::testing::dedent("\n    a\n      b\n\n    c\n    "), "a\n  b\n\nc\n");
    // The closing fence says the indent: lines deeper than it keep the rest.
    assert_eq!(super::testing::dedent("\n      a\n        b\n    "), "  a\n    b\n");
    // A closing fence after text: the indent the lines share.
    assert_eq!(super::testing::dedent("\n    a\n      b"), "a\n  b\n");
}

/// A file of `text` in a directory of its own, its tests, and each one's outcome (or its error).
fn run_tests_of(name: &str, text: &str) -> Vec<(String, Result<Vec<String>, String>)> {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension(name);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.rr");
    std::fs::write(&file, text).unwrap();
    let out = match super::testing::tests(&file) {
        Ok(tests) => tests
            .iter()
            .enumerate()
            .map(|(k, t)| (t.name.clone(), super::testing::run(&file, t, &dir.join(format!("w{k}"))).map(|o| o.failures)))
            .collect(),
        Err(e) => vec![(String::new(), Err(e))],
    };
    let _ = std::fs::remove_dir_all(&dir);
    out
}

/// A unit test (decision 0130): `.given r` drops r's rules, so r holds the test's facts alone, and the test reads
/// no file. A file of rules with no `.violation` (a library's) is tested so too. `.expect only` names every row.
#[test]
fn a_unit_test_gives_a_relation_its_rows_and_reads_no_file() {
    let library = r#"
.decl seed(x: symbol)
seed("from the rule").
.decl edge(a: symbol, b: symbol)
edge(x, "end") :- seed(x).
.decl reach(a: symbol, b: symbol)
reach(a, b) :- edge(a, b).
reach(a, c) :- reach(a, b), edge(b, c).
.test given_edges uses chain
    .expect only reach("a", "b")
    .expect only reach("b", "c")
    .expect only reach("a", "c")
    .expect no reach(_, "end")
.end
.fixtures chain
    .given edge
    edge("a", "b").
    edge("b", "c").
.end
.test only_names_every_row
    .given edge
    edge("a", "b").
    edge("b", "c").
    .expect only reach("a", "b")
.end
.test the_rules_left_alone
    .expect only reach("from the rule", "end")
    .expect count reach = 1
.end
"#;
    let got = run_tests_of("rr-unit", library);
    assert_eq!(got, [
        ("given_edges".to_owned(), Ok(vec![])),
        ("only_names_every_row".to_owned(), Ok(vec!["line 24: .expect only reach: also 2: (a, c), (b, c)".to_owned()])),
        ("the_rules_left_alone".to_owned(), Ok(vec![])),
    ]);
}

/// What a test may not do is refused with its line: give a core relation, give one no rule declares, use a set that
/// is not there, or have two fixtures at one path.
#[test]
fn a_test_that_cannot_run_is_refused_with_its_line() {
    let head = ".decl r(x: symbol)\nr(\"a\").\n";
    for (test, why) in [
        (".test t\n    .given kind\n.end\n", "t.rr:4: .given kind: a core relation is the engine's"),
        (".test t\n    .given nothing\n.end\n", "t.rr:4: .given nothing: no rule declares it"),
        (".test t uses absent\n.end\n", "t.rr:3: no fixture set named absent"),
        (".test t\n    .fixture \"a.py\" ```\n        x\n        ```\n    .fixture \"a.py\" ```\n        y\n        ```\n.end\n", "t.rr:3: two fixtures at \"a.py\""),
        (".test t\n    .expect r(\"a\"\n.end\n", "t.rr:4: `.expect"),
    ] {
        let got = run_tests_of("rr-refused", &format!("{head}{test}"));
        let error = match &got[0].1 { Err(e) => e.clone(), Ok(f) => format!("ran: {f:?}") };
        assert!(error.contains(why), "{test:?}: {error}");
    }
}

/// A test printed (`print_test`, as the converter prints a case) reads back as written: its fixture's bytes, however
/// many backticks and blank lines they hold, and its expectations.
#[test]
fn a_printed_test_reads_back_as_written() {
    // Every line indented: the closing fence keeps it.
    let content = "  x = ```a```\n\n    y = \"````\"\n";
    let text = format!(
        ".decl r(x: symbol)\nr(\"a\").\n{}",
        super::testing::print_test("printed", false, &[("src/a.py".to_owned(), content.to_owned())], &["no _ at \"src/a.py\"".to_owned(), "r(\"a\")".to_owned()])
    );
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("rr-printed");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("t.rr"), &text).unwrap();
    let tests = super::testing::tests(&dir.join("t.rr")).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(tests.len(), 1, "{text}");
    assert_eq!(tests[0].fixtures, [("src/a.py".to_owned(), content.as_bytes().to_vec())], "{text}");
    assert_eq!(tests[0].expects.iter().map(ToString::to_string).collect::<Vec<_>>(), ["no _ at \"src/a.py\"", "r(\"a\")"]);
}

/// `flatten` keeps a file's text as written (`program_text`), so its tests and fixture sets go with it.
#[test]
fn a_flattened_file_keeps_its_tests() {
    let file = rule_syntax().join("spikes/rr-tests/print-calls.rr");
    let flat = crate::code_rule_dsl::tools::flatten(&file).unwrap();
    let blocks = super::testing::blocks(&std::fs::read_to_string(&file).unwrap());
    assert!(blocks.len() >= 6 && blocks.iter().any(|b| b.starts_with(".fixtures logging_app")), "{blocks:?}");
    assert_eq!(super::testing::blocks(&flat), blocks);
    assert_eq!(format!("{:?}", super::parse(&flat).unwrap()), format!("{:?}", super::parse(&without_tests(&flat)).unwrap()));
}

/// `convert-rr --cases` prints a draft's cases as tests: a caught case expects a finding in its file, a passed one
/// none. They are printed for a rule that needs a hand-written port too, and read back one test a case.
#[test]
fn converted_cases_are_tests() {
    let rule = repo().join("packs/rerail/rules/rerail-fixed-value-lists.json");
    let cases = repo().join("fixtures/rerail/rerail-fixed-value-lists.cases.json");
    let report = crate::code_rule_dsl::tools::convert_rr_with(&[(rule, Some(cases.clone()))]);
    let tests = report[0]["tests"].as_str().unwrap_or_else(|| panic!("{report}"));
    let n: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&cases).unwrap()).unwrap();
    let (caught, passed) = (n["caught"].as_array().unwrap().len(), n["passed"].as_array().unwrap().len());
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("rr-cases");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("t.rr"), tests).unwrap();
    let read = super::testing::tests(&dir.join("t.rr")).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(read.len(), caught + passed);
    assert_eq!(read.iter().filter(|t| t.expects[0].negated).count(), passed);
    let first = &n["caught"][0];
    assert_eq!(read[0].fixtures, [(first["path"].as_str().unwrap().to_owned(), first["content"].as_str().unwrap().as_bytes().to_vec())]);
}

/// Aggregate demand's stratification shortcut (`aggregate_demand::optimize`): every data-layer policy lowers to the
/// program it lowers to with each candidate stratified whole.
#[test]
fn aggregate_demand_keeps_the_candidates_a_whole_stratification_keeps() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/data-layer");
    let mut paths: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "rr")).collect();
    paths.sort();
    let (mut read, mut demanded) = (0, 0);
    for path in &paths {
        let text = std::fs::read_to_string(path).unwrap();
        let lowered = |full: bool| {
            super::aggregate_demand::FULL_CHECKS.with(|c| c.set(full));
            let p = super::read_text(&text).and_then(|t| t.lower(true)).map(|p| super::print(&p).unwrap());
            super::aggregate_demand::FULL_CHECKS.with(|c| c.set(false));
            p
        };
        let (Ok(short), Ok(whole)) = (lowered(false), lowered(true)) else { continue };
        read += 1;
        demanded += usize::from(short.contains("rr_demand_agg_"));
        assert_eq!(short, whole, "{}", path.display());
    }
    assert!(read > 20 && demanded > 3, "{read} policies read, {demanded} with aggregate demand");
}

/// Lowering is idempotent: each program file under `dirs` (shard `k` of `n`, by its place in name order), printed and
/// lowered again, prints as the same text, statement for statement and in order.
/// A file that is no program of its own (a library module's part) is passed over. `RERAIL_IDEMPOTENCY_DUMP=<dir>`
/// writes both prints of each that differs.
fn lowers_back(dirs: &[&str], k: usize, n: usize) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax");
    let mut paths: Vec<_> = dirs
        .iter()
        .flat_map(|d| std::fs::read_dir(root.join(d)).unwrap().map(|e| e.unwrap().path()))
        .filter(|p| p.extension().is_some_and(|e| e == "rr"))
        .collect();
    paths.sort();
    let (mut read, mut differ) = (0, Vec::new());
    for path in paths.iter().skip(k).step_by(n) {
        let Ok((program, _)) = super::read_file(path) else { continue };
        let Ok(printed) = super::print(&program) else { continue };
        read += 1;
        let again = match super::read_text(&printed).and_then(|t| t.lower(true)).and_then(|p| super::print(&p)) {
            Ok(again) => again,
            Err(e) => {
                differ.push(format!("{}: its print does not lower: {e}", path.display()));
                continue;
            }
        };
        if again != printed {
            if let Some(dir) = std::env::var_os("RERAIL_IDEMPOTENCY_DUMP") {
                let name = path.file_stem().unwrap().to_string_lossy().into_owned();
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(std::path::Path::new(&dir).join(format!("{name}.first.rr")), &printed).unwrap();
                std::fs::write(std::path::Path::new(&dir).join(format!("{name}.again.rr")), &again).unwrap();
            }
            let at = printed.lines().zip(again.lines()).position(|(a, b)| a != b).unwrap_or(printed.lines().count().min(again.lines().count()));
            differ.push(format!("{}: line {at}: {:?} / {:?}", path.display(), printed.lines().nth(at), again.lines().nth(at)));
        }
    }
    assert!(read > 0, "no program read in {dirs:?}");
    assert!(differ.is_empty(), "{} of {read} lower to another program:\n{}", differ.len(), differ.join("\n"));
}

#[test]
fn lowering_is_idempotent_data_layer() {
    lowers_back(&["data-layer"], 0, 1);
}

#[test]
fn lowering_is_idempotent_python_0() {
    lowers_back(&["packs/python"], 0, 8);
}

#[test]
fn lowering_is_idempotent_python_1() {
    lowers_back(&["packs/python"], 1, 8);
}

#[test]
fn lowering_is_idempotent_python_2() {
    lowers_back(&["packs/python"], 2, 8);
}

#[test]
fn lowering_is_idempotent_python_3() {
    lowers_back(&["packs/python"], 3, 8);
}

#[test]
fn lowering_is_idempotent_python_4() {
    lowers_back(&["packs/python"], 4, 8);
}

#[test]
fn lowering_is_idempotent_python_5() {
    lowers_back(&["packs/python"], 5, 8);
}

#[test]
fn lowering_is_idempotent_python_6() {
    lowers_back(&["packs/python"], 6, 8);
}

#[test]
fn lowering_is_idempotent_python_7() {
    lowers_back(&["packs/python"], 7, 8);
}

#[test]
fn lowering_is_idempotent_javascript_typescript() {
    lowers_back(&["packs/javascript", "packs/typescript"], 0, 1);
}

#[test]
fn lowering_is_idempotent_other_packs() {
    lowers_back(&["packs/react", "packs/node", "packs/rust", "packs/vue", "packs/sql", "packs/next", "packs/php", "packs/blade", "packs/complexity", "packs/rr"], 0, 1);
}

/// `unreached_dropped`: an included file's clauses that nothing reaches are dropped before they are checked and
/// lowered. For every `.rr` program under `dirs` that reads (the data layer's policies, and the packs', a test for
/// each group within the time a test may take), `front::prune` of the program lowered from the rest is the program lowered whole, pruned: the same rules (each helper of the lowering's, numbered as it is met,
/// as many), natives and outputs, the same inputs read, and the same declarations of the relations those rules name.
fn dropping_unreached_clauses_in(dirs: &[&str], skip: &[&str], part: (usize, usize)) -> usize {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax");
    let mut pending: Vec<PathBuf> = dirs.iter().map(|d| manifest.join(d)).collect();
    let mut paths = Vec::new();
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if !skip.iter().any(|s| path == manifest.join(s)) {
                    pending.push(path);
                }
            } else if path.extension().is_some_and(|e| e == "rr") {
                paths.push(path);
            }
        }
    }
    // The `part.0`th of `part.1` parts, by position in name order: a group too long for one test is split.
    paths.sort();
    let counts: Vec<(usize, usize)> = paths.iter().enumerate().filter(|(i, _)| i % part.1 == part.0).map(|(_, p)| p).filter_map(|path| {
        let path = path.as_path();
        let Ok((statements, _)) = program(&path) else { return None };
        let Ok(whole) = lower_files_dropping(&path, statements.clone(), false) else { return None };
        let own = path.canonicalize().unwrap();
        let clauses = |s: &Files| s.iter().flat_map(|(_, items)| items).filter(|(i, _)| matches!(i, syntax::Item::Clause { .. })).count();
        let (n, kept) = (clauses(&statements), clauses(&unreached_dropped(&own, statements.clone())));
        let less = lower_files_dropping(&path, statements, true).unwrap_or_else(|e| panic!("{}: lowered whole, not with its unreached clauses dropped: {e}", path.display()));
        // The lowering numbers its helpers (`rr_up9`, `rr_agg7`) in the order it meets them: renumbered by their
        // first appearance in the pruned rules, as dropped clauses no longer take numbers before them.
        let helper = regex::Regex::new(r"\brr_([a-z_]*[a-z])([0-9]+)\b").unwrap();
        let shape = |mut p: Program| {
            // Pruned as a corpus run prunes it: the requests the engine reads are outputs where a rule makes them
            // (`corpus::run_observed`), those a library derives (embeds, loads) and those lowering makes of a body's
            // file and guest reads (`rr_dir_req`, `rr_embed_req` …), which the engine answers for every reader.
            let requests = super::LIBRARY_DERIVED.iter().copied().chain(super::WORLD.iter().map(|(_, req)| *req)).chain(["rr_embed_req", "rr_parse_req", "rr_splice_req"]);
            for rel in requests {
                if p.rules.iter().any(|r| r.head.rel == rel) && !p.outputs.iter().any(|o| o == rel) {
                    p.outputs.push(rel.to_owned());
                }
            }
            let p = front::prune(p);
            let mut names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
            let mut renumber = |s: &str| -> String {
                helper.replace_all(s, |c: &regex::Captures| {
                    let n = names.len();
                    names.entry(c[0].to_owned()).or_insert_with(|| format!("rr_{}#{n}", &c[1])).clone()
                }).into_owned()
            };
            // A violation's rule for `@place` values (`rr_v0 < -1`) is made when any rule of the program makes one
            // (`lower`'s `place_values`): an unreached clause made it for the whole program, where no reached
            // rule makes the value it reads.
            let place_variant = |r: &front::Rule| r.head.rel.as_str() == "violation" && r.head.args.iter().any(|a| matches!(a, Term::Call(n, _) if n == "place_path"));
            // A helper's rules (`rr_agg7`, its columns its fresh variables in the order of their names, which the
            // numbers change) are compared by their number for each helper; every other rule exactly.
            let numbered = |rel: &str| helper.is_match(rel);
            let mut rules: Vec<String> = p.rules.iter().filter(|r| !place_variant(r)).map(|r| {
                let text = renumber(&format!("{r:?}"));
                if numbered(r.head.rel.as_str()) { format!("helper {}", renumber(r.head.rel.as_str())) } else { text }
            }).collect();
            rules.sort();
            let named: BTreeSet<String> = p.rules.iter().flat_map(|r| std::iter::once(r.head.rel.to_string()).chain(r.body.iter().filter_map(Literal::atom).map(|a| a.rel.to_string()))).collect();
            let mut decls: Vec<String> = p.decls.iter().filter(|(r, _)| named.contains(*r)).map(|(r, c)| if numbered(r) { format!("{}/{}", renumber(r), c.len()) } else { renumber(&format!("{r}{c:?}")) }).collect();
            decls.sort();
            let natives: BTreeSet<String> = p.natives.keys().map(|n| renumber(n)).collect();
            // The inputs the rules read: the lowering declares one (`node_order`, `rr_inside_<set>`) for each clause
            // that reads it, and an unreached clause's stays among the inputs after `prune`, read by none.
            let text = format!("{:?}{:?}", p.rules, p.natives);
            let mut inputs: Vec<String> = p.inputs.iter().filter(|i| text.contains(&format!("\"{i}\""))).cloned().collect();
            inputs.sort();
            let mut outputs = p.outputs.clone();
            outputs.sort();
            (rules, decls, natives, outputs, inputs)
        };
        let (a, b) = (shape(less), shape(whole));
        if a != b {
            let only = |x: &Vec<String>, y: &Vec<String>| x.iter().filter(|r| !y.contains(r)).take(3).cloned().collect::<Vec<_>>();
            panic!(
                "{}: its pruned program changed with its unreached clauses dropped\n rules only dropped: {:?}\n rules only whole: {:?}\n decls only dropped: {:?}\n decls only whole: {:?}\n natives {:?} vs {:?}\n outputs {:?} vs {:?}\n inputs {:?} vs {:?}",
                path.display(), only(&a.0, &b.0), only(&b.0, &a.0), only(&a.1, &b.1), only(&b.1, &a.1), a.2, b.2, a.3, b.3,
                (a.0.len(), a.1.len(), a.4.len(), only(&a.4, &b.4)), (b.0.len(), b.1.len(), b.4.len(), only(&b.4, &a.4))
            );
        }
        Some((n, n - kept))
    }).collect();
    let (compared, all, dropped) = (counts.len(), counts.iter().map(|c| c.0).sum::<usize>(), counts.iter().map(|c| c.1).sum::<usize>());
    eprintln!("{compared} programs; {dropped} of {all} clauses dropped");
    compared
}

#[test]
fn dropping_unreached_clauses_prunes_to_the_same_program() {
    let n = dropping_unreached_clauses_in(&["data-layer"], &[], (0, 1));
    assert!(n > 40, "{n} programs compared");
}

#[test]
fn dropping_unreached_clauses_prunes_to_the_same_program_python_pack_0() {
    let n = dropping_unreached_clauses_in(&["packs/python"], &[], (0, 4));
    assert!(n > 50, "{n} programs compared");
}

#[test]
fn dropping_unreached_clauses_prunes_to_the_same_program_python_pack_1() {
    let n = dropping_unreached_clauses_in(&["packs/python"], &[], (1, 4));
    assert!(n > 50, "{n} programs compared");
}

#[test]
fn dropping_unreached_clauses_prunes_to_the_same_program_python_pack_2() {
    let n = dropping_unreached_clauses_in(&["packs/python"], &[], (2, 4));
    assert!(n > 50, "{n} programs compared");
}

#[test]
fn dropping_unreached_clauses_prunes_to_the_same_program_python_pack_3() {
    let n = dropping_unreached_clauses_in(&["packs/python"], &[], (3, 4));
    assert!(n > 50, "{n} programs compared");
}

#[test]
fn dropping_unreached_clauses_prunes_to_the_same_program_script_packs() {
    let n = dropping_unreached_clauses_in(&["packs/javascript", "packs/typescript", "packs/react", "packs/next", "packs/node", "packs/vue"], &[], (0, 1));
    assert!(n > 50, "{n} programs compared");
}

#[test]
fn dropping_unreached_clauses_prunes_to_the_same_program_other_packs() {
    let n = dropping_unreached_clauses_in(&["packs/php", "packs/blade", "packs/rust", "packs/sql", "packs/rr", "packs/complexity"], &[], (0, 1));
    assert!(n > 20, "{n} programs compared");
}

