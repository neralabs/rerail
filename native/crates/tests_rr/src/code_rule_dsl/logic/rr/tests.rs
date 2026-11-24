//! The `.rerail` front end: the rule-syntax files parse and pass `check_rr.py`'s checks, the spellings lower to the
//! engine's forms, and every `.rdl` program translates to `.rerail` that lowers to the same program.

use rerail_store::Canonical as _;
use super::*;
use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;
use crate::code_rule_dsl::logic::program_image::{Lit, RuleRef, TermRef, Version, about::About, analyses};
use std::collections::BTreeSet;
use std::sync::Arc;

/// A program's `.rerail` text, lowered to its version.
fn parse(src: &str) -> Result<Arc<Version>, Error> {
    parse_version(src)
}

/// Whether `name` (or the text it starts) is a helper of stem `stem`, as the walk's end names it (decision 0639 §4):
/// `rr_h_{stem}_…` by its content, `rr_m_{stem}_…` by its module, 128 bits in hex.
fn is_helper(name: &str, stem: &str) -> bool {
    ["rr_h_", "rr_m_"].iter().any(|tier| name.strip_prefix(tier).and_then(|r| r.strip_prefix(stem)).and_then(|r| r.strip_prefix('_')).is_some_and(|r| r.len() >= 32 && r.as_bytes()[..32].iter().all(u8::is_ascii_hexdigit)))
}

/// Whether `text` names a helper of stem `stem` (`is_helper`).
fn names_helper(text: &str, stem: &str) -> bool {
    text.match_indices("rr_").any(|(i, _)| is_helper(&text[i..], stem))
}

/// `v`'s rules whose head's relation passes `keep`, each written out.
fn shown(v: &Version, keep: impl Fn(&str) -> bool) -> Vec<String> {
    v.rules().filter(|r| keep(v.rel_name(r.head().rel()))).map(|r| v.show_rule(r)).collect()
}

/// An alternative of a disjunction holds its own literals alone: `(i = k ; within(i, k))` is a rule with `i = k` and
/// one with `within(i, k)`. The rule `within` wrote outside its pick held both, so the first alternative read only
/// the keys' descendants (django-practice-bulk-not-loop reported `filter(**{field: value})`).
#[test]
fn an_alternative_holds_only_its_own_literals() {
    let p = parse(".decl key(i: node)\nkey(i) :- kind(p, \"pair\"), field(p, \"key\", k), (i = k ; within(i, k)).\n.output key\n").unwrap();
    let rules = shown(&p, |h| h == "key");
    let within = format!("{}(", crate::code_rule_dsl::facts::vtab::syntax::WITHIN);
    assert_eq!(rules.len(), 2, "{rules:?}");
    assert_eq!(rules.iter().filter(|r| r.contains(&within)).count(), 1, "one alternative reads `within`: {rules:?}");
}

/// `v`'s rules whose head's relation passes `keep`.
fn rules_of<'v>(v: &'v Version, keep: impl Fn(&str) -> bool) -> Vec<RuleRef<'v>> {
    v.rules().filter(|r| keep(v.rel_name(r.head().rel()))).collect()
}

/// Rule `r`'s body literals, each written out.
fn body_of(v: &Version, r: RuleRef<'_>) -> Vec<String> {
    r.body().map(|l| v.show_lit(r, l)).collect()
}

/// `v`'s components in T's order, or the first that a negation or an aggregate closes.
fn strata(v: &Version) -> Result<Vec<BTreeSet<String>>, Error> {
    analyses::components_by_name(v, &About::of(v))
}

#[test]
fn include_seed_ignores_quoted_directives_and_preserves_real_includes() {
    let quoted = parse(r##".decl x(s:symbol)
x("#include \"absent.rerail\"").
// #include "absent.rerail"
"##).unwrap();
    assert_eq!(quoted.fact_rows("x").len(), 1);
    let included = parse("#include \"core.rerail\"\n.decl answer(n:node)\nanswer(n) :- kind(n, \"call\").\n").unwrap();
    assert!(included.declares("doc"), "a real include still supplies its declarations");
    assert!(parse("#unknown \"x.rerail\"\n").is_err());
    assert!(parse(".decl broken(\n").is_err(), "the lexical shortcut does not bypass compilation errors");
}

fn finding_contract(source: &str, program: &str) -> Vec<serde_json::Value> {
    let root = crate::code_rule_dsl::fixtures::temp().with_extension("rr-contract");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.py"), source).unwrap();
    let rule = root.join("rule.rerail");
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
        let error = parse(&rule).expect_err("unsupported regex must fail, never report a clean source").to_string();
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
#include "python/ast.rerail"
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
    let rule = root.join("rule.rerail");
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
#include "python/ast.rerail"
#include "python/calls.rerail"
.decl selected(d: number, t: symbol)
selected(d, t) :- source(d, sp), text(sp, t), contains("marker", t).
.violation Seen(at: node, source_text: symbol) "{source_text}"
Seen(n, t) :- selected(d, t), kind(n, "identifier"), docof(n, d).
"#);
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row["message"] == "# marker\nx = y\n"));
}

fn repo() -> PathBuf {
    crate::source_tree::native().join("..")
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

/// The hand-written `.rerail` files: the library, the data-layer rules and the spikes (`site/` is the page's code).
fn rr_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    files(&rule_syntax(), "rerail", &mut out);
    // `typing/programs` holds copies of the engine's typing programs (T's dialect, its own inputs): not rules.
    // `packs/python-main` is generated from all of main's rules (`site/regen_main_pack.sh`), the ones the reader
    // refuses too, so a test over every rule of main sees them all; its own check is `reader-check.txt`.
    let generated = rule_syntax().join("packs/python-main");
    out.retain(|p| !p.starts_with(rule_syntax().join("site")) && !p.starts_with(rule_syntax().join("typing")) && !p.starts_with(&generated));
    out
}

fn short(p: &Path) -> String {
    p.strip_prefix(rule_syntax()).unwrap_or(p).to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/")
}

#[test]
fn rule_syntax_files_parse() {
    let files = rr_files();
    assert!(files.len() >= 20, "{} files", files.len());
    let failures: Vec<String> = files.iter().filter_map(|f| read(f).err().map(|e| format!("{}: {e}", short(f)))).collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// `check_rr.py`: every rule grounded; per program (a file, its includes and `core.rerail`) no relation declared twice
/// nor a built-in constraint declared, and every cycle through `!` or an aggregate allowed by `ordered by`.
#[test]
fn rule_syntax_files_pass_check_rr() {
    let lib: Vec<PathBuf> = rr_files().into_iter().filter(|p| p.starts_with(library())).collect();
    let mut given = BTreeSet::new();
    for f in &lib {
        given.extend(check::fix_relations(&read(f).unwrap().0));
    }
    let mut problems = Vec::new();
    // Phase 0 resolves every file's includes in one run of `bootstrap.rerail`, not one run each.
    let seeds: Vec<_> = rr_files().into_iter().map(|f| crate::code_rule_dsl::logic::plan::corpus::phase0::Seed::File(f.canonical().unwrap())).collect();
    crate::code_rule_dsl::logic::plan::corpus::phase0::graphs(&seeds).unwrap();
    let programs: Vec<(PathBuf, Vec<(PathBuf, Vec<from_tape::Stmt>)>)> = rr_files().into_iter().map(|f| {
        let program = program(&f).unwrap().0;
        (f, program)
    }).collect();
    // A library file another library file includes is a part of that file's program, as a run reads a directory of
    // programs (`tools::logic_command`: "not a program of its own"): alone it has neither the declarations it reads
    // nor the rules that fill its inputs (`git/messages.rerail`, which `git.rerail` includes before declaring
    // `git_message`). It is checked in its includer's program, which reads it in.
    // Canonical both: the files are named from the manifest's directory (`native/..`), the library as the engine finds it.
    let lib_dir = library().canonical().unwrap();
    let parts: BTreeSet<PathBuf> = programs
        .iter()
        .filter(|(f, _)| f.canonical().unwrap().starts_with(&lib_dir))
        // A program's own file is its last (`rr::program`: includes first).
        .flat_map(|(_, program)| program.iter().rev().skip(1).filter_map(|(p, _)| p.canonical().ok()).collect::<Vec<_>>())
        .collect();
    for (f, program) in programs {
        for (line, vars) in check::grounded(&read(&f).unwrap().0, &given) {
            problems.push(format!("{}:{line}: unbound {}", short(&f), vars.join(", ")));
        }
        if parts.contains(&f.canonical().unwrap()) {
            continue;
        }
        let mut program = program;
        let core = library().join("core.rerail").canonical().unwrap();
        if !program.iter().any(|(p, _)| *p == core) {
            program.push((core.clone(), read(&core).unwrap().0));
        }
        let named: Vec<(String, Vec<from_tape::Stmt>)> = program.iter().map(|(p, s)| (short(p), s.clone())).collect();
        problems.extend(check::declarations(&named).into_iter().map(|p| format!("{}: {p}", short(&f))));
        let all: Vec<&from_tape::Stmt> = program.iter().flat_map(|(_, s)| s).collect();
        problems.extend(check::strata(&all).into_iter().map(|p| format!("{}: {p}", short(&f))));
        // A relation read and declared nowhere in the file's program, as check-rr and a run refuse it.
        problems.extend(check::undeclared(&all).into_iter().map(|p| format!("{}: {p}", short(&f))));
    }
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
}

/// The checks find what they are for.
#[test]
fn check_rr_finds_problems() {
    let statements = |src: &str| from_tape::RrFile::read(src).unwrap().statements();
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
    let refs: Vec<&from_tape::Stmt> = cyclic.iter().collect();
    assert_eq!(check::strata(&refs).len(), 1);
    // A negation in a component that reads a world answer is the engine's gated negation (decision 0065 §2a, as
    // `c/includes.rerail`'s `#include_next`); an aggregate there is still refused.
    let world = ".decl dir_entry(d: symbol, n: symbol, k: number)\n.input dir_entry\n.decl a(x: symbol)\n.decl b(x: symbol)\n";
    let gated = statements(&format!("{world}a(x) :- b(x), dir_entry(x, _, 0), !a(x).\nb(x) :- a(x).\n"));
    assert!(check::strata(&gated.iter().collect::<Vec<_>>()).is_empty());
    let counted = statements(&format!("{world}.decl c(x: symbol, n: number)\nc(x, n) :- b(x), dir_entry(x, _, 0), n = count : {{ c(x, _) }}.\nb(x) :- c(x, _).\n"));
    assert_eq!(check::strata(&counted.iter().collect::<Vec<_>>()).len(), 1);
    let twice = vec![("a.rerail".to_owned(), statements(".decl a(x: number)")), ("b.rerail".to_owned(), statements(".decl a(x: number)\n.decl glob(x: symbol)"))];
    assert_eq!(check::declarations(&twice).len(), 2);
}

/// A comparison, as a version writes it.
fn cmp(l: String, op: &str, r: String) -> String {
    format!("{l} {op} {r}")
}

fn call(f: &str, args: Vec<String>) -> String {
    format!("@{f}({})", args.join(", "))
}

fn s(x: &str) -> String {
    serde_json::to_string(x).unwrap()
}

fn v(x: &str) -> String {
    format!("${x}")
}

fn set(items: &[&str]) -> String {
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
    assert_eq!(p.columns_of("name").unwrap(), [("n", "node"), ("s", "sym")].map(|(c, t)| (c.to_owned(), t.to_owned())));
    assert_eq!(p.kindsets()["scope"], vec!["module".to_owned(), "class_definition".to_owned()]);
    assert!(!p.declares("scope") && !p.states_facts("scope"));
    let lattice = p.lattice();
    let best = &lattice["best"];
    assert_eq!((best.keys, best.ty.as_str(), best.merge.as_deref(), best.widen), (1, "sym", Some("shortlex_min"), Some(8)));
    assert_eq!((lattice["cv"].ty.as_str(), lattice["cv"].merge.as_deref()), ("flat", None));
    assert_eq!(ProgramFields::demand(&*p), vec!["private".to_owned(), "cv".to_owned()]);
    let rules: Vec<Vec<String>> = rules_of(&p, |h| h == "private").into_iter().map(|r| body_of(&p, r)).collect();
    let one = || "1".to_owned();
    assert_eq!(rules[0][1..], [cmp(call("prefix", vec![v("x"), s("_")]), "=", one()), cmp(call("glob", vec![v("x"), set(&["*_test", "test_*"])]), "=", "0".to_owned())]);
    assert_eq!(rules[1][1], cmp(call("glob", vec![v("x"), set(&["a*", "b*"])]), "=", one()));
    assert_eq!(rules[2][1], cmp(call("equals", vec![v("x"), set(&["a", "b"])]), "=", one()));
    assert_eq!(rules[3][1], cmp(call("rx", vec![v("x"), s("^(?:[a-z]+)$")]), "=", one()));
    // A disjunction is a rule per alternative.
    assert_eq!(rules.len(), 6);
    let owner = rules_of(&p, |h| h == "owner");
    assert_eq!(body_of(&p, owner[0]), ["nearest($n, $a, scope)"]);
    assert!(matches!(owner[1].body().next().map(|l| l.view()), Some(Lit::Atom(a)) if p.rel_name(a.rel()) == "rr_inside_rr_kind_module"));
    // A count over two literals reads a helper of every variable, its `_`s named (each row counts), grouped by the
    // ones shared (`_` for the body's own); a min over two, the ones shared and its own.
    let helper = |head: &str| -> Vec<String> {
        let r = rules_of(&p, |h| h == head)[0];
        r.body().filter_map(|l| if let Lit::Agg { atom, .. } = l.view() { Some(p.show_atom(r, atom)) } else { None }).collect()
    };
    let [kinds] = &helper("kinds")[..] else { panic!("{:?}", helper("kinds")) };
    let (name, args) = kinds.split_once('(').unwrap();
    assert!(is_helper(name, "agg") && args == "_, $n, _, _, _, _)", "{kinds}");
    assert_eq!(p.columns_of(name).unwrap().len(), 6);
    let first = helper("first");
    assert_eq!(first.len(), 2);
    assert!(first[0].contains("$j") && first[0].contains("$n") && !first[0].contains("$x"), "{first:?}");
    let head_arg = |rel: &str, k: usize| {
        let r = rules_of(&p, |h| h == rel)[0];
        p.show_term(r, r.head().args().nth(k).unwrap())
    };
    assert_eq!(head_arg("best", 1), call("cat", vec![s("a"), v("x")]));
    assert_eq!(head_arg("cv", 1), "top");
}

/// A context built from a rule's literals that sit in the rule's own recursion (another relation of it, as a
/// translator's demand is) is widened past them: the aggregate it narrows would otherwise close the cycle
/// (`pyi051_has_of` with `pyi051_demand_has_of`, main's pack).
#[test]
fn a_context_in_a_negations_cycle_is_widened() {
    let p = parse(concat!(
        ".decl e(n: node, x: symbol, s: number)\n.decl seed(n: node)\n.input e, seed\n",
        ".decl node(d: number, n: node, kind: symbol, start: number, end: number)\n.input node\n",
        ".decl ask(n: node)\n.decl cb(n: node)\n.output cb\n",
        "ask(n) :- seed(n).\n",
        "ask(m) :- cb(n), e(n, x, s), e(m, x, s).\n",
        "cb(n) :- ask(n), count : { e(m, \"x\", _), m > n } = 0.\n",
    ))
    .unwrap();
    strata(&p).unwrap_or_else(|e| panic!("{e}"));
    let ctx: Vec<String> = shown(&p, |h| is_helper(h, "ctx"));
    assert!(!ctx.is_empty() && ctx.iter().all(|r| !r.contains("ask(")), "{ctx:?}");
}

/// A count below a bound (`rank_form`, decision 0361 step 7), end to end: each statement's place among its module's,
/// comments not counted (python/ast's `body_stmt`).
#[test]
fn a_count_below_a_bound_numbers_the_statements_past_comments() {
    let rows = finding_contract("a = 1\n# c\nb = 2\n# d\n# e\nc = 3\n", r#"
.policy "contract:rank" revision 1
.language python
.violation At(at: node, i: number) code "RANK001"
    "{i}"
At(s, i) :- kind(m, "module"), child(m, s, _), !kind(s, "comment"),
    i = count : { child(m, s2, _), !kind(s2, "comment"), s2 < s }.
"#);
    let got: BTreeSet<(i64, String)> = rows.iter().map(|r| (r["line"].as_i64().unwrap(), r["message"].as_str().unwrap().to_owned())).collect();
    assert_eq!(got, [(1, "0".to_owned()), (3, "1".to_owned()), (6, "2".to_owned())].into_iter().collect());
}

/// Counting above (`s2 > s`): each statement's count of later ones, comments not counted.
#[test]
fn a_count_above_a_bound_counts_the_later_statements() {
    let rows = finding_contract("a = 1\n# c\nb = 2\nc = 3\n", r#"
.policy "contract:rank-above" revision 1
.language python
.violation At(at: node, i: number) code "RANK002"
    "{i}"
At(s, i) :- kind(m, "module"), child(m, s, _), !kind(s, "comment"),
    i = count : { child(m, s2, _), !kind(s2, "comment"), s2 > s }.
"#);
    let got: BTreeSet<(i64, String)> = rows.iter().map(|r| (r["line"].as_i64().unwrap(), r["message"].as_str().unwrap().to_owned())).collect();
    assert_eq!(got, [(1, "2".to_owned()), (3, "1".to_owned()), (4, "0".to_owned())].into_iter().collect());
}

/// An aggregate's context leaves out the clause's own head where the rest binds its variables: through it, the
/// aggregate would sit in the head's recursion (lazy-analyzer.rerail's `la_cb`).
#[test]
fn a_context_leaves_out_the_clause_head() {
    let p = parse(concat!(
        ".decl e(n: node, x: symbol, s: number)\n.decl seed(n: node)\n.input e, seed\n",
        ".decl cb(n: node)\n.output cb\n",
        "cb(n) :- seed(n).\n",
        "cb(n) :- e(n, x, s), cb(n), count : { e(n, x, s2), s2 > s } = 0.\n",
    ))
    .unwrap();
    strata(&p).unwrap_or_else(|e| panic!("{e}"));
    let ctx: Vec<String> = shown(&p, |h| is_helper(h, "ctx"));
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
    let ctx: Vec<String> = shown(&p, |h| is_helper(h, "ctx"));
    assert!(!ctx.is_empty(), "{:#?}", shown(&p, |_| true));
    // Every rule of the context reaches v from c through the comprehension's target t: the clause `child(c, fc, _)`
    // joined to `field(fc, "left", t)`, its variables named by their places (a helper named by its content).
    let joined = |r: &str| -> bool {
        let Some(at) = r.find("child($") else { return false };
        let args: Vec<&str> = r[at + 6..].split(')').next().unwrap_or_default().split(", ").collect();
        args.len() == 3 && r.contains(&format!("field({}, \"left\", $", args[1]))
    };
    assert!(ctx.iter().all(|r| joined(r)), "{ctx:#?}");
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
    strata(&p).unwrap_or_else(|e| panic!("{e}"));
    let ctx: Vec<String> = shown(&p, |h| is_helper(h, "ctx"));
    assert!(!ctx.is_empty(), "the parse's demand has a context");
    assert!(ctx.iter().all(|r| !r.contains("root(")), "{ctx:?}");
}

/// A literal over a file's nodes tied to the asked variables only through their document tests that some node is
/// in the same file: in the context it was every pair of the file's nodes (`latest_alias`'s `max` beside
/// `docof(a, d), docof(e, d)`, 18 M rows on 100 files), so it is left out. A relation of the document itself stays.
#[test]
fn a_context_leaves_out_a_files_nodes_tied_only_through_the_document() {
    let p = parse(concat!(
        ".decl site(e: node, x: symbol)\n.decl alias_at(a: node, x: symbol)\n.decl docof(n: node, d: number)\n",
        ".decl visit_index(n: node, i: number)\n.decl earlier(m: node, n: node)\n.decl first(d: number)\n",
        ".input site, alias_at, docof, visit_index, earlier, first\n",
        ".decl alias(a: node, x: symbol)\nalias(a, x) :- alias_at(a, x).\n",
        ".decl last(e: node, a: node)\n.output last\n",
        "last(e, a) :- site(e, x), k = max j : { alias(b, x), earlier(b, e), visit_index(b, j) },\n",
        "    alias(a, x), visit_index(a, k), docof(a, d), docof(e, d), first(d).\n",
    ))
    .unwrap();
    strata(&p).unwrap_or_else(|e| panic!("{e}"));
    let ctx: Vec<String> = shown(&p, |h| is_helper(h, "ctx") || is_helper(h, "cover"));
    assert!(ctx.iter().any(|r| is_helper(r, "ctx")), "the max has a context: {ctx:?}");
    assert!(ctx.iter().all(|r| !r.contains("visit_index(") && r.matches("docof(").count() <= 1), "{ctx:?}");
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
    strata(&p).unwrap_or_else(|e| panic!("{e}"));
    let shown = |stem: &str| -> Vec<String> { shown(&p, |h| is_helper(h, stem)) };
    let (ctx, cover) = (shown("ctx"), shown("cover"));
    assert!(!ctx.is_empty(), "the parse's demand has a context");
    assert!(ctx.iter().all(|r| r.contains("\"call\"") && names_helper(r, "cover") && !r.contains("\"name\"")), "{ctx:?}");
    assert!(!cover.is_empty() && cover.iter().all(|r| r.contains("\"name\"") && !r.contains("\"call\"")), "{cover:?}");
    assert!(p.declared_names().iter().any(|rel| is_helper(rel, "cover") && p.columns_of(rel).unwrap().len() == 1), "a relation of s alone");
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
    strata(&p).unwrap_or_else(|e| panic!("{e}"));
    let cover: Vec<String> = shown(&p, |h| is_helper(h, "cover"));
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
    strata(&p).unwrap_or_else(|e| panic!("{e}"));
    let cover: Vec<String> = shown(&p, |h| is_helper(h, "cover"));
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
    let rule = shown(&p, |h| h == "hit").remove(0);
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
    let text = shown(&p, |h| h == "a").join("\n");
    assert!(text.contains("(?:.*)") && !text.contains("(?<z>"), "{text}");
}

#[test]
fn bound_family_keeps_its_contract_when_printed_and_rejects_incompatible_keys() {
    let source = r#"
.decl selected(r: symbol, k: number)
.decl ask(r: symbol, k: number) extensible
.decl complete(r: symbol, k: number)
.decl member(r: symbol, k: number, value: symbol)
.pragma "bound-family" "ask complete member"
.decl empty(r: symbol, k: number)
empty(r,k) :- selected(r,k), !member(r,k,_).
.output empty
"#;
    // The reader asks for its keys (`ask`, from `selected`), and its negation reads `complete` at them.
    let check = |p: &Arc<Version>| {
        let demand = rules_of(p, |h| h == "ask");
        assert_eq!(demand.len(), 1, "{:?}", shown(p, |h| h == "ask"));
        let first = demand[0].body().next().and_then(|l| l.atom()).map(|a| p.rel_name(a.rel()).to_owned());
        assert_eq!(first.as_deref(), Some("selected"), "{:?}", shown(p, |h| h == "ask"));
        let query = rules_of(p, |h| h == "empty");
        let complete = query[0].body().filter(|l| matches!(l.view(), Lit::Atom(a) if !a.neg() && p.rel_name(a.rel()) == "complete")).count();
        assert_eq!(complete, 1, "{:?}", shown(p, |h| h == "empty"));
    };
    let program = parse(source).unwrap();
    check(&program);
    check(&parse(&print_version(&program).unwrap()).unwrap());
    for (from, to, why) in [
        ("ask complete member", "ask complete absent", "undeclared relation absent"),
        ("complete(r: symbol, k: number)", "complete(r: symbol)", "needs the key columns"),
        ("member(r: symbol, k: number", "member(r: symbol, k: symbol", "needs the key columns"),
    ] {
        let error = parse(&source.replace(from, to)).unwrap_err().to_string();
        assert!(error.contains(why), "{error}");
    }
}

/// What is not lowered yet says so by name. A `must` head needs its relation declared `must, may`; a `.violation`
/// at a span or loc whose values can be spans the engine does not make lowers (such a row has no `docof`, and so no
/// finding).
#[test]
fn unlowered_constructs_are_named() {
    let src = ".decl a(x: number)\na(x) :- a(y), x = mean z : { a(z) }.";
    let e = parse(src).unwrap_err().to_string();
    assert!(e.contains("not lowered yet") && e.contains("mean"), "{src}: {e}");
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
    assert!(p.output_names().contains(&"BAD".to_owned()) && p.output_names().contains(&"violation".to_owned()));
    let rule = |rel: &str| shown(&p, |h| h == rel).into_iter().next().unwrap_or_default();
    assert_eq!(rule("violation"), "violation(\"data-layer:p\", \"BAD\", $rr_v1, $rr_p, $rr_l, $rr_c) :- BAD($rr_v0, $rr_v1), node($rr_d, $rr_v0, _, _, _), doc($rr_d, $rr_p, _), line($rr_v0, $rr_l), col($rr_v0, $rr_c)");
    let bad = rule("BAD");
    for part in ["$j = @add($i, 1)", "@concat($t, @to_string(@strlen($t)))", "!rr_inside_fdef($c, _)"] {
        assert!(bad.contains(part), "{part} in {bad}");
    }
    assert!(shown(&p, |h| h == "drop").is_empty(), "a fix is not run");
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
    let rules: Vec<String> = shown(&p, |h| h == "violation");
    assert!(rules.iter().any(|r| r.contains("\"@next/next/no-page-custom-font\"") && r.contains("font($rr_v0, _)")), "{rules:?}");
    assert!(rules.iter().any(|r| r.contains("SEC($rr_v0, $rr_v1)") && r.contains("docof($rr_v0, $rr_d)")), "{rules:?}");
    assert!(rules.iter().any(|r| r.contains("rr_place_missing($rr_p, $rr_l, _)")), "{rules:?}");
    assert!(rules_of(&p, |h| h == "rr_place_missing").iter().any(|r| matches!(r.head().args().nth(1), Some(TermRef::Int(1)))));
    let settings: Vec<String> = ProgramFields::settings(&*p).iter().map(|(k, t)| format!("{k} {t}")).collect();
    assert!(settings.contains(&"suppress font eslint \"@next/next/no-page-custom-font\"".to_owned()), "{settings:?}");
    assert!(settings.contains(&"suppress font phpcs \"A.B\", \"A.B.C\", \"A.D\"".to_owned()), "{settings:?}");
    assert!(settings.contains(&"code_in SEC \"SEC\\\\d+\"".to_owned()), "{settings:?}");
    // The printed program reads back as itself.
    let printed = print_version(&p).unwrap();
    assert_eq!(ProgramFields::settings(&*parse(&printed).unwrap()), ProgramFields::settings(&*p), "{printed}");
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
    let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
    assert_eq!(read("joined"), [["12-7-12"], ["7-12-7"]]);
    assert_eq!(read("num"), [["12", "12", "2", "12", "10"], ["7", "7", "2", "10", "7"]]);
    assert_eq!(read("fl"), [["1.5", "3"], ["1e3", "2000"]]);
    assert_eq!(read("neg"), [["-2.25"]]);
    assert!(read("assigns").last().and_then(|r| r.last()).is_some_and(|v| v == "3"), "{:?}", read("assigns"));
    // `sum strlen(v) * 2 : { kv(_, v) }` sums over kv's four rows, a twice (Soufflé's tuples): 8.
    assert_eq!(read("agg"), [["a,b,c,a", "10", "8", ""]]);
    assert_eq!(read("prod"), [["24", "1"]]);
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
.decl ks(k: number, s: number)
ks(1, 20). ks(1, 10). ks(2, 30).
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
    // (A min or max over symbols is refused: `front::tests::named_errors`.)
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
    assert_eq!(read("lo"), [["1", "3"], ["2", "4"]]);
    assert_eq!(read("hi"), [["1", "9"], ["2", "4"]]);
    assert_eq!(read("one"), [["1"], ["2"]]);
    assert_eq!(read("up"), [["a", "3", ""], ["a.b.c", "1", "a.b"], ["a.b.c", "2", "a"]]);
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
    let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
    assert_eq!(read("near"), [["f", "block"], ["g", "function_definition"]]);
    assert_eq!(read("out_of_loop"), [["h"]]);
    assert_eq!(read("fn_with_call"), [["outer"]]);
    assert_eq!(read("own"), [["g", "inner"]]);
    assert_eq!(read("lonely"), [["f"], ["g"]]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two versions of a program, compared field by field: the first difference.
fn difference(a: &Version, b: &Version) -> Option<String> {
    fn fields(p: &Version) -> Vec<(&'static str, String)> {
        let mut inputs = p.input_names();
        inputs.sort();
        let decls: Vec<(String, Vec<(String, String)>)> = p.declared_names().into_iter().map(|r| (r.clone(), p.columns_of(&r).unwrap_or_default())).collect();
        let facts: Vec<(String, Vec<_>)> = p.fact_names().into_iter().map(|r| (r.clone(), p.fact_rows(&r))).collect();
        vec![
            ("decls", format!("{decls:#?}")),
            ("lattice", format!("{:#?}", p.lattice())),
            ("facts", format!("{facts:#?}")),
            ("rules", format!("{:#?}", shown(p, |_| true))),
            // Inputs as a set: the lowering declares `node_order` after the requests it makes (decision 0174).
            ("inputs", format!("{inputs:#?}")),
            ("outputs", format!("{:#?}", p.output_names())),
            ("kindsets", format!("{:#?}", p.kindsets())),
            ("demand", format!("{:#?}", ProgramFields::demand(p))),
            ("policies", format!("{:#?}", ProgramFields::policies(p))),
        ]
    }
    for ((f, x), (_, y)) in fields(a).into_iter().zip(fields(b)) {
        if x != y {
            let at = x.lines().zip(y.lines()).position(|(l, r)| l != r).unwrap_or(0);
            let around = |t: &str| t.lines().skip(at.saturating_sub(3)).take(8).collect::<Vec<_>>().join("\n");
            return Some(format!("{f}: first\n{}\nsecond\n{}", around(&x), around(&y)));
        }
    }
    None
}

/// The authored `.rdl` programs, as (name, text): T's, the pinned goldens, the data-layer programs (each after its
/// prelude) and the serverplan-quality program.
fn authored() -> Vec<(String, String, PathBuf)> {
    let src = crate::source_tree::native().join("src/code_rule_dsl");
    let mut out = Vec::new();
    for dir in ["typing/programs", "logic/plan/golden/programs"] {
        let mut found = Vec::new();
        files(&src.join(dir), "rdl", &mut found);
        out.extend(found.into_iter().map(|p| (format!("{dir}/{}", p.file_name().unwrap().to_string_lossy()), p)));
    }
    out.into_iter().map(|(name, path)| (name, std::fs::read_to_string(&path).unwrap(), path)).collect()
}

/// A lowered `.rerail` program without the requests its lowering derives (`rr_embed_req` for `parsed`, decision 0055;
/// `rr_parse_req`, `rr_splice_req` for tape operations, 0058;
/// `rr_dir_req`, `rr_file_req` for the file system, t21) and the context relations only they read (a context's
/// `rr_ctx` and the `rr_cover` groups it joins): an `.rdl` program
/// asks for nothing, and the requests change no relation the program derives.
fn without_requests(p: &Version) -> Arc<Version> {
    let mut gone: BTreeSet<String> = ["rr_embed_req", "rr_parse_req", "rr_splice_req"].into_iter().map(str::to_owned).chain(super::WORLD.iter().map(|(_, d)| (*d).to_owned())).collect();
    loop {
        let kept = rules_of(p, |h| !gone.contains(h));
        let read: BTreeSet<&str> = kept.iter().flat_map(|r| r.reads()).map(|a| p.rel_name(a.rel())).collect();
        let more: Vec<String> = kept.iter().map(|r| p.rel_name(r.head().rel()).to_owned()).filter(|h| (is_helper(h, "ctx") || is_helper(h, "cover")) && !read.contains(h.as_str()) && !gone.contains(h.as_str())).collect();
        if more.is_empty() {
            break;
        }
        gone.extend(more);
    }
    let mut b = crate::code_rule_dsl::logic::program_image::Builder::derive(p);
    for rel in p.declared_names().iter().filter(|r| gone.contains(*r)) {
        b.fields().undeclare(rel);
    }
    for r in rules_of(p, |h| !gone.contains(h)) {
        b.keep(p, r);
    }
    b.finish().unwrap().0
}

/// Each copied `.rdl` program (T's, until step 31 removes them) translates to `.rerail` that lowers, its `.rerail` beside it
/// is that translation, and the lowered program prints back as itself. (The `.rdl` was also read by the front end's
/// own parser and compared; that parser went with the intermediate forms, 0276 step 6.)
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
        let beside = path.with_extension("rerail");
        if beside.exists() && std::fs::read_to_string(&beside).unwrap() != rr {
            failures.push(format!("{name}: {} is not its translation", beside.display()));
        }
        // The program's own statements: an `#include` passes through the translation as it is (checked above), and
        // what it names is read with the file (`parse_file`), not here.
        let own: String = rr.lines().map(|l| if l.starts_with("#include") { "" } else { l }).collect::<Vec<_>>().join("\n");
        // Printed, the program reads back as itself.
        match parse(&own) {
            Ok(b) => match print_version(&b).and_then(|text| parse(&text)) {
                Ok(c) => failures.extend(difference(&without_requests(&b), &without_requests(&c)).map(|d| format!("{name}, printed: {d}"))),
                Err(e) => failures.push(format!("{name}, printed: {e}")),
            },
            Err(e) => failures.push(format!("{name}: {e}")),
        }
    }
    assert!(failures.is_empty(), "{} programs differ:\n{}", failures.len(), failures.join("\n\n"));
}

/// The engine's own programs (the data-layer programs on their prelude, the serverplan quality program), printed,
/// read back as themselves.
#[test]
fn rr_programs_print_as_themselves() {
    let dir = crate::source_tree::native().join("src/code_rule_dsl/logic/programs/data_layer");
    let prelude = std::fs::read_to_string(dir.join("prelude.rerail")).unwrap();
    let mut found = Vec::new();
    files(&dir, "rerail", &mut found);
    let mut programs: Vec<(String, String)> = found
        .into_iter()
        .filter(|p| !p.ends_with("prelude.rerail"))
        .map(|p| (short(&p), format!("{prelude}\n{}", std::fs::read_to_string(&p).unwrap())))
        .collect();
    let quality = repo().join("gate/data-layer/final-shape/data-layer-serverplan-quality.rerail");
    programs.push((short(&quality), std::fs::read_to_string(&quality).unwrap()));
    let mut failures = Vec::new();
    let mut lowered = 0;
    for (name, text) in programs {
        let Ok(b) = parse(&text) else { continue };
        lowered += 1;
        match print_version(&b).and_then(|text| parse(&text)) {
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
    let text = print_version(&prog).unwrap();
    assert!(text.contains(r#"match("WHEN.*BEGIN", s)"#), "{text}");
    assert!(text.contains(r#"!match("(?i)x(y)", s)"#), "{text}");
    assert!(!text.contains("(?s)"), "{text}");
    assert_eq!(difference(&prog, &parse(&text).unwrap()), None);
    assert_eq!(super::print::search("rx", "a.b"), "(?s:.*)(?:a.b)(?s:.*)");
    assert_eq!(super::print::search("irx", "a"), "(?i:(?s:.*)(?:a)(?s:.*))");
    assert_eq!(super::print::search("rx", "^(?:a)|(?:b)$"), "(?s:.*)(?:^(?:a)|(?:b)$)(?s:.*)");
    assert_eq!(super::print::search("rx", r"^(?:a\)[)]b)$"), r"a\)[)]b");
}

/// Writes each authored program's `.rerail` beside it (run once per change of an `.rdl`).
#[test]
#[ignore = "writes files: the .rerail translations of the authored .rdl programs"]
fn write_rr_translations() {
    for (name, text, path) in authored() {
        std::fs::write(path.with_extension("rerail"), translate(&text).unwrap_or_else(|e| panic!("{name}: {e}"))).unwrap();
    }
}

/// Which hand-written data-layer rules lower to the engine today, and what stops the others (a report).
/// `RERAIL_RR_REPORT_EXTRA=<dir>[:<dir>…]` reports the `.rerail` files under those directories too (another branch's
/// packs, say), with `#include` still read from this library.
#[test]
#[ignore = "a report: what the hand-written rules need from the lowering"]
fn hand_written_rules_lowering_report() {
    let mut out = Vec::new();
    files(&rule_syntax().join("data-layer"), "rerail", &mut out);
    files(&library(), "rerail", &mut out);
    for dir in std::env::var("RERAIL_RR_REPORT_EXTRA").iter().flat_map(|dirs| dirs.split(':')).filter(|d| !d.is_empty()) {
        files(Path::new(dir), "rerail", &mut out);
    }
    for f in out {
        let result = parse_file_version(&f);
        println!("{}: {}", short(&f), result.map_or_else(|e| e.to_string(), |p| format!("lowers: {} rules", p.rules().count())));
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
    // Each rule's aggregate: written out, the variable it is over, its atom written out, and the atom's relation.
    let agg = |p: &Arc<Version>, rel: &str| -> (String, Option<String>, String, String) {
        let r = rules_of(p, |h| h == rel)[0];
        r.body()
            .find_map(|l| match l.view() {
                Lit::Agg { over, atom, .. } => Some((p.show_lit(r, l), over.map(|o| p.text(r.var(o)).to_owned()), p.show_atom(r, atom), p.rel_name(atom.rel()).to_owned())),
                _ => None,
            })
            .unwrap()
    };
    let (lit, over, atom, _) = agg(&p, "s");
    assert_eq!((over.as_deref(), atom.as_str()), (Some("i"), "arg($c, _, $i)"), "{lit}");
    assert!(lit.starts_with("$n = sum $i : "), "{lit}");
    let (lit, over, atom, _) = agg(&p, "j");
    assert_eq!((lit.as_str(), over.as_deref(), atom.as_str()), ("$x = join $t by ($i) sep \", \" : { arg($c, $t, $i) }", Some("t"), "arg($c, $t, $i)"));
    // Keys (n, i): the helper's columns in order c, i, n, t.
    let (lit, _, atom, rel) = agg(&p, "k");
    assert!(lit.contains(" by ($n, $i) sep \"\" : "), "{lit}");
    let types: Vec<String> = p.columns_of(&rel).unwrap().into_iter().map(|(_, t)| t).collect();
    assert_eq!(types, ["node", "int", "int", "sym"], "{atom}");
    let (lit, over, _, rel) = agg(&p, "g");
    let over = over.unwrap();
    assert!(lit.contains(" = sum $") && over.starts_with("rr_val"), "{lit}");
    // A helper named by its content names its columns by their places (decision 0639 §4).
    let cols = p.columns_of(&rel).unwrap();
    assert_eq!(cols.len(), 4, "c, t, the named `_` and the value: {cols:?}");
    assert!(cols.iter().any(|(_, t)| t == "int"), "{cols:?}");
    let (_, _, _, rel) = agg(&p, "r");
    assert_ne!(rel, "arg", "arg(c, t, t) tests t = t: a helper relation");
    let e = ".decl a(x: number)\na(x) :- a(y), x = sum z sep \",\" : { a(z) }.";
    assert!(matches!(parse(e), Err(Error::Syntax(_))), "{e}");
    let p = parse(".decl b(z: symbol)\n.input b\n.decl a(x: symbol)\na(x) :- x = join z : { b(z) }.\n").unwrap();
    let (lit, ..) = agg(&p, "a");
    assert!(lit.contains(" by () sep \"\" : "), "{lit}");
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
    let cc = rules_of(&p, |h| h == "cc")[0];
    let var = cc.body().find_map(|l| if let Lit::Agg { var, op: crate::code_rule_dsl::logic::program_image::AggRef::Count, .. } = l.view() { Some(var) } else { None }).unwrap_or_else(|| panic!("{}", p.show_rule(cc)));
    assert_eq!(p.show_term(cc, cc.head().args().nth(1).unwrap()), format!("@add(1, ${})", p.text(cc.var(var))), "{}", p.show_rule(cc));
    let two = rules_of(&p, |h| h == "two")[0];
    assert_eq!(two.body().filter(|l| matches!(l.view(), Lit::Agg { .. })).count(), 2, "{}", p.show_rule(two));
}

/// The aggregates print as `.rerail` that reads back as the same program.
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
    let c = parse(&print_version(&b).unwrap()).unwrap_or_else(|e| panic!("{e}\n{}", print_version(&b).unwrap()));
    assert_eq!(difference(&b, &c), None, "{}", print_version(&b).unwrap());
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
    let heads: Vec<String> = rules_of(&p, |h| h == "a").into_iter().map(|r| p.show_atom(r, r.head())).collect();
    assert_eq!(
        heads,
        [
            "a(@concat(@concat(@concat($s, \"-\"), $s), \".\"), @add(@max($n, 0), @mod(@min($n, 9), 4)))",
            "a(@lower($s), @to_number($s))",
        ]
    );
    let back = parse(&print_version(&p).unwrap()).unwrap();
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
    let all = ProgramFields::settings(&*p);
    let settings: Vec<(&str, &str)> = all.iter().map(|(k, t)| (k.as_str(), t.as_str())).collect();
    assert_eq!(settings, [("policy", "\"p\" revision 1"), ("suppress", "V eslint \"@scope/a-b\", \"a-b\" over {x}"), ("suppress", "V oxlint \"a-b\""), ("fix", "V safe drop \"Remove it\""), ("message", "V \"{x}\""), ("message", "W \"w\"")]);
    let rules = |rel: &str| rules_of(&p, |h| h == "violation").into_iter().filter(|r| p.show_term(*r, r.head().args().nth(1).unwrap()) == s(rel)).map(|r| p.show_rule(r)).collect::<Vec<_>>();
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
    let printed = print_version(&p).unwrap();
    let again = parse(&printed).unwrap();
    assert_eq!(difference(&p, &again), None, "{printed}");
    assert_eq!(ProgramFields::settings(&*p), ProgramFields::settings(&*again));
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
    let rules: Vec<Vec<String>> = rules_of(&p, |h| h == "r").into_iter().map(|r| body_of(&p, r)).collect();
    assert_eq!(rules[0], ["rr_has_fn($n, $a)"]);
    assert!(!p.kindsets().contains_key("blk") && p.kindsets().contains_key("fn"));
    // The nearest block: a climb from n that stops at the first member of blk, which is a.
    assert!(rules[1].iter().any(|l| is_helper(l, "up") && l.ends_with("($n, $a)")), "{:?}", rules[1]);
    assert!(rules[1].iter().any(|l| l == "blk($a)"), "{:?}", rules[1]);
    // `next_sibling` is core.rerail's, written out over `child` in the rule (`rr::lower::core`): no relation of the program.
    assert!(shown(&p, |h| h == "next_sibling").is_empty() && !p.input_names().contains(&"next_sibling".to_owned()));
    let again = parse(&print_version(&p).unwrap()).unwrap();
    assert_eq!(difference(&p, &again), None);
}

/// What the axes still refuse says so by name; core.rerail's `within`, `earlier` and `precedes_stop` negate with a node
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
fn many_within_stops_read_their_sets_relation() {
    // Five `within_stop`s of two alternatives each: past 16 combinations the fifth reads `rr_wstop_<S>`, made once
    // for both clauses.
    let clause = |h: &str| format!("{h}(n, a) :- e(n, a), within_stop(n, a, \"x\"), within_stop(n, a, \"y\"), within_stop(n, a, \"z\"), within_stop(n, a, \"u\"), within_stop(n, a, \"v\").\n");
    let text = format!(".decl e(n: node, a: node)\n.input e\n.decl r(n: node, a: node)\n.decl q(n: node, a: node)\n{}{}", clause("r"), clause("q"));
    let p = parse(&text).unwrap();
    for h in ["r", "q"] {
        let rules = shown(&p, |r| r == h);
        assert_eq!(rules.len(), 16, "{rules:#?}");
        assert!(rules.iter().all(|r| r.ends_with("rr_wstop_v($n, $a)")), "{rules:#?}");
    }
    assert_eq!(shown(&p, |r| r == "rr_wstop_v").len(), 2);
    let printed = print_version(&p).and_then(|text| parse(&text)).unwrap();
    assert_eq!(difference(&p, &printed), None);
}

#[test]
fn equal_helpers_of_one_walk_are_one() {
    // Two aggregates that only test `x` each need a context of x: the same rule, so one relation (`name_helpers`).
    let p = parse(concat!(
        ".decl e(x: number)\n.decl f(y: number, w: number)\n.input e, f\n",
        ".decl a(x: number, n: number, m: number)\n",
        "a(x, n, m) :- e(x), n = count : { f(y, _), y > x }, m = count : { f(z, _), z > x }.\n",
    ))
    .unwrap();
    let ctx: BTreeSet<String> = p.rules().map(|r| p.rel_name(r.head().rel()).to_owned()).filter(|h| is_helper(h, "ctx")).collect();
    assert_eq!(ctx.len(), 1, "{:#?}", shown(&p, |_| true));
    let printed = print_version(&p).and_then(|text| parse(&text)).unwrap();
    assert_eq!(difference(&p, &printed), None);
}

#[test]
fn a_nest_of_disjunctions_is_outlined() {
    // Five choices of two: past 16 combinations, the fifth is a helper relation of its alternatives, a rule each.
    let mut text = String::from(".decl e(x: number)\n.input e\n");
    for k in 1..=5 {
        text.push_str(&format!(".decl f{k}(x: number)\n.decl g{k}(x: number)\n.input f{k}, g{k}\n"));
    }
    text.push_str(".decl a(x: number)\na(x) :- e(x), (f1(x) ; g1(x)), (f2(x) ; g2(x)), (f3(x) ; g3(x)), (f4(x) ; g4(x)), (f5(x) ; g5(x)).\n");
    let p = parse(&text).unwrap();
    let rules = shown(&p, |h| h == "a");
    assert_eq!(rules.len(), 16, "{rules:#?}");
    let helper = rules[0].rsplit(", ").next().unwrap().trim_end_matches("($x)").to_owned();
    assert!(is_helper(&helper, "or"), "{rules:#?}");
    assert_eq!(rules[0], format!("a($x) :- e($x), f1($x), f2($x), f3($x), f4($x), {helper}($x)"));
    assert_eq!(rules[15], format!("a($x) :- e($x), g1($x), g2($x), g3($x), g4($x), {helper}($x)"));
    assert!(rules.iter().all(|r| r.ends_with(&format!("{helper}($x)"))), "{rules:#?}");
    assert_eq!(shown(&p, |h| h == helper), [format!("{helper}($x) :- f5($x)"), format!("{helper}($x) :- g5($x)")]);
    let printed = print_version(&p).and_then(|text| parse(&text)).unwrap();
    assert_eq!(difference(&p, &printed), None);
}

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
    // The rule's negated helpers, and each helper's rules as text with its name as `H`.
    let helpers = |rel: &str| -> Vec<(String, Vec<String>)> {
        let [rule] = rules_of(&p, |h| h == rel)[..] else { panic!("{rel}: one rule") };
        rule.body()
            .filter_map(|l| match l.view() {
                Lit::Atom(a) if a.neg() => {
                    let name = p.rel_name(a.rel());
                    Some((p.show_atom(rule, a).replace(name, "H"), shown(&p, |h| h == name).iter().map(|r| r.replace(name, "H")).collect()))
                }
                _ => None,
            })
            .collect()
    };
    assert_eq!(helpers("a"), [("!H($x)".to_owned(), vec!["H($x) :- f($x, $y), g($y)".to_owned()])]);
    assert_eq!(helpers("b"), [("!H($x)".to_owned(), vec!["H($x) :- f($x, _)".to_owned()]), ("!H($x)".to_owned(), vec!["H($x) :- g($x)".to_owned()])]);
    let c = helpers("c");
    assert_eq!(c.len(), 1);
    let [body] = &c[0].1[..] else { panic!("{c:?}") };
    assert!(body.strip_prefix("H($x) :- ").is_some_and(|b| is_helper(b, "ctx")) && body.ends_with("($x), $x > 2, o(\"k\", \"on\")"), "{body}");
    assert_eq!(helpers("d"), [("!H(0)".to_owned(), vec!["H(0) :- o(\"k\", \"on\"), g(_)".to_owned()])]);
    let printed = print_version(&p).and_then(|text| parse(&text)).unwrap();
    assert_eq!(difference(&p, &printed), None);
}

/// Decision 0044 §4: a program is a derivation of the library files it read. Reading it again
/// reuses it; editing a file it read, or creating one where an include looked, reads it again.
#[test]
fn programs_are_kept_by_the_hashes_of_what_they_read() {
    let dir = std::env::temp_dir().join(format!("rr-library-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (main, helper) = (dir.join("main.rerail"), dir.join("zz_helper_0044.rerail"));
    std::fs::write(&helper, ".decl h(x: number)\nh(1).\n").unwrap();
    std::fs::write(&main, "#include \"zz_helper_0044.rerail\"\n.decl r(x: number)\nr(x) :- h(x).\n.output r\n").unwrap();
    let (first, files) = read_file(&main).unwrap();
    let names: Vec<String> = files.iter().map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(names.first().map(String::as_str), Some("core.rerail"), "core.rerail first: {names:?}");
    // In reading order: each file, then what it includes; the helper twice, as the library probe
    // that found nothing and as the file beside main.rerail.
    assert_eq!(names, ["core.rerail", "main.rerail", "zz_helper_0044.rerail", "zz_helper_0044.rerail"]);
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

/// `must, may` (core.rerail 3): two sides, negation reading the other one, and what reads them three-valued too.
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
    let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
    assert_eq!(read("seen"), [["a"], ["b"]]);
    assert_eq!(read("rr_must_seen"), [["a"]]);
    assert_eq!(read("q"), [["b"], ["c"]]);
    assert_eq!(read("rr_must_q"), [["c"]]);
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
    let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
    assert_eq!(read("pairs"), [["13", "21"]]);
    assert_eq!(read("by_set"), [["13", "21"], ["13", "29"], ["21", "29"]]);
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
    let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
    assert_eq!(read("other"), [["38", "0"], ["46", "17"]]);
    assert_eq!(read("same"), [["0"]]);
    assert_eq!(read("empty"), [["0", ""]]);
    assert_eq!(read("given"), [["38"], ["46"]]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file's lines are its language's (decision 0612), one definition for `src_line` and every place: Python's end at
/// `\r\n`, `\r` and `\n` alone (CPython's tokenizer, Ruff), so a `\v`, `\f`, `\x1c`-`\x1e`, `\x85`, U+2028 or U+2029
/// stays inside a line; ECMAScript's at U+2028 and U+2029 too (ESLint, TypeScript); Rust's at `\n` alone (rustc),
/// a lone `\r` inside a line. Long and empty lines are whole, the empty line after a final terminator is one, and a
/// node's `line` is the `src_line` holding it. Each document's lines are its own.
#[test]
fn a_files_lines_are_its_languages() {
    let long = "x".repeat(400);
    let tail = "\u{000b}two\u{000c}\u{001c}four\u{001d}five\u{001e}six\u{0085}seven\u{2028}eight\u{2029}nine";
    let first = format!("{long}{tail}");
    let a = format!("{first}\r\nten\releven\nf(x)\n");
    // Each language's files, its lines (from 1), and the line its call is on.
    type Case<'a> = (&'a str, Vec<(&'a str, &'a str, Vec<&'a str>)>, Vec<(&'a str, u32)>);
    let cases: [Case<'_>; 3] = [
        ("python", vec![("a.py", &a, vec![&first, "ten", "eleven", "f(x)", ""]), ("b.py", "short", vec!["short"]), ("empty.py", "", vec![""])], vec![("a.py", 4)]),
        ("javascript", vec![("c.js", "a\u{2028}b\rc\r\nf(x)\n", vec!["a", "b", "c", "f(x)", ""])], vec![("c.js", 4)]),
        ("rust", vec![("d.rs", "a\rb\r\nfn f() { g(x) }\n", vec!["a\rb", "fn f() { g(x) }", ""])], vec![("d.rs", 2)]),
    ];
    for (language, files, calls) in cases {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension(format!("lines-{language}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (path, source, _) in &files {
            std::fs::write(dir.join(path), source).unwrap();
        }
        let program = format!(r#"
.language {language}
.decl seen(p: symbol, l: number, t: symbol)
seen(p, l, t) :- doc(d, p, _), src_line(d, l, sp), text(sp, t).
.decl call_at(p: symbol, l: number)
call_at(p, l) :- doc(d, p, _), node(d, n, k, _, _), (k = "call" ; k = "call_expression"), line(n, l), src_line(d, l, sp), text(sp, t), contains("(x)", t).
.output seen, call_at
"#);
        let policy = dir.join("lines.rerail");
        std::fs::write(&policy, program).unwrap();
        let program = crate::code_rule_dsl::tools::read_rr(&policy).unwrap();
        let out = dir.join("out");
        crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        let rows = crate::code_rule_dsl::logic::plan::output::shown(&out, "seen").unwrap();
        let mut expected: Vec<Vec<String>> = files.iter().flat_map(|(p, _, lines)| lines.iter().enumerate().map(move |(i, t)| vec![(*p).to_owned(), (i + 1).to_string(), (*t).to_owned()])).collect();
        expected.sort();
        assert_eq!(rows, expected, "{language}");
        let got = crate::code_rule_dsl::logic::plan::output::shown(&out, "call_at").unwrap();
        let want: Vec<Vec<String>> = calls.iter().map(|(p, l)| vec![(*p).to_owned(), l.to_string()]).collect();
        assert_eq!(got, want, "{language}: a call's line is the src_line holding it");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Empty scopes still run global absence checks; they must not become a planning error or an early return.
#[test]
fn empty_corpora_run_global_absence_findings() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("emptyrr");
    let corpus = dir.join("corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    let policy = dir.join("rule.rerail");
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
    let policy = dir.join("rule.rerail");
    std::fs::write(&policy, r#"
.policy "explicit-inputs" revision 1
.language python
.violation Seen(at: span, t: symbol) "{t}"
Seen(@place(p, l), t) :- doc(d, p, _), src_line(d, l, sp), text(sp, t).
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
    let policy = dir.join("rule.rerail");
    std::fs::write(&policy, r#"
.policy "settings-read" revision 1
.language python
.violation Seen(at: span, m: symbol) "{m}"
Seen(@place(p, l), m) :- doc(d, p, _), src_line(d, l, _), env("mark", m).
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

/// A program `run-rr` read is handed to the run, keyed by its digest (decision 0249 §5). With a store, the run's
/// whole-run cutoff asks whether it would read alike: of the program handed, never of its key as a text. Run twice
/// over an unchanged root, the second finds what the first did.
#[test]
fn a_handed_program_run_again_finds_the_same() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("rrhanded");
    let root = dir.join("corpus");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.py"), "def f():\n    pass\n\nf()\n").unwrap();
    let policy = dir.join("rule.rerail");
    std::fs::write(&policy, ".policy \"handed-again\" revision 1\n.language python\n.violation Call(at: node) code \"CALL\" \"a call\"\nCall(n) :- kind(n, \"call\").\n").unwrap();
    crate::code_rule_dsl::logic::plan::corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
    let run = |out: &str| crate::code_rule_dsl::tools::run_rr(&policy, &root, &dir.join(out), &serde_json::Value::Null).map(|r| r["diagnostics"].clone());
    let first = run("first").unwrap();
    assert_eq!(first.as_array().map(Vec::len), Some(1), "{first}");
    let again = run("again");
    crate::code_rule_dsl::logic::plan::corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = None);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(again.unwrap(), first);
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
    let rule = dir.join("rule.rerail");
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
    let rule = dir.join("rule.rerail");
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
    let js = dir.join("js.rerail");
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
    let rule = dir.join("rule.rerail");
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
#include "data-layer/signing.rerail"
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
    assert_eq!(crate::code_rule_dsl::logic::plan::output::shown(&out, "verified").unwrap(), [["original"]]);
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
    let rule = dir.join("digest.rerail");
    std::fs::write(&rule, r#"
.decl rhs(l: symbol, r: node)
rhs(l, r) :- field(a, "left", x), text(x, l), field(a, "right", r), kind(a, "assignment").
.decl same(l: symbol, m: symbol)
same(l, m) :- rhs(l, r), rhs(m, s), l < m, @tree_digest(r) = @tree_digest(s).
.output same
"#).unwrap();
    let program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    assert_eq!(crate::code_rule_dsl::logic::plan::output::shown(&out, "same").unwrap(), [["a", "b"], ["c", "d"], ["e", "f"], ["g", "h"]]);
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
        let rule = dir.join("d.rerail");
        std::fs::write(&rule, r#"
.decl unit(k: symbol, s: symbol)
unit("<module>", s) :- def_digest(n, s), kind(n, "module").
unit(x, s) :- def_digest(n, s), field(n, "name", i), text(i, x).
.output unit
"#).unwrap();
        let program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
        let out = dir.join("out");
        crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        crate::code_rule_dsl::logic::plan::output::shown(&out, "unit").unwrap().into_iter().map(|r| (r[0].clone(), r[1].clone())).collect()
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
    let rows = |source: &str| -> std::collections::BTreeSet<Vec<String>> {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.py"), source).unwrap();
        let rule = dir.join("d.rerail");
        std::fs::write(&rule, ".decl k(n: node, s: symbol)\nk(n, s) :- kind(n, s).\n.decl p(n: node, q: node, i: number)\np(n, q, i) :- child(n, q, i).\n.output k, p\n").unwrap();
        let program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
        let out = dir.join("out");
        crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        ["k", "p"].iter().flat_map(|r| crate::code_rule_dsl::logic::plan::output::shown(&out, r).unwrap().into_iter().map(|row| std::iter::once((*r).to_owned()).chain(row).collect::<Vec<_>>()).collect::<Vec<_>>()).collect()
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

/// `match_all` and `match` with captures (`core.rerail`): over a `source` or `src_line` span each group is a span, with
/// its text and line (`rx_span`); over a symbol, `match_all` gives a row per match with its groups' text.
#[test]
fn match_all_captures_spans_over_source_text_and_symbols_over_symbols() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("matchall");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "x = 1  # SEC1 SEC22\nclass Foo:\n    pass\nclass Bar: pass\n").unwrap();
    let rule = dir.join("m.rerail");
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
    let program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
    assert_eq!(read("sec"), [["1", "SEC1", "1"], ["1", "SEC22", "1"]]);
    assert_eq!(read("cls"), [["Bar", "4"], ["Foo", "2"]]);
    assert_eq!(read("whole"), [["1", "1"]]);
    assert_eq!(read("kv"), [["a", "1"], ["b", "22"]]);
    assert_eq!(read("hasb"), [["a=1 b=22 c=x"]]);
    assert_eq!(read("nob"), [["none here"]]);
    assert_eq!(read("via"), [["Bar"], ["Foo"]]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A text test of a capture in the body that captured it tests the capture's text: `match_all("(?<w>…)", t),
/// match("…", w)` over a string's content (RUF001's words). It once tested the span's id as a text, and never held.
#[test]
fn a_text_test_of_a_capture_in_its_own_body_tests_the_captured_text() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("capturetest");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "a = \"x\u{211d}y plain\"\nb = \"2\u{211d}6789\"\n").unwrap();
    let rule = dir.join("m.rerail");
    std::fs::write(&rule, r#"
.decl piece(t: loc)
piece(p) :- kind(p, "string_content").
.decl wide(w: symbol)
wide(x) :- piece(t), match_all("(?<w>[\\p{Alphabetic}\\p{N}]+)", t), match("[\\s\\S]*[^\\x00-\\x7f][\\s\\S]*", w), text(w, x).
.decl odd(w: symbol)
odd(x) :- piece(t), match_all("(?<w>[a-z0-9ℝ]+)", t), !match(".*ℝ.*", w), text(w, x).
.output wide, odd
"#).unwrap();
    let program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
    assert_eq!(read("wide"), [["2ℝ6789"], ["xℝy"]]);
    assert_eq!(read("odd"), [["plain"]]);
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
    let rule = dir.join("m.rerail");
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
    let program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
    // Two strings with one text: each its own spans, at its own line and column.
    assert_eq!(read("param"), [["id", "1", "9"], ["id", "2", "9"], ["name", "1", "14"], ["name", "2", "14"]]);
    assert_eq!(read("digit"), [["1"], ["2"], ["3"]]);
    assert_eq!(read("ndig"), [["AB12", "2"], ["CD3", "1"]]);
}

/// `match_all` over a node's own text, and over an earlier match's captures (decision 0081): each group is a span
/// placed in the file, with its text and line. Strings and comments in one relation give both; the same text in two
/// strings is matched once and placed at each.
#[test]
fn match_all_captures_spans_over_a_nodes_text_and_over_captures() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("matchnode");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "x = 'ab1 cd22'\ny = 'ab1 cd22'  # ef3\n").unwrap();
    let rule = dir.join("m.rerail");
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
    let program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
    assert_eq!(read("digits"), [["1", "1", "7"], ["1", "2", "7"], ["22", "1", "11"], ["22", "2", "11"], ["3", "2", "20"]]);
    assert_eq!(read("per"), [["ab1", "1"], ["cd22", "2"], ["ef3", "1"]]);
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
    let rule = dir.join("m.rerail");
    std::fs::write(&rule, r#"
.decl comma(l: number, c: number)
comma(l, c) :- kind(n, "argument_list"), match_all(",", n, m), line(m, l), col(m, c).
.decl in_node(t: symbol, k: number)
in_node(t, k) :- kind(n, "argument_list"), text(n, t), k = count : { match_all(",", n, _) }.
.decl in_text(t: symbol, k: number)
in_text(t, k) :- kind(n, "argument_list"), text(n, t), k = count : { match_all(",", t, _, _, _) }.
.output comma, in_node, in_text
"#).unwrap();
    let program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
    assert_eq!(read("comma"), [["1", "3"], ["1", "6"]]);
    assert_eq!(read("in_node"), [["(a, b, c)", "2"], ["(d)", "0"]]);
    assert_eq!(read("in_text"), [["(a, b, c)", "2"], ["(d)", "0"]]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `data-layer/noqa-sec-ratchet.rerail`, whose `match_all` over each line's span is what lowering refused (O80): every
/// SEC code on a suppression line counts, inside the comment or not.
#[test]
fn noqa_sec_ratchet_counts_every_sec_code_on_suppression_lines() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("noqa");
    std::fs::create_dir_all(dir.join("impl")).unwrap();
    std::fs::write(dir.join("impl/a.py"), "x = \"SEC1\"  # noqa: SEC2\ny = 1  # noqa: E501, SEC3 SEC44\nz = \"SEC9\"\n").unwrap();
    let rule = crate::code_rule_dsl::logic::rr::library().join("../data-layer/noqa-sec-ratchet.rerail");
    let program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    assert_eq!(crate::code_rule_dsl::logic::plan::output::shown(&out, "annotations").unwrap(), [["noqa_sec_annotations", "4"]]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Error directives report rejected syntax while raw-source consumers keep their document views.
#[test]
fn syntax_error_directives_preserve_raw_source_relations() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("withheld");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("good.py"), "x = 1\n").unwrap();
    std::fs::write(dir.join("bad.py"), "y = 2\ndef (:\n").unwrap();
    let rule = dir.join("w.rerail");
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
    let program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
    assert_eq!(read("violation"), [["t:withheld", "ASSIGN", "ASSIGN", "good.py", "1", "0"], ["t:withheld", "syntax_error", "BAD000", "bad.py", "1", "0"]]);
    assert_eq!(read("lines").len(), 2, "raw-text policies retain both documents");
    assert_eq!(read("err").len(), 1);
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
    let rule = dir.join("c.rerail");
    std::fs::write(&rule, r#"
.decl defined(x: symbol)
defined(x) :- kind(n, "function_definition"), field(n, "name", m), text(m, x).
.decl called(x: symbol, p: symbol)
called(x, p) :- kind(c, "call"), field(c, "function", f), text(f, x), defined(x), docof(c, d), doc(d, p, _).
.violation UNKNOWN(at: node, x: symbol) "call of {x}"
UNKNOWN(c, x) :- kind(c, "call"), field(c, "function", f), text(f, x), !defined(x).
.output called
"#).unwrap();
    let program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
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
        ["called", "violation"].map(|rel| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap())
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
    let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
    assert_eq!(read("hit"), [["10", "30", "1"], ["20", "30", "5"], ["40", "30", "2"]]);
    assert_eq!(read("hm"), [["10", "30", "1"], ["40", "30", "2"]]);
    assert_eq!(read("none"), [["50"]]);
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
    let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
    assert_eq!(read("named"), [["1", "1"], ["2", "2"], ["3", "2"], ["4", "4"]]);
    // 1 and 4 have the same digest and the same closure; 2 and 3 share their component.
    assert_eq!(read("same"), [["1", "4"], ["2", "3"]]);
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
    let printed = print_version(&parsed).unwrap();
    let roundtrip = parse(&printed).unwrap();
    assert_eq!(difference(&roundtrip, &parsed), None, "{printed}");
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
    // The empty line after the final terminator is a line too (decision 0612).
    assert_eq!(lines, [(1, source.split('\n').next().unwrap()), (2, "pass"), (3, "")]);
    // Decision 0108: a kind the rule reads by `kind` has its text at any length (the module's here, named by
    // `Subtree`); the source span and the lines are exact either way.
    assert_eq!(rows.iter().filter(|row| row["code"] == "TREE").count(), 1, "the module, a kind the rule reads, has its whole text");
}

/// Decision 0055's guests: a run of `rule` over `files`, each output's sorted rows, each value as shown.
fn guests_run(name: &str, files: &[(&str, &str)], rule: &str, outputs: &[&str]) -> Vec<Vec<Vec<String>>> {
    guests_run_with(name, files, rule, |o| outputs.iter().map(|rel| o.get(rel).map(|r| r.shown()).unwrap_or_default()).collect())
}

/// Each output's rows as shown, a row's cells joined by tabs: for comparing with expected rows written as one string.
fn joined(outputs: Vec<Vec<Vec<String>>>) -> Vec<Vec<String>> {
    outputs.into_iter().map(|rows| rows.into_iter().map(|r| r.join("\t")).collect()).collect()
}

/// One row's values as shown, for comparing with `guests_run`'s.
fn shown_row(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_owned()).collect()
}

/// A run of `rule` over `files`, its outputs read by `read`.
fn guests_run_with<T>(name: &str, files: &[(&str, &str)], rule: &str, read: impl FnOnce(&crate::code_rule_dsl::logic::plan::output::Outputs) -> T) -> T {
    // The languages, as the library's load installs them where a load hook runs (a runner with none installs them
    // at the first store it opens, after the program is lowered).
    crate::install_store_functions();
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension(name);
    std::fs::create_dir_all(&dir).unwrap();
    for (path, text) in files {
        std::fs::create_dir_all(dir.join(path).parent().unwrap()).unwrap();
        std::fs::write(dir.join(path), text).unwrap();
    }
    let path = dir.join("g.rerail");
    std::fs::write(&path, rule).unwrap();
    let program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&path).unwrap()).unwrap();
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let rows = read(&crate::code_rule_dsl::logic::plan::output::Outputs::open(&out).unwrap());
    let _ = std::fs::remove_dir_all(&dir);
    rows
}

/// A string annotation is a tape value (decision 0431): its names are placed in the file through the string
/// (`guest_place`), through the escapes and the parts of a concatenation (the normalized text's map); a string inside
/// a parsed one is parsed in the next round, for a rule that reaches it through the annotation (`ann`), and is placed
/// in the annotation's text, which is no file's: an annotation is not shown to rules that do not name it, so
/// `ident`'s `kind(s, "string")` does not see it. Bytes give no guest.
#[test]
fn string_annotations_are_guests_at_their_host_lines() {
    let py = "x: \"List[int]\" = []\n\ndef f(y: 'Dict[str, Foo]') -> \"Bar\": ...\nz: \"A\\x42\" = 1\nw: (\"Li\" \"st\") = 2\nq: \"Opt['Baz']\" = 3\nb: b\"Nope\" = 4\n";
    let rule = r#"
.decl ident(x: symbol, l: number, c: number)
ident(x, l, c) :- kind(s, "string"), parsed(s, "annotation", _), guest_of(i, s), kind(i, "identifier"), text(i, x), guest_place(i, s, l, c).
.decl ann(n: node)
ann(r) :- kind(s, "string"), parsed(s, "annotation", r).
ann(c) :- ann(p), child(p, c, _).
.decl nested(x: symbol)
nested(x) :- ann(s), kind(s, "string"), parsed(s, "annotation", _), guest_of(i, s), kind(i, "identifier"), text(i, x).
.decl whole(x: symbol, l: number, c: number)
whole(x, l, c) :- kind(s, "concatenated_string"), parsed(s, "annotation", _), guest_of(i, s), kind(i, "identifier"), text(i, x), guest_place(i, s, l, c).
.decl roots(k: symbol)
roots(k) :- kind(s, "string"), parsed(s, "annotation", r), kind(r, k).
.output ident, nested, whole, roots
"#;
    let [names, nested, whole, roots] = <[Vec<Vec<String>>; 4]>::try_from(guests_run("annotations", &[("a.py", py)], rule, &["ident", "nested", "whole", "roots"])).unwrap();
    assert!(names.contains(&shown_row(&["List", "1", "4"])), "{names:?}");
    assert!(names.contains(&shown_row(&["int", "1", "9"])), "{names:?}");
    assert!(names.contains(&shown_row(&["Foo", "3", "20"])), "{names:?}");
    assert!(names.contains(&shown_row(&["Bar", "3", "31"])), "{names:?}");
    // "A\x42" is AB: its normalized text, mapped (decision 0058 §3), so AB starts at the A.
    assert!(names.contains(&shown_row(&["AB", "4", "4"])), "{names:?}");
    // ("Li" "st"): each part is a string of its own, and the concatenation is List, from its first part's text.
    assert!(names.contains(&shown_row(&["Li", "5", "5"])), "{names:?}");
    assert_eq!(whole, [["List", "5", "5"]]);
    // Each root is a `type` (decision 0466 R5): what follows `x:` in a file.
    assert_eq!(roots, [["type"]]);
    // 'Baz' inside "Opt['Baz']": round 2, through the annotation; `ident` does not see inside it.
    assert_eq!(nested, [["Baz"]]);
    assert!(!names.iter().any(|n| n[0].starts_with("Baz")), "{names:?}");
    assert!(!names.iter().any(|n| n[0].starts_with("Nope")), "{names:?}");
}

/// Decision 0431 §7 (it replaces Q30): a string a rule parses is a tape value, stored with its text behind the run,
/// so the next run finds it by the text and parses nothing (R6). The host's source is stored, and so is the
/// annotation's text, as it is (no wrapper). Decision 0466 R6: its tape is stored under the fragment's own tag, and
/// no tape of the text under Python's.
#[test]
fn a_run_stores_its_tape_values_by_their_text() {
    use sha2::Digest as _;
    let py = "x: \"List[int]\" = []\n";
    let rule = r#"
.decl ident(x: symbol)
ident(x) :- kind(s, "string"), parsed(s, "annotation", _), guest_of(i, s), kind(i, "identifier"), text(i, x).
.output ident
"#;
    let [names] = <[Vec<Vec<String>>; 1]>::try_from(guests_run("tape-value-sources", &[("a.py", py)], rule, &["ident"])).unwrap();
    assert!(names.contains(&shown_row(&["List"])), "the tape value was made: {names:?}");
    let store = crate::content_store::ContentStore::open(&crate::code_rule_dsl::logic::plan::corpus::tape_cache_path().unwrap()).unwrap();
    let key = |t: &str| crate::content_store::HashKey::from_sha256_digest(sha2::Sha256::digest(t.as_bytes()).into());
    assert!(store.source_by_hash(key(py)).unwrap().is_some(), "the host's source is stored");
    let text = store.source_by_hash(key("List[int]")).unwrap().expect("the tape value's text is stored");
    let (gv, fv) = (crate::code_rule_dsl::build_ids::grammar_version(), i64::from(crate::code_rule_dsl::tape::formats::stored_version()));
    let tag = |t: u8| i64::from(t);
    let fragment = crate::code_rule_dsl::language::fragment_tag("annotation").unwrap();
    let python = crate::code_rule_dsl::language::tape_tag(crate::code_rule_dsl::language::GrammarDialect::Python).unwrap();
    assert!(store.tape_frames_for(&[text], tag(fragment), gv, fv).unwrap()[0].is_some(), "under the fragment's tag");
    assert!(store.tape_frames_for(&[text], tag(python), gv, fv).unwrap()[0].is_none(), "not under Python's");
}

/// Decision 0431 R1: there are no guest documents. A program the grammar holds (an HTML file's scripts) is part of its
/// file: its nodes are numbered in the file's id space, so `docof` of a script's root is the file's, as the `raw_text`
/// it stands at is, and its lines are the file's.
#[test]
fn a_program_a_file_holds_is_the_files() {
    let html = "<!doctype html>\n<script>\nlet a = f(1);\n</script>\n<script type=\"module\">g(2)</script>\n";
    let rule = r#"
.language html
#include "html/embeds.rerail"
.decl same(d: number, e: number)
same(d, e) :- kind(r, "raw_text"), parsed(r, "javascript", p), docof(r, d), docof(p, e).
.decl calls(x: symbol, l: number)
calls(x, l) :- kind(r, "raw_text"), parsed(r, "javascript", p), within(c, p), kind(c, "call_expression"), text(c, x), line(c, l).
.output same, calls
"#;
    let [same, calls] = <[Vec<Vec<String>>; 2]>::try_from(guests_run("program-in-file", &[("a.html", html)], rule, &["same", "calls"])).unwrap();
    assert_eq!(same.len(), 1, "both scripts are a.html's: {same:?}");
    assert_eq!(same[0][0], same[0][1], "a script's root is its file's: {same:?}");
    assert_eq!(calls, [["f(1)", "3"], ["g(2)", "5"]]);
}

/// Decision 0431 §4: a file's programs are granules of the file, so an edit inside one script renumbers no node of
/// the other script and none of the file's own.
#[test]
fn an_edit_in_one_script_renumbers_no_other_program() {
    let html = |body: &str| format!("<!doctype html>\n<p>hi</p>\n<script>\n{body}\n</script>\n<script>g(2); h(3)</script>\n");
    let rule = r#"
.language html
#include "html/embeds.rerail"
.decl ids(x: symbol, n: node, d: number)
ids(x, c, d) :- kind(r, "raw_text"), parsed(r, "javascript", p), within(c, p), kind(c, "call_expression"), text(c, x), docof(c, d).
ids(x, e, d) :- kind(e, "element"), text(e, x), docof(e, d).
.output ids
"#;
    let before = guests_run("one-script-a", &[("a.html", &html("let a = f(1);"))], rule, &["ids"]).remove(0);
    let after = guests_run("one-script-b", &[("a.html", &html("let a = f(1) + k(4);"))], rule, &["ids"]).remove(0);
    let kept = |rows: &[Vec<String>]| rows.iter().filter(|r| ["g(2)", "h(3)", "<p>hi</p>"].contains(&r[0].as_str())).cloned().collect::<Vec<_>>();
    assert_eq!(kept(&before).len(), 3, "{before:?}");
    assert_eq!(kept(&before), kept(&after), "the second script's and the file's ids stay");
    assert!(before.iter().all(|r| r[2] == before[0][2]), "every node is a.html's: {before:?}");
}

/// Decision 0431 R2: `guest_of(n, s)` names the string node a guest node was parsed from (it replaces
/// `docof(n, d), guest(d, _, s, _)`), for every node of the guest and for no node of the file's own tree.
#[test]
fn guest_of_names_the_string_a_node_was_parsed_from() {
    let py = "x: \"List[int]\" = []\nList = 1\n";
    let rule = r#"
.decl from(x: symbol, l: number, c: number)
from(x, l, c) :- kind(s, "string"), parsed(s, "annotation", _), guest_of(i, s), kind(i, "identifier"), text(i, x), line(s, l), col(s, c).
.decl own(x: symbol)
own(x) :- kind(i, "identifier"), text(i, x), !guest_of(i, _).
.output from, own
"#;
    let [from, own] = <[Vec<Vec<String>>; 2]>::try_from(guests_run("guest-of", &[("a.py", py)], rule, &["from", "own"])).unwrap();
    assert_eq!(from, [["List", "1", "3"], ["int", "1", "3"]], "each name of the annotation, at its string");
    assert_eq!(own, [["List"], ["x"]], "the file's own identifiers are no guest's");
}

/// Decision 0431 R3: a string a rule parses is a tape value. The same annotation in two files is one tape and one
/// set of nodes: both strings' `parsed` give the same root. A tape value's node is no file's (its positions are its
/// own text's); a rule places it through its string.
#[test]
fn one_tape_at_two_places_is_one_set_of_nodes() {
    let a = "x: \"List[int]\" = []\n";
    let b = "\n\n\ny: \"List[int]\" = []\n";
    let rule = r#"
.decl tapes(t: tape)
tapes(t) :- kind(s, "string"), sub_tape(s, "annotation", t).
.decl roots(r: node)
roots(r) :- kind(s, "string"), parsed(s, "annotation", r).
.decl files(f: symbol)
files(f) :- kind(s, "string"), parsed(s, "annotation", r), docof(r, d), doc(d, f, _).
.decl at(f: symbol, l: number)
at(f, l) :- kind(s, "string"), parsed(s, "annotation", _), line(s, l), docof(s, d), doc(d, f, _).
.output tapes, roots, files, at
"#;
    let [tapes, roots, files, at] = <[Vec<Vec<String>>; 4]>::try_from(guests_run("guest-two-places", &[("a.py", a), ("b.py", b)], rule, &["tapes", "roots", "files", "at"])).unwrap();
    assert_eq!(tapes.len(), 1, "one tape for the one text: {tapes:?}");
    assert_eq!(roots.len(), 1, "one root, the tape's, for both places: {roots:?}");
    assert!(files.is_empty(), "a tape value's node is no file's: {files:?}");
    assert_eq!(at, [["a.py", "1"], ["b.py", "4"]], "each place at its string");
}

/// Decision 0431 R9: a finding at a tape value's node stands at each string that holds it (`guest_place`, §3: "a
/// finding at s by n, placed through s"); at a node of a value no string holds (`@parse` of a rule's text), it has no
/// place, and is refused, not dropped.
#[test]
fn a_finding_at_a_tape_values_node_is_placed_through_its_string() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("tape-finding-placed");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "import os\n\nx: \"List[int]\" = []\n").unwrap();
    let program = r#"
.policy "t:tape-finding-placed" revision 1
.language python
.violation Inner(at: node) "a name inside an annotation"
Inner(i) :- string_annotation(s), parsed(s, "annotation", _), guest_of(i, s), kind(i, "identifier"), text(i, "List").
"#;
    let rule = dir.with_extension("tape-finding-placed.rerail");
    std::fs::write(&rule, program).unwrap();
    let flat = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    std::fs::remove_file(&rule).unwrap();
    let ran = crate::code_rule_dsl::logic::plan::corpus::run(&flat, &dir, None, &dir.join("out"), "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18);
    let found = ran.as_ref().ok().and_then(|_| crate::code_rule_dsl::logic::plan::output::shown(&dir.join("out"), "violation").ok());
    let _ = std::fs::remove_dir_all(&dir);
    let found = found.unwrap_or_else(|| panic!("the run places the finding: {:?}", ran.err()));
    let at: Vec<(&str, &str)> = found.iter().map(|r| (r[3].as_str(), r[4].as_str())).collect();
    assert_eq!(at, [("a.py", "3")], "at the string's line in its file: {found:?}");
}

/// Decision 0431 R9: a tape value's node's place in a file (`guest_place` at a string, its document's path) is one
/// relation a program's findings read, not a join each finding writes again: 346 diagnostics on corpus0 each made the
/// same rows, every guest's placement.
#[test]
fn findings_placed_through_a_string_read_one_placement() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("tape-placed-once");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let program = r#"
.policy "t:tape-placed-once" revision 1
.language python
.violation Inner(at: node) "a name inside an annotation"
.violation Other(at: node) "an integer inside an annotation"
Inner(i) :- string_annotation(s), parsed(s, "annotation", _), guest_of(i, s), kind(i, "identifier"), text(i, "List").
Other(i) :- string_annotation(s), parsed(s, "annotation", _), guest_of(i, s), kind(i, "integer").
"#;
    let rule = dir.join("placed-once.rerail");
    std::fs::write(&rule, program).unwrap();
    let flat = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let rules: Vec<&str> = flat.lines().filter(|l| l.contains(":-")).collect();
    let joins: Vec<&str> = rules.iter().copied().filter(|l| l.contains("guest_place(") && l.contains("doc(")).collect();
    let placed: Vec<&&str> = joins.iter().filter(|l| l.starts_with(super::lower::findings::PLACED_LINE)).collect();
    assert_eq!(placed.len(), 1, "one rule places a value's node in its file: {joins:#?}");
    assert!(joins.iter().all(|l| l.starts_with(super::lower::findings::PLACED_AT) || l.starts_with(super::lower::findings::PLACED_LINE)), "no finding joins the placement again: {joins:#?}");
    let readers = rules.iter().filter(|l| l.contains(&format!("{}(", super::lower::findings::PLACED_LINE)) && !l.starts_with(super::lower::findings::PLACED_LINE)).count();
    assert!(readers >= 4, "each finding's diagnostic and violation read it: {rules:#?}");
}

/// Decision 0431 R9: a program refuses its unplaced findings in one rule, over its findings' nodes, not in a rule each:
/// each finding's own rule, negated reads and all, was a negation split, a demand and a join of its own (three strata
/// a finding, 470 of a pack's on corpus0). A finding writes its clause and node alone, a positive projection.
#[test]
fn a_program_refuses_its_unplaced_findings_in_one_rule() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("unplaced-once");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let guests = r#"
.policy "t:unplaced-once" revision 1
.language python
.violation Inner(at: node) "a name inside an annotation"
.violation Other(at: node) "an integer inside an annotation"
Inner(i) :- string_annotation(s), parsed(s, "annotation", _), guest_of(i, s), kind(i, "identifier"), text(i, "List").
Other(i) :- string_annotation(s), parsed(s, "annotation", _), guest_of(i, s), kind(i, "integer").
"#;
    let plain = r#"
.policy "t:unplaced-once-plain" revision 1
.language python
.violation Name(at: node) "a name"
.violation Number(at: node) "an integer"
Name(i) :- kind(i, "identifier").
Number(i) :- kind(i, "integer").
"#;
    for (name, program, found) in [("guests", guests, super::lower::findings::FOUND_THROUGH), ("plain", plain, super::lower::findings::FOUND_NODE)] {
        let rule = dir.join(format!("{name}.rerail"));
        std::fs::write(&rule, program).unwrap();
        let flat = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
        let rules: Vec<&str> = flat.lines().filter(|l| l.contains(":-")).collect();
        let unplaced: Vec<&&str> = rules.iter().filter(|l| l.starts_with(super::lower::findings::UNPLACED)).collect();
        assert_eq!(unplaced.len(), 1, "{name}: one rule refuses the unplaced findings: {unplaced:#?}");
        assert!(unplaced[0].contains(&format!("{found}(")), "{name}: it reads the findings' nodes: {unplaced:#?}");
        let projections: Vec<&&str> = rules.iter().filter(|l| l.starts_with(&format!("{found}("))).collect();
        assert_eq!(projections.len(), 2, "{name}: a projection a finding: {projections:#?}");
        assert!(projections.iter().all(|l| !l.contains('!')), "{name}: each positive: {projections:#?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Decision 0431 R9: a finding at a node of a string inside another tape value (`'int'` in `"List['int']"`) has no
/// place: that string stands in the outer value's text, which is no file's. It is refused, naming its clause, not
/// dropped.
#[test]
fn a_finding_at_a_tape_values_node_is_refused() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("tape-finding");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "x: \"List['int']\" = []\n").unwrap();
    let program = r#"
.policy "t:tape-finding" revision 1
.language python
.violation Inner(at: node) "a name inside a string inside an annotation"
Inner(i) :- string_annotation(s), parsed(s, "annotation", _), guest_of(i, s), kind(i, "identifier"), text(i, "int").
"#;
    let rule = dir.with_extension("tape-finding.rerail");
    std::fs::write(&rule, program).unwrap();
    let flat = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    std::fs::remove_file(&rule).unwrap();
    let ran = crate::code_rule_dsl::logic::plan::corpus::run(&flat, &dir, None, &dir.join("out"), "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18);
    let _ = std::fs::remove_dir_all(&dir);
    let err = ran.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains("Inner") && err.contains("tape value"), "the finding is refused, naming its clause: {err:?}");
}

/// Decision 0431 R4: the rules decide what is parsed. A plain string holding the same text as an annotation is not
/// parsed: no node of it is a guest's.
#[test]
fn a_string_no_rule_asks_for_is_not_parsed() {
    let py = "x: \"List[int]\" = []\ny = \"Dict[str, int]\"\n";
    let rule = r#"
.decl names(x: symbol)
names(x) :- string_annotation(s), parsed(s, "annotation", _), kind(i, "identifier"), guest_of(i, _), text(i, x).
.output names
"#;
    let [names] = <[Vec<Vec<String>>; 1]>::try_from(guests_run("guest-asked-only", &[("a.py", py)], rule, &["names"])).unwrap();
    assert_eq!(names, [["List"], ["int"]], "only the annotation is parsed: {names:?}");
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
roots(l) :- kind(s, "string"), parsed(s, "annotation", _), line(s, l).
.decl kept(l: number)
kept(l) :- kind(s, "string"), sub_tape(s, "annotation", _), line(s, l).
.output roots, kept
"#
        )
    };
    let [v, roots, kept] = <[Vec<Vec<String>>; 3]>::try_from(guests_run("gskip", &[("a.py", py)], &rule(".on guest_error annotation report GBAD"), &["violation", "roots", "kept"])).unwrap();
    assert_eq!(v, [["t:guest", "guest_error", "GBAD", "a.py", "2", "0"]]);
    assert_eq!(roots, [["1"]], "the broken guest has no tree");
    assert_eq!(kept, [["1"], ["2"]], "both are guests");
    let [v, roots] = <[Vec<Vec<String>>; 2]>::try_from(guests_run("gpartial", &[("a.py", py)], &rule(".on guest_error partial"), &["violation", "roots"])).unwrap();
    assert!(v.is_empty(), "{v:?}");
    assert_eq!(roots, [["1"], ["2"]], "the broken guest keeps its partial tree");
}

/// A string a rule parses is reached through `parsed` (decision 0431 §2; the libraries' `embed` went with §4).
/// `sub_tape` gives its tape: equal texts share one. A tape value is never code to a rule that does not name it
/// (`top`), `.guests off` or not; the rule that parses it reaches it either way (decision 0058 §1).
#[test]
fn parsed_strings_are_reached_through_parsed() {
    let py = "a = \"import os\"\nb = \"import os\"\nc = 1\n";
    let rule = |guests: &str| {
        format!(
            r#"
{guests}
.decl imp(x: symbol, l: number)
imp(x, l) :- kind(s, "string"), parsed(s, "python", r), within(i, r), kind(i, "import_statement"), text(i, x), line(s, l).
.decl tapes(t: tape)
tapes(t) :- kind(s, "string"), sub_tape(s, "python", t).
.decl top(x: symbol, l: number)
top(x, l) :- kind(i, "import_statement"), text(i, x), line(i, l).
.output imp, tapes, top
"#
        )
    };
    let parsed = [["import os", "1"], ["import os", "2"]];
    let [imp, tapes, top] = <[Vec<Vec<String>>; 3]>::try_from(guests_run("embed", &[("a.py", py)], &rule(""), &["imp", "tapes", "top"])).unwrap();
    assert_eq!(imp, parsed);
    assert_eq!(tapes.len(), 1, "{tapes:?}");
    assert!(top.is_empty(), "a tape value is no rule's code that does not name it: {top:?}");
}

/// Decision 0523 R1: `.guests` is retired, and `.scope embedded …` with it (its other name). A program saying either
/// is refused, and the message names `.embedded`, the setting that says what a run reads of other languages' files.
#[test]
fn the_guests_setting_is_refused() {
    let body = ".decl top(x: symbol)\ntop(x) :- kind(i, \"import_statement\"), text(i, x).\n.output top\n";
    for setting in [".guests off", ".guests on", ".scope embedded false", ".scope embedded true"] {
        let err = crate::code_rule_dsl::logic::rr::parse_version(&format!("{setting}\n{body}")).map(|_| ()).unwrap_err().to_string();
        assert!(err.contains(".embedded") && err.contains("0523"), "{setting}: {err}");
    }
    // R2: a program states no `guest_policy("library", …)`, and shows files and the run language's code guests.
    let lowered = crate::code_rule_dsl::logic::rr::parse_version(body).unwrap();
    let library = lowered.fact_rows(crate::code_rule_dsl::facts::vtab::syntax::GUEST_POLICY).into_iter().any(|r| matches!(r.first(), Some(crate::code_rule_dsl::logic::front::Value::Str(k)) if k == "library"));
    assert!(!library, "no library policy");
}

/// Decision 0523 R3, R4: no library rule reads `guest_policy("library", …)`, and no pack rule says `.guests`.
#[test]
fn no_library_or_pack_rule_reads_the_guests_setting() {
    let root = rerail_store::source_tree::native().join("../docs/rule-syntax");
    let (mut found, mut files): (Vec<String>, usize) = (Vec::new(), 0);
    let mut todo = vec![root.join("lib"), root.join("packs")];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                todo.push(path);
            } else if path.extension().is_some_and(|e| e == "rerail") {
                let text = std::fs::read_to_string(&path).unwrap();
                files += 1;
                for (i, line) in text.lines().enumerate() {
                    let code = line.split("//").next().unwrap_or("");
                    if code.contains("guest_policy(\"library\"") || code.trim_start().starts_with(".guests") {
                        found.push(format!("{}:{}", path.strip_prefix(&root).unwrap().display(), i + 1));
                    }
                }
            }
        }
    }
    assert!(files > 500, "the libraries' and packs' rule files are read: {files}");
    assert!(found.is_empty(), "{found:?}");
}

/// SQL in a Python string (decision 0058 §1–2): a rule's `parsed` makes an SQL tape value (tag 10). A rule that
/// reaches it through `parsed` names SQL kinds, and they print as SQL's, placed at the file's lines through the string
/// (`guest_place`); a rule that does not name a guest sees only the Python file, even for the qualified `"sql:…"` kind.
#[test]
fn sql_in_python_is_an_sql_tape_hidden_from_unscoped_rules() {
    let py = "def f(db):\n    db.execute(\"SELECT name FROM users WHERE id = 1\")\n    db.execute(\"SELECT \\\"id\\\" FROM orders\")\n";
    let rule = r#"
.decl sqlstr(s: node)
sqlstr(s) :- kind(c, "call"), field(c, "arguments", a), child(a, s, _), kind(s, "string").
.decl sqlkinds(k: symbol)
sqlkinds(k) :- sqlstr(s), parsed(s, "sql", r), (n = r ; within(n, r)), node(_, n, k, _, _).
.decl docs(g: number)
docs(g) :- sqlstr(s), parsed(s, "sql", r), node(g, r, _, _, _).
.decl objects(x: symbol, l: number, c: number)
objects(x, l, c) :- sqlstr(s), parsed(s, "sql", r), within(n, r), kind(n, "object_reference"), text(n, x), guest_place(n, s, l, c).
.decl unscoped(k: symbol)
unscoped(k) :- node(_, _, k, _, _).
.decl qualified(x: symbol)
qualified(x) :- kind(n, "sql:object_reference"), text(n, x).
.decl counted(c: number)
counted(c) :- c = count : { kind(_, "sql:object_reference") }.
.output sqlkinds, docs, objects, unscoped, qualified, counted
"#;
    let names = ["sqlkinds", "objects", "unscoped", "qualified", "counted"];
    let (shown, docs) = guests_run_with("sqlguest", &[("a.py", py)], rule, |o| {
        let shown: Vec<Vec<Vec<String>>> = names.iter().map(|rel| o.get(rel).map(|r| r.shown()).unwrap_or_default()).collect();
        (shown, o.rows("docs").iter().map(|r| r.get(0).int().expect("a document number")).collect::<Vec<i64>>())
    });
    let [sqlkinds, objects, unscoped, qualified, counted] = <[Vec<Vec<String>>; 5]>::try_from(shown).unwrap();
    assert!(sqlkinds.contains(&shown_row(&["object_reference"])), "{sqlkinds:?}");
    assert!(!sqlkinds.contains(&shown_row(&["call"])), "SQL nodes are SQL's kinds: {sqlkinds:?}");
    assert!(docs.len() == 2 && docs.iter().all(|&g| g >= i64::from(crate::code_rule_dsl::facts::vtab::syntax::HIDDEN_KEYS)), "an SQL tape value in a Python run is hidden: {docs:?}");
    assert!(objects.contains(&shown_row(&["users", "2", "33"])), "{objects:?}");
    // `"SELECT \"id\" FROM orders"` parses as its normalized text, and `orders` keeps its column past the escapes.
    assert!(objects.contains(&shown_row(&["orders", "3", "35"])), "{objects:?}");
    assert!(unscoped.contains(&shown_row(&["call"])), "{unscoped:?}");
    assert!(!unscoped.contains(&shown_row(&["object_reference"])), "SQL nodes are hidden: {unscoped:?}");
    assert!(qualified.is_empty(), "{qualified:?}");
    // An aggregate's atom is bounded too.
    assert!(counted.iter().all(|c| c == &["0"]), "{counted:?}");
}

/// 0597 rung 5.3: an f-string in a language's grammar is its template, a text root whose holes are a marker
/// (`syntax::HOLE`): `users` stands at its own column, the hole stands at `{uid}`, and its host node is the
/// interpolation. Python's `annotation` reads a str's value, so an f-string parses as none.
#[test]
fn an_fstrings_template_is_a_guest_whose_holes_stand_at_their_fields() {
    let py = "def f(db, uid):\n    db.execute(f\"SELECT name FROM users WHERE id = {uid}\")\n    db.execute(f\"SELECT \\\"id\\\" FROM orders WHERE a = {uid!r} AND b = {uid:>3}\")\n";
    let rule = r#"
.decl q(s: node)
q(s) :- kind(c, "call"), field(c, "arguments", a), child(a, s, _), kind(s, "string").
.decl objects(x: symbol, l: number, c: number)
objects(x, l, c) :- q(s), parsed(s, "sql", r), within(n, r), kind(n, "object_reference"), text(n, x), guest_place(n, s, l, c).
.decl holes(x: symbol, l: number, c: number)
holes(x, l, c) :- q(s), parsed(s, "sql", r), within(m, r), text(m, "rr_hole"), !hole_inner(m), host_node(m, h), kind(h, "interpolation"), text(h, x), guest_place(m, s, l, c).
.decl hole_inner(m: node)
hole_inner(m) :- q(s), parsed(s, "sql", r), within(m, r), text(m, "rr_hole"), within(m, p), text(p, "rr_hole"), within(p, r).
.decl annotated(s: node)
annotated(s) :- q(s), parsed(s, "annotation", _).
.output objects, holes, annotated
"#;
    let names = ["objects", "holes", "annotated"];
    let shown = guests_run_with("fstring-sql", &[("a.py", py)], rule, |o| names.iter().map(|rel| o.get(rel).map(|r| r.shown()).unwrap_or_default()).collect::<Vec<Vec<Vec<String>>>>());
    let [objects, holes, annotated] = <[Vec<Vec<String>>; 3]>::try_from(shown).unwrap();
    assert!(objects.contains(&shown_row(&["users", "2", "34"])), "{objects:?}");
    // Past an escape (`\"id\"`), `orders` keeps its column.
    assert!(objects.contains(&shown_row(&["orders", "3", "36"])), "{objects:?}");
    assert!(holes.contains(&shown_row(&["{uid}", "2", "51"])), "{holes:?}");
    assert!(holes.contains(&shown_row(&["{uid!r}", "3", "53"])) && holes.contains(&shown_row(&["{uid:>3}", "3", "69"])), "{holes:?}");
    assert!(annotated.is_empty(), "an f-string is no annotation: {annotated:?}");
}

/// An HTML file's `<script>` and `<style>` are its grammar's programs (decision 0238; 0431 §4: `embed::raw_text`, which
/// `html/embeds.rerail` describes): a script's text is
/// JavaScript, or JSON by its type, and none for a type no grammar reads; a style's text is CSS. Each stands at its
/// `raw_text`, so its nodes have the file's lines.
#[test]
fn html_scripts_and_styles_are_javascript_json_and_css_guests() {
    let html = "<!doctype html>\n<script>\nlet a = f(1);\n</script>\n<script type=\"text/template\"><b>x</b></script>\n<script type=\"module\">g(2)</script>\n<script type=\"application/ld+json\">{\"a\": 1}</script>\n<style>\np { color: red; }\n</style>\n";
    let rule = r#"
.language html
#include "html/embeds.rerail"
.decl calls(x: symbol, l: number)
calls(x, l) :- kind(t, "raw_text"), parsed(t, "javascript", p), within(c, p), kind(c, "call_expression"), text(c, x), line(c, l).
.decl decls(x: symbol, l: number)
decls(x, l) :- kind(t, "raw_text"), parsed(t, "css", p), within(n, p), kind(n, "declaration"), text(n, x), line(n, l).
.decl pairs(x: symbol, l: number)
pairs(x, l) :- kind(t, "raw_text"), parsed(t, "json", p), within(n, p), kind(n, "pair"), text(n, x), line(n, l).
.decl guests(g: symbol)
guests(g) :- kind(t, "raw_text"), parsed(t, g, _).
.decl own(x: symbol)
own(x) :- kind(t, "raw_text"), parsed(t, "javascript", p), within(c, p), kind(c, "call_expression"), text(c, x), node(d, c, _, _, _), docof(t, d).
.decl bydoc(x: symbol)
bydoc(x) :- kind(t, "raw_text"), parsed(t, "javascript", p), node(d, p, _, _, _), node(d, c, "javascript:call_expression", _, _), text(c, x).
.output calls, decls, pairs, guests, own, bydoc
"#;
    let [calls, decls, pairs, guests, own, bydoc] = <[Vec<String>; 6]>::try_from(joined(guests_run("htmlguests", &[("a.html", html)], rule, &["calls", "decls", "pairs", "guests", "own", "bydoc"]))).unwrap();
    assert_eq!(calls, ["f(1)\t3", "g(2)\t6"]);
    assert_eq!(decls, ["color: red;\t9"]);
    assert_eq!(pairs, ["\"a\": 1\t7"]);
    assert_eq!(guests, ["css", "javascript", "json"], "the template script has no program");
    // A program is named by its file and place (decision 0431 §4): its nodes' document is the file's, and a lookup by
    // the file finds every program's. A file's document is no one grammar's, so a kind read through it is qualified.
    assert_eq!(own, ["f(1)", "g(2)"], "a program's node is in its file's document");
    assert_eq!(bydoc, ["f(1)", "g(2)"], "the file's document holds both scripts' calls");
}

/// A Markdown file's inline content is its programs in tree-sitter-md's inline grammar (decision 0238; 0431 §4:
/// `embed::inline_content`, which `markdown/embeds.rerail` describes): each `inline` and `pipe_table_cell` node, its text
/// less the block continuations inside it (a quote's `> `), so its nodes stand at the file's lines and columns, past
/// the continuations too.
#[test]
fn markdown_inline_content_is_a_guest_at_its_host_places() {
    let md = "# A *b*\n\n> Some *text* and a [link](u)\n> on `two` lines.\n\n| a | **b** |\n|---|---|\n| `c` | d |\n";
    let rule = r#"
.language markdown
#include "markdown/embeds.rerail"
.decl at(k: symbol, x: symbol, l: number, c: number)
at("emphasis", x, l, c) :- parsed(_, "markdown-inline", p), within(n, p), kind(n, "emphasis"), text(n, x), line(n, l), col(n, c).
at("strong_emphasis", x, l, c) :- parsed(_, "markdown-inline", p), within(n, p), kind(n, "strong_emphasis"), text(n, x), line(n, l), col(n, c).
at("code_span", x, l, c) :- parsed(_, "markdown-inline", p), within(n, p), kind(n, "code_span"), text(n, x), line(n, l), col(n, c).
at("link_destination", x, l, c) :- parsed(_, "markdown-inline", p), within(n, p), kind(n, "link_destination"), text(n, x), line(n, l), col(n, c).
// A grammar named by a variable asks for nothing (`parsed(h, "markdown-inline", _)` beside `kind(h, k)` would ask
// for every node's text): this reads the programs the file holds.
.decl hosts(g: symbol, k: symbol)
hosts(g, k) :- parsed(h, g, _), kind(h, k).
.output at, hosts
"#;
    let [at, hosts] = <[Vec<String>; 2]>::try_from(joined(guests_run("mdinline", &[("a.md", md)], rule, &["at", "hosts"]))).unwrap();
    assert_eq!(
        at,
        [
            "code_span\t`c`\t8\t2",
            "code_span\t`two`\t4\t5",
            "emphasis\t*b*\t1\t4",
            "emphasis\t*text*\t3\t7",
            "link_destination\tu\t3\t27",
            "strong_emphasis\t**b**\t6\t6",
        ]
    );
    assert_eq!(hosts, ["markdown-inline\tinline", "markdown-inline\tpipe_table_cell"], "each program stands at its inline or cell");
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
.output tables, back, fixed, seen
"#;
    let [tables, back, fixed, seen] = <[Vec<Vec<String>>; 4]>::try_from(guests_run("tapeops", &[("a.py", py)], rule, &["tables", "back", "fixed", "seen"])).unwrap();
    // `"SELECT name \<newline> FROM users"`: the escaped newline is gone from the text that is parsed.
    assert_eq!(back, [["SELECT name  FROM users"]]);
    assert_eq!(tables, [["users", "1"]], "a value's places are its own text's");
    assert_eq!(fixed, [["SELECT name  FROM accounts"]]);
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
    let combined = crate::code_rule_dsl::logic::plan::corpus::combine(&[&a, &b]).unwrap();
    let natives = ProgramFields::natives(&*combined);
    assert_eq!(natives.len(), 2);
    for native in natives.values() {
        for input in native.reads() { assert!(combined.declares(input), "{input}"); }
    }
    let rows = lowered_findings_of(&combined);
    assert_eq!(rows.iter().filter(|r| r["code"] == "A").count(), 2, "{rows:?}");
    assert_eq!(rows.iter().filter(|r| r["code"] == "B").count(), 2, "{rows:?}");
}

/// Printed engine programs use reserved helper names; run through the internal text boundary used by run_rr.
fn complete_rule(program: &str) -> String {
    let path = crate::code_rule_dsl::fixtures::temp().with_extension("public.rerail");
    std::fs::write(&path, program).unwrap();
    let program = parse_file_version(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    print_version(&program).unwrap()
}

fn lowered_findings(program: &str) -> Vec<serde_json::Value> {
    lowered_findings_with(|root, out| {
        crate::code_rule_dsl::logic::plan::corpus::run(program, root, None, out, "program",
            crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).map(|_| ())
    })
}

/// `lowered_findings` of a program already read: the run handed it (`corpus::run_read`).
fn lowered_findings_of(program: &std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>) -> Vec<serde_json::Value> {
    lowered_findings_with(|root, out| {
        crate::code_rule_dsl::logic::plan::corpus::run_read(program, root, out, "program",
            crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18, &Default::default(), &serde_json::json!({})).map(|_| ())
    })
}

fn lowered_findings_with(run: impl FnOnce(&std::path::Path, &std::path::Path) -> crate::code_rule_dsl::logic::plan::Result<()>) -> Vec<serde_json::Value> {
    let root = crate::code_rule_dsl::fixtures::temp().with_extension("lowered-findings");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.py"), "").unwrap();
    let out = root.join("out");
    run(&root, &out).unwrap();
    // Each diagnostic, from its finding relation's typed row (`rr::lower::findings`, `tools::diagnostic`).
    let rows = crate::code_rule_dsl::tools::diagnostics(&out).unwrap();
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
        assert!(!rules_of(&original, |h| h.starts_with("rr_")).is_empty());
        let mut current = original.clone();
        for _ in 0..3 {
            current = parse(&print_version(&current).unwrap()).unwrap();
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
        let path = dir.join("g.rerail");
        std::fs::write(&path, rule).unwrap();
        let program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&path).unwrap()).unwrap();
        let out = dir.join("out");
        let report = crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        let rows: Vec<Vec<Vec<String>>> = ["tables", "objects"].iter().map(|rel| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap_or_default()).collect();
        let _ = std::fs::remove_dir_all(&dir);
        (report.memo, rows)
    };
    let (memo, first) = run("valmemo1");
    assert_eq!(memo, (0, 0), "every chunk's rows are attributed to files");
    assert_eq!(first[0].len(), 3, "{first:?}");
    assert_eq!(first[1].iter().map(|r| r[1].clone()).collect::<BTreeSet<_>>(), BTreeSet::from(["orders".to_owned(), "users".to_owned()]), "{first:?}");
    let (memo, second) = run("valmemo2");
    assert_eq!(memo, (3, 0), "every file is read from the memo");
    assert_eq!(second, first);
}

/// A combined run's programs share one batch and one set of tape values, and see the same documents (decision 0523
/// R2, R5; `.guests` once bounded each program apart, decision 0058 §1, O97). A tape value is no unscoped rule's code
/// in either (decision 0431 §3); the rule that parses it reaches it in both.
#[test]
fn combined_programs_see_the_same_documents() {
    let py = "a = \"import os\"\nb = \"import sys\"\n";
    let program = r#"
.decl top(x: symbol, l: number)
top(x, l) :- kind(i, "import_statement"), text(i, x), line(i, l).
.decl through(x: symbol)
through(x) :- kind(s, "string"), parsed(s, "python", r), within(i, r), kind(i, "import_statement"), text(i, x).
.output top, through
"#;
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("combined_guests");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), py).unwrap();
    let printed: Vec<String> = (0..2)
        .map(|i| {
            let path = dir.join(format!("p{i}.rerail"));
            std::fs::write(&path, program).unwrap();
            crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&path).unwrap()).unwrap()
        })
        .collect();
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run_all(&printed.iter().map(String::as_str).collect::<Vec<_>>(), &dir, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap_or_default();
    let (top0, top1, through0, through1) = (read("p0_top"), read("p1_top"), read("p0_through"), read("p1_through"));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(top0.is_empty() && top1.is_empty(), "a tape value is no unscoped rule's code: {top0:?} {top1:?}");
    assert_eq!(through0, [["import os"], ["import sys"]], "a guest relation reaches them");
    assert_eq!(through1, through0);
}

/// A kind set read by `nearest` over a scoped SQL node is resolved in SQL's grammar (decision 0058 §2, O98): the
/// set is copied as `clauses@sql`, its kinds qualified. The same set over an unscoped node stays the run's.
#[test]
fn kind_sets_over_a_guest_are_its_grammars() {
    let py = "def f(db):\n    db.execute(\"SELECT name FROM users\")\n";
    let rule = r#"
.decl clauses(kind: symbol)
clauses("relation"). clauses("function_definition").
.decl enclosing(x: symbol, k: symbol)
enclosing(x, k) :- kind(c, "call"), field(c, "arguments", l), child(l, s, _), kind(s, "string"), parsed(s, "sql", r), within(n, r), kind(n, "object_reference"), text(n, x), nearest(n, a, clauses), node(_, a, k, _, _).
.decl outer(k: symbol)
outer(k) :- kind(n, "call"), nearest(n, a, clauses), node(_, a, k, _, _).
.output enclosing, outer
"#;
    let [within, outer] = <[Vec<Vec<String>>; 2]>::try_from(guests_run("sqlkindset", &[("a.py", py)], rule, &["enclosing", "outer"])).unwrap();
    assert_eq!(within, [["users", "relation"]], "the set names SQL's `relation` over the guest");
    assert_eq!(outer, [["function_definition"]], "and Python's kinds over the file");
}

/// The tree axes over a guest's nodes (decision 0058 §2): `has(r, f, "field")` and `inside(x, f, "field")` name
/// `field` in the guest's grammar (a format string's, decision 0176), and the axis's other node is the guest's
/// too, so it is not bounded to the shown documents. Before, both found nothing.
#[test]
fn tree_axes_over_a_guest_are_its_grammars() {
    let py = "\"{a} {b:>{w}}\".format(a=1, b=2, w=3)\n";
    let rule = r#"
.decl fields(f: symbol)
fields(t) :- kind(s, "string"), parsed(s, "braceFormat", r), has(r, f, "field"), text(f, t).
.decl names(x: symbol)
names(t) :- kind(s, "string"), parsed(s, "braceFormat", r), within(x, r), kind(x, "name"), inside(x, _, "field"), text(x, t).
.output fields, names
"#;
    let [fields, names] = <[Vec<Vec<String>>; 2]>::try_from(guests_run("guestaxes", &[("a.py", py)], rule, &["fields", "names"])).unwrap();
    assert_eq!(fields, [["{a}"], ["{b:>{w}}"], ["{w}"]], "`has` reaches the guest's fields, nested ones too");
    assert_eq!(names, [["a"], ["b"], ["w"]], "`inside` climbs to a guest's field");
}

/// The `try` statements of `py` in a `with` block's body, read through a relation an annotation's root may fill too
/// (as `sym_of` is), with `negated` (`!inside`, `within_stop` or `!has`) excluding those a def is around or in. Hiding
/// clones the rule for annotation nodes, its axis over a copy of the kind set; folding that set into its original left
/// the clone's axis on a set that no longer was, with no rows, so the negation held for every `try`.
fn tries_in_the_block(name: &str, py: &str, negated: &str) -> Vec<Vec<String>> {
    let rule = format!(
        r#"
.decl body(b: node)
body(b) :- kind(w, "with_statement"), field(w, "body", b).
body(r) :- kind(s, "string"), parsed(s, "annotation", r).
.decl attempt(t: node)
attempt(t) :- kind(t, "try_statement").
.decl found(l: number)
found(l) :- attempt(t), body(b), {negated}, line(t, l).
.output found
"#
    );
    guests_run(name, &[("a.py", py)], &rule, &["found"]).remove(0)
}

/// A `try` on line 2, and one on line 7 inside a def nested in the block.
const TRY_IN_A_NESTED_DEF: &str = "with lock:\n    try:\n        pass\n    except E:\n        pass\n    def later():\n        try:\n            pass\n        except E:\n            pass\n";

/// `!inside(t, _, S)` over a node of a relation an annotation may fill holds only where no ancestor is in S.
#[test]
fn a_negated_inside_over_a_maybe_annotation_node_reads_its_kind_set() {
    assert_eq!(tries_in_the_block("negatedinside", TRY_IN_A_NESTED_DEF, "within(t, b), !inside(t, _, \"function_definition\")"), [["2"]]);
}

/// `within_stop(t, b, S)` from a node of a relation an annotation may fill stops at S.
#[test]
fn within_stop_over_a_maybe_annotation_node_stops_at_its_kind_set() {
    assert_eq!(tries_in_the_block("withinstop", TRY_IN_A_NESTED_DEF, "within_stop(t, b, \"function_definition\")"), [["2"]]);
}

/// `!has(t, _, S)` over a node of a relation an annotation may fill holds only where no descendant is in S: the `try`
/// on line 6 holds a def.
#[test]
fn a_negated_has_over_a_maybe_annotation_node_reads_its_kind_set() {
    let py = "with lock:\n    try:\n        pass\n    except E:\n        pass\n    try:\n        def later():\n            pass\n    except E:\n        pass\n";
    assert_eq!(tries_in_the_block("negatedhas", py, "within(t, b), !has(t, _, \"function_definition\")"), [["2"]]);
}

/// `.scope` selects a repository's files (`core.rerail`): a finding in a document outside `dirs`, or under a `tests`
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
        let mut got: Vec<String> = crate::code_rule_dsl::logic::plan::output::shown(&out, "violation").unwrap().iter().map(|r| r[3].clone()).collect();
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
#include "python/ast.rerail"
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
    let rule = dir.with_extension("scope-forms.rerail");
    std::fs::write(&rule, program).unwrap();
    let flat = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    std::fs::remove_file(&rule).unwrap();
    let paths = |options: serde_json::Value| {
        let out = dir.join("out");
        crate::code_rule_dsl::logic::plan::corpus::run_with_options(&[flat.as_str()], &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18, &Default::default(), &options).unwrap();
        let mut got: Vec<String> = crate::code_rule_dsl::logic::plan::output::shown(&out, "violation").unwrap().iter().map(|r| format!("{}:{}", r[3], r[4])).collect();
        got.sort();
        got
    };
    // `docs/d.md` is read as text (its TODO line); `docs/e.txt` matches no text glob; `tools/` is read but reports
    // nothing; `app/` only with the option.
    assert_eq!(paths(serde_json::json!({})), ["docs/d.md:2", "impl/a.py:1"]);
    assert_eq!(paths(serde_json::json!({"roots": ["app"]})), ["app/x/c.py:1", "docs/d.md:2", "impl/a.py:1"]);
    let lowered = crate::code_rule_dsl::logic::rr::parse_version(&flat).unwrap();
    assert_eq!(ProgramFields::text_globs(&*lowered), ["**/*.md"]);
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
#include "python/ast.rerail"
.violation Neg(at: node) "a doubled minus"
Neg(u) :- kind(u, "unary_operator"), op(u, "-"), field(u, "argument", a), kind(a, "unary_operator").
.decl tctx(n: node)
tctx(t) :- kind(t, "type").
tctx(r) :- tctx(s), str_node(s), parsed(s, "annotation", r).
tctx(c) :- tctx(p), child(p, c, _).
"#;
    let rule = dir.with_extension("finding-columns.rerail");
    std::fs::write(&rule, program).unwrap();
    let flat = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    std::fs::remove_file(&rule).unwrap();
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(&flat, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let mut got: Vec<String> = crate::code_rule_dsl::logic::plan::output::shown(&out, "violation").unwrap().iter().map(|r| r[3..].join(":")).collect();
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
    let rule = dir.join("rule.rerail");
    std::fs::write(&rule, r#"
.policy "t:pattern" revision 1
.language python
.violation Cut(at: node, k: symbol) "{k}"
Cut(n, k) :- n ~ `$O[:-$K]`, text(K, k).
.violation Plain(at: node) "no step"
Plain(n) :- kind(n, "subscript"), !(n ~ `$O[:-$K]`).
"#).unwrap();
    let program = crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap();
    assert_eq!(ProgramFields::ast_patterns(&*program).len(), 1);
    let text = crate::code_rule_dsl::logic::rr::print_version(&program).unwrap();
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(&text, &dir.join("src"), None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let mut got: Vec<String> = crate::code_rule_dsl::logic::plan::output::shown(&out, "violation").unwrap().iter().map(|f| format!("{}:{}", f[1], f[4])).collect();
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
    let printed = print_version(&once).unwrap();
    let twice = parse(&printed).unwrap_or_else(|e| panic!("{e:?} reading back:\n{printed}"));
    assert!(difference(&once, &twice).is_none(), "{:?}", difference(&once, &twice));
    assert_eq!(print_version(&twice).unwrap(), printed);
    for demand in ["rr_embed_req", "rr_parse_req", "rr_splice_req", "rr_file_req"] {
        assert!(once.declares(demand), "{demand} is made");
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
    let [values, order, select, group] = <[Vec<Vec<String>>; 4]>::try_from(guests_run("sqlkeywords", &[("a.py", "x = 1\n")], rule, &["values", "order", "select", "group"])).unwrap();
    assert_eq!(values, [["1"], ["2"], ["3"]]);
    assert_eq!(order, [["1", "2"], ["1", "3"], ["2", "3"]]);
    assert_eq!(select, [["3"]]);
    assert_eq!(group, [["3"]]);
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
    let [last, hit, one] = <[Vec<Vec<String>>; 3]>::try_from(guests_run("stringsteps", &[("a.py", "alpha = beta + gamma\nwolf = 1\n")], rule, &["last", "hit", "one"])).unwrap();
    let mut words: Vec<String> = last.iter().map(|l| l[1].clone()).collect();
    words.sort();
    assert_eq!(words, ["alpha", "beta", "gamma", "wolf"]);
    assert_eq!(hit, [["beta"], ["wolf"]]);
    assert_eq!(one, [["alpha"], ["beta"]]);
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
    std::fs::write(dir.join("fact-spaces-lib.rerail"), r#"
.decl def_at(f: node, s: symbol)
def_at(f, s) :- kind(f, "function_definition"), field(f, "name", n), text(n, s).
"#).unwrap();
    let policy = |name: &str, scope: &str, body: &str| -> String {
        let rule = dir.join(format!("{name}.rerail"));
        std::fs::write(&rule, format!(".policy \"t:{name}\" revision 1\n.language python\n#include \"python/ast.rerail\"\n#include \"fact-spaces-lib.rerail\"\n.scope dirs \"{scope}\"\n{body}\n")).unwrap();
        crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap()
    };
    let a = policy("a", "a", ".violation Fn(at: node) \"a def\"\nFn(f) :- def_at(f, _).");
    let b = policy("b", "b", ".violation Dup(at: node) \"two defs\"\nDup(f) :- def_at(f, s), def_at(g, s), f != g.");
    let run = |programs: &[&str], out: &str| -> Vec<String> {
        let out = dir.join(out);
        crate::code_rule_dsl::logic::plan::corpus::run_all(programs, &dir.join("src"), &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        let mut got: Vec<String> = crate::code_rule_dsl::logic::plan::output::shown(&out, "violation").unwrap_or_default().iter().map(|r| format!("{}:{}", r[1], r[3])).collect();
        got.sort();
        got
    };
    let alone: Vec<String> = [run(&[&a], "out-a"), run(&[&b], "out-b")].concat();
    assert_eq!(alone, ["Fn:a/x.py"]);
    assert_eq!(run(&[&a, &b], "out-ab"), alone, "the pack's findings are the policies' alone");
    // The library's relation is unguarded in both policies (so the simplifier makes it one); B's rule is guarded.
    let combined = crate::code_rule_dsl::logic::plan::corpus::combine(&[&a, &b]).unwrap().heads_and_reads();
    let reads_scope = |head: &str| combined.iter().filter(|(h, _)| h == head).any(|(_, reads)| reads.iter().any(|(_, x)| x.contains("rr_scope_")));
    assert!(!reads_scope("p0_def_at") && !reads_scope("p1_def_at"), "the library's rules stay as written");
    assert!(combined.iter().any(|(h, reads)| h.starts_with("p1_") && reads.iter().any(|(neg, x)| *neg && x == "p1_rr_scope_not")), "B's reads are guarded");
    // `.global`: alike (up to variable names) is shared; different is refused.
    let named = |name: &str, scope: &str, rule: &str| policy(name, scope, &format!(".global named\n.decl named(f: node)\n{rule}\n.violation N(at: node) \"n\"\nN(f) :- named(f)."));
    let c = named("c", "a", "named(f) :- kind(f, \"function_definition\").");
    let d = named("d", "b", "named(x) :- kind(x, \"function_definition\").");
    let e = named("e", "b", "named(f) :- kind(f, \"class_definition\").");
    assert!(crate::code_rule_dsl::logic::plan::corpus::combine(&[&c, &d]).is_ok());
    let refused = crate::code_rule_dsl::logic::plan::corpus::combine(&[&c, &e]).err().map(|e| e.to_string()).unwrap_or_default();
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
    let policy = |name: &str, scope: &str, body: &str| format!(".policy \"t:{name}\" revision 1\n.language python\n#include \"python/ast.rerail\"\n.scope dirs \"{scope}\"\n{body}\n");
    let a = policy("a", "a", ".violation Fn(at: node) \"a call\"\nFn(c) :- call_name(c, _).");
    let b = policy("b", "b", ".violation Dup(at: node) \"two calls\"\nDup(c) :- call_name(c, s), call_name(d, s), c != d.");
    let run = |programs: &[&str], out: &str| -> Vec<String> {
        let out = dir.join(out);
        crate::code_rule_dsl::logic::plan::corpus::run_all(programs, &dir.join("src"), &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        let mut got: Vec<String> = crate::code_rule_dsl::logic::plan::output::shown(&out, "violation").unwrap_or_default().iter().map(|r| format!("{}:{}", r[1], r[3])).collect();
        got.sort();
        got
    };
    let alone: Vec<String> = [run(&[&a], "out-a"), run(&[&b], "out-b")].concat();
    assert_eq!(alone, ["Fn:a/x.py"]);
    assert_eq!(run(&[&a, &b], "out-ab"), alone, "the pack's findings are the policies' alone");
    let combined = crate::code_rule_dsl::logic::plan::corpus::combine(&[&a, &b]).unwrap().heads_and_reads();
    let rules_of = |head: &str| combined.iter().filter(|(h, _)| h == head).collect::<Vec<_>>();
    assert!(!rules_of("call_name").is_empty() && rules_of("p0_call_name").is_empty(), "the library is held once, under its own names");
    assert!(rules_of("call_name").iter().all(|(_, reads)| reads.iter().all(|(_, x)| !x.contains("rr_scope_"))), "the library's rules are unguarded");
    assert!(combined.iter().any(|(h, reads)| h.starts_with("p1_") && reads.iter().any(|(neg, x)| *neg && x == "p1_rr_scope_not")), "B's rule is guarded");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A grammar a rule names is one the language registry has (decision 0082): a dialect's label or a fragment.
#[test]
fn a_grammar_the_registry_does_not_have_is_refused() {
    let lower = |text: &str| {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("grammar");
        std::fs::create_dir_all(&dir).unwrap();
        let rule = dir.join("g.rerail");
        std::fs::write(&rule, text).unwrap();
        let got = crate::code_rule_dsl::logic::rr::parse_file_version(&rule).map(|_| ()).map_err(|e| e.to_string());
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
    let shown: Vec<String> = shown(&p, |_| true);
    // The inner count-zero lowers to anti-existence. Its helper still carries both outer keys.
    let helper = p.rules().find(|r| r.body().filter_map(|l| l.atom()).any(|a| a.neg() && p.rel_name(a.rel()) == "ref")).expect("the inner anti-existence rule");
    // Its head's variables are the inner count's keys f and x, `ref`'s second and third (named by their places).
    let vars: BTreeSet<&str> = helper.head().args().filter_map(|t| t.var()).map(|x| p.text(helper.var(x))).collect();
    let negated = helper.body().filter_map(|l| l.atom()).find(|a| a.neg() && p.rel_name(a.rel()) == "ref").unwrap();
    let keys: BTreeSet<&str> = negated.args().skip(1).filter_map(|t| t.var()).map(|x| p.text(helper.var(x))).collect();
    assert!(is_helper(p.rel_name(helper.head().rel()), "agg") && vars.len() == 2 && vars == keys, "{shown:#?}");
    assert!(helper.reads().any(|a| is_helper(p.rel_name(a.rel()), "ctx")), "{shown:#?}");
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
    let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
    let lines = [1, 2, 4, 6, 7, 8, 10];
    let mut want: Vec<Vec<String>> = lines.iter().enumerate().flat_map(|(i, a)| lines[i + 1..].iter().map(move |b| vec![a.to_string(), b.to_string()])).collect();
    want.sort_unstable();
    want.dedup();
    assert_eq!(read("before"), want);
    assert_eq!(read("last"), [["10"]]);
    assert_eq!(read("first"), [["1"]]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A text read once (`read_text`) gives the modules `modules` gives and the programs `parse` and `parse_own` give.
#[test]
fn a_text_read_once_is_read_as_each_reader_reads_it() {
    let text = "#include \"python/ast.rerail\"\n#include \"python/calls.rerail\"\n.decl hit(c: node)\nhit(c) :- kind(c, \"call\").\n.output hit\n";
    let shown = |p: &Version| shown(p, |_| true);
    let modules = super::modules(text).unwrap().expect("modules");
    let once = super::read_text(text).unwrap().modules().expect("modules");
    assert_eq!((once.files, once.heads, once.declared), (modules.files, modules.heads, modules.declared));
    let (whole, own) = (super::parse_version(text).unwrap(), super::parse_own_version(text).unwrap());
    assert_eq!(shown(&super::read_text(text).unwrap().lower_version(true).unwrap()), shown(&whole));
    let read_own = super::read_text(text).unwrap().lower_version(false).unwrap();
    assert_eq!(difference(&read_own, &own), None);
    assert!(own.rules().count() < whole.rules().count(), "own reads the libraries' declarations, not their rules");
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

/// Every test a rule file carries passes (`rerail-engine test-rr` over `docs/rule-syntax/`). Each test is a run of
/// its own (`testing::run`, 3–9 s at O0): four run at once, so the 44 the files carry fit the test's time.
#[test]
fn rule_files_pass_their_own_tests() {
    let work = crate::code_rule_dsl::prefilter::fixtures::temp().with_extension("rr-tests");
    let mut cases = Vec::new();
    for (k, file) in files_with_tests().iter().enumerate() {
        for (j, test) in super::testing::tests(file).unwrap().into_iter().enumerate() {
            cases.push((file.clone(), test, work.join(format!("{k}-{j}"))));
        }
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let failed = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| {
                while let Some((file, test, dir)) = cases.get(next.fetch_add(1, std::sync::atomic::Ordering::Relaxed)) {
                    let why = match super::testing::run(file, test, dir) {
                        Ok(outcome) if outcome.passed() => continue,
                        Ok(outcome) => format!("{}::{}: {:?} (findings {:?})", file.display(), test.name, outcome.failures, outcome.findings),
                        Err(e) => format!("{}::{}: {e}", file.display(), test.name),
                    };
                    failed.lock().unwrap().push(why);
                }
            });
        }
    });
    let _ = std::fs::remove_dir_all(&work);
    assert!(cases.len() >= 6, "{} tests", cases.len());
    let mut failed = failed.into_inner().unwrap();
    failed.sort();
    assert!(failed.is_empty(), "{failed:#?}");
}

/// A test is no part of its file's program (decision 0130): the program, and so every key made of it, is the one the
/// file has with its tests' lines blank.
#[test]
fn a_rule_files_tests_are_no_part_of_its_program() {
    let files: Vec<(PathBuf, String)> = files_with_tests().into_iter().map(|f| {
        let text = std::fs::read_to_string(&f).unwrap();
        (f, text)
    }).collect();
    let blank: Vec<String> = files.iter().map(|(_, t)| without_tests(t)).collect();
    keep_graphs(files.iter().map(|(_, t)| t.as_str()).chain(blank.iter().map(String::as_str)));
    let printed = in_parallel(&files.iter().zip(&blank).collect::<Vec<_>>(), |((file, text), blank)| {
        let with = super::parse_version(text).unwrap();
        let without = super::parse_version(blank).unwrap();
        assert!(!super::testing::tests(file).unwrap().is_empty(), "{}", file.display());
        (format!("{with:?}"), format!("{without:?}"))
    });
    for ((file, _), (with, without)) in files.iter().zip(printed) {
        assert_eq!(with, without, "{}", file.display());
    }
}

/// Each text's include graph derived by one run of phase 0 over all of them and kept for the process
/// (`phase0::graphs`), as `phase0::keep_files` does a pack's files: lowered one by one with none kept, each text that
/// includes a module asked for a run of its own.
fn keep_graphs<'t>(texts: impl IntoIterator<Item = &'t str>) {
    let seeds: Vec<_> = texts.into_iter().filter_map(|t| super::text_seed(t).ok().flatten()).collect();
    let _ = crate::code_rule_dsl::logic::plan::corpus::phase0::graphs(&seeds);
}

/// `f` of each of `items` on a few threads, in `items`' order: each lowering is a second or less, one after another
/// they were a test of a minute (57 policies lowered twice, at the tests' opt-level).
fn in_parallel<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let out: Vec<std::sync::Mutex<Option<R>>> = items.iter().map(|_| std::sync::Mutex::new(None)).collect();
    // The test's object (decision 0442), which holds what it kept (`keep_graphs`), handed to each thread.
    let object = crate::code_rule_dsl::logic::profile::shared_here();
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                let _within = crate::code_rule_dsl::logic::profile::within(std::sync::Arc::clone(&object));
                loop {
                    let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(item) = items.get(at) else { break };
                    *out[at].lock().unwrap() = Some(f(item));
                }
            });
        }
    });
    out.into_iter().map(|r| r.into_inner().unwrap().expect("each item's result")).collect()
}

/// A failed `.expect` names its line and what the run had instead; `exactly` names the findings no `.expect` lists.
#[test]
fn failed_expectations_say_what_the_run_had() {
    let dir = crate::code_rule_dsl::prefilter::fixtures::temp().with_extension("rr-test-fail");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.rerail");
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

/// A file the report is given by name is read, tests or none: one that is not there, or that does not parse, is an
/// error. The files a directory holds are read only when they may hold a test, so a directory's other files pass unread.
#[test]
fn a_named_rule_file_is_read_with_or_without_tests() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("rr-named");
    std::fs::create_dir_all(&dir).unwrap();
    let (missing, junk, rule) = (dir.join("missing.rerail"), dir.join("junk.rerail"), dir.join("rule.rerail"));
    std::fs::write(&junk, "garbage here (\n").unwrap();
    std::fs::write(&rule, ".decl r(x: symbol)\nr(\"a\").\n").unwrap();
    let missing_report = super::testing::report(std::slice::from_ref(&missing), None);
    let junk_report = super::testing::report(std::slice::from_ref(&junk), None);
    let rule_rows = super::testing::report(std::slice::from_ref(&rule), None);
    let walked = super::testing::report(std::slice::from_ref(&dir), None);
    let _ = std::fs::remove_dir_all(&dir);
    let missing_error = missing_report.unwrap_err();
    let junk_error = junk_report.unwrap_err();
    assert!(missing_error.contains("missing.rerail"), "{missing_error}");
    assert!(junk_error.contains("junk.rerail"), "{junk_error}");
    assert_eq!(rule_rows, Ok(serde_json::json!([])));
    assert_eq!(walked, Ok(serde_json::json!([])));
}

/// A file's tests run again add no corpus root to the store: each test runs in the same directory every run
/// (`testing::in_work`), so the store's root for it, and the frames kept by its path, are the last run's. A directory
/// of each run's own made the store a root and its frames more on every run of the same tests.
#[test]
fn running_a_files_tests_again_adds_no_root_to_the_store() {
    let dir = crate::code_rule_dsl::prefilter::fixtures::temp().with_extension("rr-test-again");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.rerail");
    std::fs::write(&file, r#".policy "t:again" revision 1
.language python
.scope include "**/*.py"
.violation Call(at: node) code "C1" "a call"
Call(c) :- kind(c, "call").
.test one
    .fixture "a.py" ```
        f()
        ```
    .expect C1 at "a.py":1
.end
"#).unwrap();
    let roots = || -> std::collections::BTreeSet<String> {
        let store = crate::code_rule_dsl::logic::plan::corpus::tape_cache_path().expect("the test's store");
        let c = rusqlite::Connection::open_with_flags(&store, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let mut st = c.prepare("SELECT path FROM corpus_roots").unwrap();
        st.query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap()
    };
    let mut seen = Vec::new();
    for _ in 0..2 {
        let report = super::testing::report(std::slice::from_ref(&file), None).unwrap();
        assert_eq!(report[0]["status"], "pass", "{report}");
        seen.push(roots());
    }
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(!seen[0].is_empty(), "the test's run recorded its root");
    assert_eq!(seen[1], seen[0], "the store's corpus roots after the first run, then after the second");
}

/// A test's directory is the same every run over one store, so the store keeps one root for it; with no store
/// (`RERAIL_TAPE_CACHE=off`) nothing is kept, and each run has a directory of its own, so two processes at once never
/// share one.
#[test]
fn a_tests_directory_is_the_stores_or_the_runs_own() {
    let dir = crate::code_rule_dsl::prefilter::fixtures::temp().with_extension("rr-test-dirs");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.rerail");
    std::fs::write(&file, "").unwrap();
    let store = dir.join("store.sqlite");
    let at = |store: Option<&std::path::Path>| super::testing::in_work(store, &file, "t", |work| work.to_owned()).unwrap();
    let (first, second) = (at(Some(&store)), at(Some(&store)));
    let (own, other) = (at(None), at(None));
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(first, second, "over one store, the same directory");
    assert_ne!(own, other, "with no store, each run's own");
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
        let error = super::parse_version(text).map(|_| ()).unwrap_err().to_string();
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
    let dir = crate::code_rule_dsl::prefilter::fixtures::temp().with_extension(name);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.rerail");
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

/// A tool call is a test's input (`.tool_call`, `docs/rule-syntax/tool-calls.md`): the run is given the test's calls,
/// "1", "2", … in order, and a `.scope tool_calls` policy's finding is at the call (path "", line 0, `at` its id). A
/// call of another tool, a `--dry-run`, or the command in another argument is no finding.
#[test]
fn a_tests_tool_calls_are_its_runs_and_a_finding_is_at_the_call() {
    let policy = r#".policy "t:tool-calls" revision 1
.mode steer
.scope tool_calls
.violation Push(at: symbol) code "no-push" "a push from call {at}"
Push(c) :- tool_call(c, "Bash"), tool_input(c, "command", s), match("(?s:.*)(?:\\bgit\\s+push\\b)(?s:.*)", s), !match("(?s:.*)(?:--dry-run)(?s:.*)", s).
.test caught exactly
    .tool_call "Bash" `{"command": "cd x &&\ngit push origin main"}`
    .tool_call "Bash" `{"command": "git status"}`
    .expect "no-push"
.end
.test passes
    .tool_call "Bash" `{"command": "git push --dry-run"}`
    .tool_call "Read" `{"command": "git push"}`
    .tool_call "Bash" `{"description": "git push"}`
    .expect no "no-push"
.end
.test says_which_call
    .tool_call "Bash" `{"command": "ls"}`
    .tool_call "Bash" `{"command": "git push"}`
    .expect no "no-push"
.end
"#;
    assert_eq!(run_tests_of("rr-tool-calls", policy), [
        ("caught".to_owned(), Ok(vec![])),
        ("passes".to_owned(), Ok(vec![])),
        ("says_which_call".to_owned(), Ok(vec!["line 20: .expect no no-push: reported no-push at call \"2\"".to_owned()])),
    ]);
    let refused = run_tests_of("rr-tool-call-bad", ".decl r(x: symbol)\n.test t\n    .tool_call \"Bash\" `[1]`\n.end\n");
    assert!(matches!(&refused[..], [(_, Err(e))] if e.contains(":3: `.tool_call \"<tool>\"`")), "{refused:?}");
    // An empty fence (two backticks and nothing else) is refused with its line, not read past its ends.
    for (directive, usage) in [(".tool_call \"Bash\" ``", ":3: `.tool_call \"<tool>\"`"), (".fixture \"a.py\" ``", ":3: `.fixture \"<path>\"` then a fenced text")] {
        let refused = run_tests_of("rr-empty-fence", &format!(".decl r(x: symbol)\n.test t\n    {directive}\n.end\n"));
        assert!(matches!(&refused[..], [(_, Err(e))] if e.contains(usage)), "{directive}: {refused:?}");
    }
}

/// A policy reads tool calls when it says so, `.scope tool_calls` (as a JSON rule that reads `input` samples tool
/// calls), and then reads no file, its findings at a call (`at: symbol`).
#[test]
fn a_policy_reads_tool_calls_only_under_scope_tool_calls() {
    let head = ".policy \"t\" revision 1\n";
    let reads = "V(c) :- tool_input(c, \"command\", s), match(\"ls\", s).\n";
    let program = super::read_text(&format!("{head}.scope tool_calls\n.violation V(at: symbol) \"v\"\n{reads}")).unwrap().lower_version(false).unwrap();
    assert_eq!(crate::code_rule_dsl::logic::program_image::fields::ProgramFields::doc_scopes(&*program), Some(Vec::new()), "a policy of tool calls reads no document");
    for (text, why) in [
        (format!("{head}.decl r(s: symbol)\nr(s) :- tool_input(_, \"command\", s).\n.output r\n"), "a policy that reads tool calls says `.scope tool_calls`"),
        (format!("{head}.scope tool_calls\n.violation V(at: node) \"v\"\nV(n) :- kind(n, \"call\").\n"), "findings are at a tool call, `at: symbol`"),
        (format!("{head}.violation V(at: symbol) \"v\"\nV(s) :- tool_input(_, \"command\", s).\n"), "says `.scope tool_calls`"),
        (format!("{head}.violation V(at: symbol) \"v\"\n.decl w(s: symbol)\nw(\"a\").\nV(s) :- w(s).\n"), "only in a `.scope tool_calls` policy"),
        (format!("{head}.scope tool_calls\n.scope include \"**/*.py\"\n.violation V(at: symbol) \"v\"\n{reads}"), "no `.scope include \"**/*.py\"` beside it"),
        // A call's message names its id (`{at}`), never a property of it: a call has no line or name.
        (format!("{head}.scope tool_calls\n.violation V(at: symbol) \"v {{at.line}}\"\n{reads}"), "{at.line}: a tool call has only its id"),
        (format!("{head}.scope tool_calls\n.violation V(at: symbol) \"v {{at.name}}\"\n{reads}"), "{at.name}: a tool call has only its id"),
    ] {
        let error = super::read_text(&text).unwrap().lower_version(false).unwrap_err().to_string();
        assert!(error.contains(why), "{text}: {error}");
    }
}

/// A run of a file policy and a tool-call policy together (`docs/rule-syntax/tool-calls.md`): each finds what it finds
/// alone, the file's finding at its path and line, the call's at the call.
#[test]
fn a_run_of_file_and_tool_call_policies_finds_both() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.py"), "x = 1\nf()\n").unwrap();
    let file = dir.path().join("calls-file.rerail");
    std::fs::write(&file, ".policy \"t:calls-file\" revision 1\n.language python\n.violation Call(at: node) code \"C1\" \"a call\"\nCall(c) :- kind(c, \"call\").\n").unwrap();
    let tool = dir.path().join("calls-tool.rerail");
    std::fs::write(&tool, ".policy \"t:calls-tool\" revision 1\n.scope tool_calls\n.violation Push(at: symbol) code \"no-push\" \"a push in call {at}\"\nPush(c) :- tool_input(c, \"command\", s), match(\"(?s:.*)(?:git push)(?s:.*)\", s).\n").unwrap();
    let calls = serde_json::json!({ "toolCalls": [{"id": "e1", "tool": "Bash", "input": {"command": "git push"}}, {"id": "e2", "tool": "Bash", "input": {"command": "ls"}}] });
    let report = crate::code_rule_dsl::tools::run_policies(&[file, tool], &root, &dir.path().join("out"), &calls).unwrap();
    let mut found: Vec<_> = report["diagnostics"].as_array().unwrap().iter().map(|d| (d["code"].clone(), d["path"].clone(), d["line"].clone(), d["at"].as_str().map(str::to_owned))).collect();
    found.sort_by_key(|f| f.0.to_string());
    assert_eq!(found, [
        (serde_json::json!("C1"), serde_json::json!("a.py"), serde_json::json!(2), None),
        (serde_json::json!("no-push"), serde_json::json!(""), serde_json::json!(0), Some("e1".to_owned())),
    ], "{report}");
    // The message names the call by its id (`{at}`).
    let messages: Vec<&str> = report["diagnostics"].as_array().unwrap().iter().filter(|d| d["code"] == "no-push").filter_map(|d| d["message"].as_str()).collect();
    assert_eq!(messages, ["a push in call e1"], "{report}");
}

/// What a test may not do is refused with its line: give a core relation, give one no rule declares, use a set that
/// is not there, or have two fixtures at one path.
#[test]
fn a_test_that_cannot_run_is_refused_with_its_line() {
    let head = ".decl r(x: symbol)\nr(\"a\").\n";
    for (test, why) in [
        (".test t\n    .given kind\n.end\n", "t.rerail:4: .given kind: a core relation is the engine's"),
        (".test t\n    .given nothing\n.end\n", "t.rerail:4: .given nothing: no rule declares it"),
        (".test t uses absent\n.end\n", "t.rerail:3: no fixture set named absent"),
        (".test t\n    .fixture \"a.py\" ```\n        x\n        ```\n    .fixture \"a.py\" ```\n        y\n        ```\n.end\n", "t.rerail:3: two fixtures at \"a.py\""),
        (".test t\n    .expect r(\"a\"\n.end\n", "t.rerail:4: `.expect"),
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
    let dir = crate::code_rule_dsl::prefilter::fixtures::temp().with_extension("rr-printed");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("t.rerail"), &text).unwrap();
    let tests = super::testing::tests(&dir.join("t.rerail")).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(tests.len(), 1, "{text}");
    assert_eq!(tests[0].fixtures, [("src/a.py".to_owned(), content.as_bytes().to_vec())], "{text}");
    assert_eq!(tests[0].expects.iter().map(ToString::to_string).collect::<Vec<_>>(), ["no _ at \"src/a.py\"", "r(\"a\")"]);
}

/// `flatten` keeps a file's text as written (`program_text`), so its tests and fixture sets go with it.
#[test]
fn a_flattened_file_keeps_its_tests() {
    let file = rule_syntax().join("spikes/rr-tests/print-calls.rerail");
    let flat = crate::code_rule_dsl::tools::flatten(&file).unwrap();
    let blocks = super::testing::blocks(&std::fs::read_to_string(&file).unwrap());
    assert!(blocks.len() >= 6 && blocks.iter().any(|b| b.starts_with(".fixtures logging_app")), "{blocks:?}");
    assert_eq!(super::testing::blocks(&flat), blocks);
    assert_eq!(format!("{:?}", super::parse_version(&flat).unwrap()), format!("{:?}", super::parse_version(&without_tests(&flat)).unwrap()));
}

/// `convert-rr --cases` prints a draft's cases as tests: a caught case expects a finding in its file, a passed one
/// none. They are printed for a rule that needs a hand-written port too, and read back one test a case.
#[test]
fn converted_cases_are_tests() {
    let rule = repo().join("packs/rerail/rules/rerail-fixed-value-lists.json");
    let cases = std::path::PathBuf::from(crate::code_rule_dsl::fixtures::fixtures_dir!()).join("rerail/rerail-fixed-value-lists.cases.json");
    let report = crate::code_rule_dsl::tools::convert_rr_with(&[(rule, Some(cases.clone()))]);
    let tests = report[0]["tests"].as_str().unwrap_or_else(|| panic!("{report}"));
    let n: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&cases).unwrap()).unwrap();
    let (caught, passed) = (n["caught"].as_array().unwrap().len(), n["passed"].as_array().unwrap().len());
    let dir = crate::code_rule_dsl::prefilter::fixtures::temp().with_extension("rr-cases");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("t.rerail"), tests).unwrap();
    let read = super::testing::tests(&dir.join("t.rerail")).unwrap();
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
    let dir = crate::source_tree::native().join("../docs/rule-syntax/data-layer");
    let mut paths: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "rerail")).collect();
    paths.sort();
    let texts: Vec<String> = paths.iter().map(|p| std::fs::read_to_string(p).unwrap()).collect();
    keep_graphs(texts.iter().map(String::as_str));
    let lowered = in_parallel(&texts, |text| {
        let lowered = |full: bool| {
            super::aggregate_demand::FULL_CHECKS.with(|c| c.set(full));
            let p = super::read_text(text).and_then(|t| t.lower_version(true)).map(|p| print_version(&p).unwrap());
            super::aggregate_demand::FULL_CHECKS.with(|c| c.set(false));
            p
        };
        (lowered(false).ok(), lowered(true).ok())
    });
    let (mut read, mut demanded) = (0, 0);
    for (path, lowered) in paths.iter().zip(lowered) {
        let (Some(short), Some(whole)) = lowered else { continue };
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
    let root = crate::source_tree::native().join("../docs/rule-syntax");
    let mut paths: Vec<_> = dirs
        .iter()
        .flat_map(|d| std::fs::read_dir(root.join(d)).unwrap().map(|e| e.unwrap().path()))
        .filter(|p| p.extension().is_some_and(|e| e == "rerail"))
        .collect();
    paths.sort();
    crate::code_rule_dsl::logic::plan::corpus::phase0::keep_files(paths.iter().skip(k).step_by(n));
    let (mut read, mut differ) = (0, Vec::new());
    for path in paths.iter().skip(k).step_by(n) {
        let Ok(program) = super::parse_file_version(path) else { continue };
        let Ok(printed) = print_version(&program) else { continue };
        read += 1;
        let again = match super::read_text(&printed).and_then(|t| t.lower_version(true)).and_then(|p| print_version(&p)) {
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
                std::fs::write(std::path::Path::new(&dir).join(format!("{name}.first.rerail")), &printed).unwrap();
                std::fs::write(std::path::Path::new(&dir).join(format!("{name}.again.rerail")), &again).unwrap();
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
/// lowered. For every `.rerail` program under `dirs` that reads (the data layer's policies, and the packs', a test for
/// each group within the time a test may take), `preparation::Prune` of the program lowered from the rest is the program lowered whole, pruned: the same rules (each helper of the lowering's, numbered as it is met,
/// as many), natives and outputs, the same inputs read, and the same declarations of the relations those rules name.
fn dropping_unreached_clauses_in(dirs: &[&str], skip: &[&str], part: (usize, usize)) -> usize {
    let manifest = crate::source_tree::native().join("../docs/rule-syntax");
    let mut pending: Vec<PathBuf> = dirs.iter().map(|d| manifest.join(d)).collect();
    let mut paths = Vec::new();
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if !skip.iter().any(|s| path == manifest.join(s)) {
                    pending.push(path);
                }
            } else if path.extension().is_some_and(|e| e == "rerail") {
                paths.push(path);
            }
        }
    }
    // The `part.0`th of `part.1` parts, by position in name order: a group too long for one test is split.
    paths.sort();
    crate::code_rule_dsl::logic::plan::corpus::phase0::keep_files(paths.iter().enumerate().filter(|(i, _)| i % part.1 == part.0).map(|(_, p)| p));
    let counts: Vec<(usize, usize)> = paths.iter().enumerate().filter(|(i, _)| i % part.1 == part.0).map(|(_, p)| p).filter_map(|path| {
        let path = path.as_path();
        let Ok((statements, _)) = program(path) else { return None };
        let Ok(whole) = lower_files_dropping(path, statements.clone(), false) else { return None };
        let own = path.canonical().unwrap();
        let clauses = |s: &Files| s.iter().flat_map(|(_, items)| items).filter(|s| s.what() == Some(from_tape::S::Clause)).count();
        let (n, kept) = (clauses(&statements), clauses(&unreached_dropped(&own, statements.clone())));
        let less = lower_files_dropping(path, statements, true).unwrap_or_else(|e| panic!("{}: lowered whole, not with its unreached clauses dropped: {e}", path.display()));
        // The lowering numbers its helpers (`rr_up9`, `rr_agg7`) in the order it meets them, and `simplify` the
        // variables it renames apart (`rr_earlier27_f18`, `_t<k>` and `_f<k>` after the name, simplify.rs); dropped
        // clauses no longer take numbers before them, and a context or cover made once is read again where an equal
        // one is asked for (`Lowerer::helper_once`), so its rules sit where the first asker, maybe a dropped clause,
        // made them. So the programs are compared with every helper's number masked (`rr_agg#`, `rr_earlier#_f18`),
        // and a helper's rules by its stem alone (an aggregate's columns are its fresh variables in the order of
        // their names, which the numbers change).
        // A helper variable a reader's ask takes (decision 0291) keeps its number before the ask's suffix
        // (`rr_text20_rr_d`): masked as the helper is.
        let helper = regex::Regex::new(r"\brr_([a-z_]*[a-z])([0-9]+)((?:_rr_d)?)\b").unwrap();
        // A helper named by its module (decision 0639 §4) takes the module's digest, which the dropped clauses change.
        let by_module = regex::Regex::new(r"\brr_m_([a-z_]*[a-z])_[0-9a-f]{32}\b").unwrap();
        let fresh = regex::Regex::new(r"\brr_([a-z]+)([0-9]+)_([ft][0-9]+)\b").unwrap();
        let shape = |read: Arc<crate::code_rule_dsl::logic::program_image::Version>| {
            use crate::code_rule_dsl::logic::plan::corpus::preparation;
            use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;
            // Pruned as a corpus run prunes it: the requests the engine reads are outputs where a rule makes them
            // (`corpus::run_observed`), those a library derives (embeds, loads) and those lowering makes of a body's
            // file and guest reads (`rr_dir_req`, `rr_embed_req` …), which the engine answers for every reader.
            let requests = super::LIBRARY_DERIVED.iter().copied().chain(super::WORLD.iter().map(|(_, req)| *req)).chain(["rr_embed_req", "rr_parse_req", "rr_splice_req"]);
            let outputs = read.output_names();
            let mut b = crate::code_rule_dsl::logic::program_image::Builder::on(&read);
            for rel in requests {
                if read.rules().any(|r| read.rel_name(r.head().rel()) == rel) && !outputs.iter().any(|o| o == rel) {
                    b.fields().add_output(rel);
                }
            }
            // Then simplified and pruned again, as `corpus::run_observed` prepares it: identical relations merge
            // (`MergeIdentical`), so a widened copy (decision 0174) whose kept rules are its relation's own once its
            // unreached readers' requests are pruned is read as that relation.
            let mut pipeline = crate::code_rule_dsl::logic::program_image::manager::Pipeline::new("q3", b.finish().unwrap().0);
            let mut prune = preparation::Prune::default();
            pipeline.program(&mut prune);
            preparation::simplify::passes(&mut pipeline).unwrap();
            pipeline.program(&mut prune);
            let p = pipeline.finish().unwrap().0;
            let head = |r: crate::code_rule_dsl::logic::program_image::RuleRef<'_>| p.rel_name(r.head().rel()).to_owned();
            let reads = |r: crate::code_rule_dsl::logic::program_image::RuleRef<'_>| -> Vec<String> { r.body().filter_map(|l| l.atom()).map(|a| p.rel_name(a.rel()).to_owned()).collect() };
            let rules_of = |rel: &str| -> Vec<String> { p.rules().filter(|r| head(*r) == rel).map(|r| p.show_rule(r)).collect() };
            // Q3's property itself: no widened copy is left with its relation's own rules, each copy read as its
            // relation (a recursive copy reads itself, a copy of `r` reads the copy of what `r` reads). The classes of
            // relations equal with every equal one read as its class (`congruence`, the greatest such partition) hold
            // no copy with its relation: the merge would have made them one. (A copy whose read copy differs, as a
            // cross-file `node_text` read makes it, is rightly its own.)
            let copies = lower::ON_REQUEST.iter().flat_map(|round| round.iter()).map(|a| format!("rr_wide_{}_", a.rel)).collect::<Vec<_>>();
            let source = |rel: &str| -> String {
                let mut at = rel;
                while let Some(r) = copies.iter().find_map(|pre| at.strip_prefix(pre.as_str())) {
                    at = r;
                }
                at.to_owned()
            };
            let data = crate::code_rule_dsl::logic::program_image::manager::analysis::<crate::code_rule_dsl::logic::plan::fields::FieldsData>(&p).unwrap();
            let classes = preparation::congruence(&p, crate::code_rule_dsl::logic::plan::fields::Fields::over(&p, &data), false);
            for class in &classes {
                for w in class.iter().filter(|w| source(w) != **w) {
                    let r = source(w);
                    assert!(!class.contains(&r), "{}: {w} is left with {r}'s own rules\n {w}: {:#?}\n {r}: {:#?}", path.display(), rules_of(w), rules_of(&r));
                }
            }
            // Why a copy is needed (`RERAIL_WIDE_WITNESS`, from the tape-lab branch's form of this check): a copy kept
            // apart from its relation reads a copy that is not its relation's equal.
            if std::env::var_os("RERAIL_WIDE_WITNESS").is_some() {
                for w in p.declared_names().iter().filter(|w| source(w) != w.as_str()) {
                    let differs: Vec<String> = p.rules().filter(|r| head(*r) == *w).flat_map(reads).filter(|c| source(c) != *c).collect();
                    eprintln!("{}: {w} needed: reads {differs:?}: {:#?} against {:#?}", path.display(), rules_of(w), rules_of(&source(w)));
                }
            }
            // A desugared clause's variables (`k_rr_d117_4`, renamed apart `t_rr_d699_3_f258`) are numbered by the
            // clauses before it too, inside a variable's name where `helper` sees no word boundary.
            let desugared = regex::Regex::new(r"rr_d[0-9]+_([0-9]+)(?:_([ft])[0-9]+)?").unwrap();
            // And a fresh variable's number before it (`rr_text20_rr_d#_0`).
            let fresh_desugared = regex::Regex::new(r"\brr_([a-z_]*[a-z])[0-9]+_rr_d#").unwrap();
            let mask = |s: &str| {
                let s = desugared.replace_all(s, |c: &regex::Captures| match c.get(2) {
                    Some(k) => format!("rr_d#_{}_{}#", &c[1], k.as_str()),
                    None => format!("rr_d#_{}", &c[1]),
                });
                let s = fresh_desugared.replace_all(&s, "rr_${1}#_rr_d#");
                let s = by_module.replace_all(&s, "rr_m_${1}_#");
                helper.replace_all(&fresh.replace_all(&s, "rr_${1}#_$3"), "rr_$1#$3").into_owned()
            };
            // A violation's rule for `@place` values (`rr_v0 < -1`) is made when any rule of the program makes one
            // (`lower`'s `place_values`): an unreached clause made it for the whole program, where no reached
            // rule makes the value it reads.
            let place_variant = |shown: &str| shown.starts_with("violation(") && shown.split(":-").next().is_some_and(|h| h.contains("place_path("));
            // A helper's rules (`rr_agg7`, its columns its fresh variables in the order of their names, which the
            // numbers change) are compared by their stem; every other rule exactly, its helpers' numbers masked. A
            // widened copy (`rr_wide_node_text_rr_up9`) is compared by its rules.
            let numbered = |rel: &str| helper.is_match(rel) && !rel.starts_with("rr_wide_");
            let mut rules: Vec<String> = p
                .rules()
                .map(|r| (head(r), p.show_rule(r)))
                .filter(|(_, shown)| !place_variant(shown))
                .map(|(h, shown)| if numbered(&h) { format!("helper {}", mask(&h)) } else { mask(&shown) })
                .collect();
            rules.sort();
            let named: BTreeSet<String> = p.rules().flat_map(|r| std::iter::once(head(r)).chain(reads(r))).collect();
            let mut decls: Vec<String> = p
                .declared_names()
                .into_iter()
                .filter(|r| named.contains(r))
                .map(|r| {
                    let c = p.columns_of(&r).unwrap_or_default();
                    if numbered(&r) { format!("{}/{}", mask(&r), c.len()) } else { mask(&format!("{r}{c:?}")) }
                })
                .collect();
            decls.sort();
            let natives_map = ProgramFields::natives(&*p);
            let natives: BTreeSet<String> = natives_map.keys().map(|n| mask(n)).collect();
            let outputs = p.output_names();
            // The facts of the relations the rules name (`simplify` makes ground rules facts), and the outputs'.
            let mut facts: Vec<String> = p.fact_names().into_iter().filter(|r| named.contains(r) || outputs.contains(r)).map(|r| mask(&format!("{r}{:?}", p.fact_rows(&r)))).collect();
            facts.sort();
            // The inputs the rules read: the lowering declares one (`node_order`, `rr_inside_<set>`) for each clause
            // that reads it, and an unreached clause's stays among the inputs after `prune`, read by none.
            // Read by an atom of a rule, or named by a native: a string of a rule (Python's field `"value"`) is no read.
            let read_rels: BTreeSet<String> = p.rules().flat_map(reads).collect();
            let native_text = format!("{natives_map:?}");
            let mut inputs: Vec<String> = p.input_names().into_iter().filter(|i| read_rels.contains(i) || native_text.contains(&format!("\"{i}\""))).collect();
            inputs.sort();
            let mut outputs = outputs;
            outputs.sort();
            ((rules, decls, natives, outputs, inputs, facts), Arc::clone(&p))
        };
        let ((a, kept_less), (b, kept_whole)) = (shape(less), shape(whole));
        if a != b {
            // The first relation only the whole keeps, and the kept rules that read it, back to an output.
            let heads = |v: &crate::code_rule_dsl::logic::program_image::Version| v.rules().map(|r| v.rel_name(r.head().rel()).to_owned()).collect::<BTreeSet<String>>();
            let less_heads = heads(&kept_less);
            let w = &kept_whole;
            let mut chain: Vec<String> = Vec::new();
            if let Some(extra) = heads(w).into_iter().find(|h| !less_heads.contains(h) && !helper.is_match(h)) {
                let mut at = extra;
                while chain.len() < 12 && !chain.contains(&at) {
                    chain.push(at.clone());
                    let reader = w.rules().find(|r| w.rel_name(r.head().rel()) != at && r.body().filter_map(|l| l.atom()).any(|a| w.rel_name(a.rel()) == at));
                    match reader {
                        Some(r) => {
                            chain.push(w.show_rule(r));
                            at = w.rel_name(r.head().rel()).to_owned();
                        }
                        None => break,
                    }
                }
            }
            // What `x` has more of than `y` (a rule twice on one side is a difference).
            let only = |x: &Vec<String>, y: &Vec<String>| {
                let mut left = y.clone();
                x.iter().filter(|r| match left.iter().position(|l| l == *r) {
                    Some(i) => {
                        left.remove(i);
                        false
                    }
                    None => true,
                }).take(3).cloned().collect::<Vec<_>>()
            };
            panic!(
                "{}: its pruned program changed with its unreached clauses dropped\n read by: {chain:?}\n rules only dropped: {:?}\n rules only whole: {:?}\n decls only dropped: {:?}\n decls only whole: {:?}\n natives {:?} vs {:?}\n outputs {:?} vs {:?}\n inputs {:?} vs {:?}\n facts only dropped: {:?}\n facts only whole: {:?}",
                path.display(), only(&a.0, &b.0), only(&b.0, &a.0), only(&a.1, &b.1), only(&b.1, &a.1), a.2, b.2, a.3, b.3,
                (a.0.len(), a.1.len(), a.4.len(), only(&a.4, &b.4)), (b.0.len(), b.1.len(), b.4.len(), only(&b.4, &a.4)),
                only(&a.5, &b.5), only(&b.5, &a.5)
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


/// tsx/es-globals.rerail is es_global's rows: the engine's ECMAScript globals table for `ecmaVersion: "latest"`, name for
/// name (the scope code reads the same table, `scope::options::EsGlobals`).
#[test]
fn es_globals_match_the_engines_table() {
    let table: serde_json::Value = serde_json::from_str(include_str!("../../../../../../src/code_rule_dsl/scope/data/es_globals.generated.json")).unwrap();
    let latest = table["latest"].as_u64().unwrap().to_string();
    let want: std::collections::BTreeSet<String> = table["versions"][&latest].as_object().unwrap().keys().cloned().collect();
    let rows = include_str!("../../../../../../../docs/rule-syntax/lib/tsx/es-globals.rerail");
    let got: std::collections::BTreeSet<String> = rows
        .lines()
        .filter_map(|l| l.strip_prefix("es_global(\"")?.strip_suffix("\").").map(str::to_owned))
        .collect();
    assert!(!want.is_empty());
    assert_eq!(got, want, "regenerate tsx/es-globals.rerail from es_globals.generated.json's version {latest}");
}

/// What only the engine reads is reached: a library's request rule, whose head nothing reads but whose body reads
/// the file system (`py_listed(d) :- py_file_at(_, _, d, _), dir_entry(d, _, _)`), and a rule deriving one of the
/// engine's relations (`load(f, q, "python")`). Dropped, a run loading modules listed no directory and loaded
/// nothing (`the_cone_of_requests_is_kept_while_its_reads_and_answers_are`).
#[test]
fn dropping_unreached_clauses_keeps_what_the_engine_reads() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("engine-reads");
    std::fs::create_dir_all(&dir).unwrap();
    let rule = dir.join("p.rerail");
    std::fs::write(&rule, "#include \"python/modules.rerail\"\n.decl n_loaded(p: symbol)\nn_loaded(p) :- loaded(p, _, _).\n.output n_loaded\n").unwrap();
    let p = parse_file_version(&rule).unwrap();
    let heads: std::collections::BTreeSet<&str> = p.rules().map(|r| p.rel_name(r.head().rel())).collect();
    assert!(heads.contains("load"), "load's rule is kept");
    assert!(heads.contains("py_listed"), "the request rule is kept");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two programs run as one: each one's `has` over a guest's nodes still finds them. The merge names kind sets by
/// their kinds; it renamed `inside`'s axis to the shared set but left `has`'s on the program's own set name, which
/// guest hiding then could not name in the guest's grammar: no row.
#[test]
fn has_over_a_guest_holds_with_two_programs() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("has2");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "def f(db):\n    db.execute(\"SELECT name FROM users\")\n").unwrap();
    let text = |clause: &str, body: &str| {
        let path = dir.join(format!("{clause}.rerail"));
        std::fs::write(&path, format!(
            ".violation {clause}(at: node) code \"{clause}\" \"m\"\n{clause}(n) :- {body}.\n"
        )).unwrap();
        crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&path).unwrap()).unwrap()
    };
    let has = text("has_ref", "kind(c, \"call\"), field(c, \"arguments\", a), child(a, s, _), kind(s, \"string\"), parsed(s, \"sql\", r), has(r, _, \"object_reference\"), n = s");
    let other = text("calls", "kind(n, \"call\")");
    let out = dir.join("out");
    let options = serde_json::json!({ "memo": "off" });
    crate::code_rule_dsl::logic::plan::corpus::run_with_options(&[has.as_str(), other.as_str()], &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18, &Default::default(), &options).unwrap();
    // The findings as the image holds them (decision 0249 §6), not as text.
    let rows = crate::code_rule_dsl::tools::diagnostics(&out).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let codes: std::collections::BTreeSet<&str> = rows.iter().filter_map(|r| r["code"].as_str()).collect();
    assert!(codes.contains("has_ref") && codes.contains("calls"), "both programs' findings: {rows:?}");
}

/// Decision 0246 §1: a pack's library lowered one module at a time, each module against the modules it reaches, and
/// linked, is the program lowering its files as one gives. Every pack's library, in the order a run of the pack
/// lists its files (each program's includes first).
#[test]
fn library_modules_link_to_the_lowering_of_them_together() {
    let packs = crate::code_rule_dsl::logic::rr::library().join("../packs");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&packs).unwrap().map(|e| e.unwrap().path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    let mut checked = 0;
    // Every pack's programs' include graphs in one run of phase 0 (`modules_of`), not one a program or a pack (Q361).
    let srcs: Vec<(PathBuf, String)> = dirs.iter().flat_map(|dir| {
        let mut programs = walk_rr(dir);
        programs.sort();
        programs.into_iter().filter_map(|p| Some((dir.clone(), std::fs::read_to_string(&p).ok()?))).collect::<Vec<_>>()
    }).collect();
    let mut found: Vec<(PathBuf, Result<Option<Modules>, Error>)> = srcs.iter().map(|(d, _)| d.clone()).zip(super::modules_of(&srcs.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>())).collect();
    for dir in dirs {
        let mut paths: Vec<PathBuf> = Vec::new();
        for (_, m) in found.extract_if(.., |(d, _)| *d == dir) {
            let Ok(Some(m)) = m else { continue };
            for (path, _, _) in m.files {
                if !paths.contains(&path) {
                    paths.push(path);
                }
            }
        }
        if paths.is_empty() {
            continue;
        }
        // Lowered together in the link's order: phase 0's plan, its modules by rank, then by key (a key names its scope's
        // interfaces, so two modules of one rank may come in another order than the include order).
        let (modules, planned_files, _) = super::library_modules_planned(&paths).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        let files: Files = planned_files.iter().flatten().map(|p| (p.clone(), read(p).unwrap().0.into_iter().filter(|s| !matches!(s.what(), Some(from_tape::S::Include(_)))).collect())).collect();
        let together = super::lower_statements_version(files.iter().flat_map(|(_, s)| s.iter().cloned()).collect()).unwrap();
        // Each module read back from its image (decision 0246 §6) links to the same program, on versions (0377).
        let images: Vec<lower::Module> = modules.iter().map(|m| module_image::decode(std::sync::Arc::new(module_image::encode(m).unwrap())).unwrap_or_else(|e| panic!("{}: a module image: {e}", dir.display()))).collect();
        let on_versions = super::link_library_versions(images).unwrap_or_else(|e| panic!("{}: linked on versions from its images: {e}", dir.display()));
        let linked = super::link_library_versions(modules).unwrap_or_else(|e| panic!("{}: linked on versions: {e}", dir.display()));
        assert_eq!((linked.size(), linked.digest()), (on_versions.size(), on_versions.digest()), "{}: the library linked from its modules' images: {}", dir.display(), crate::code_rule_dsl::logic::program_image::differences(&linked, &on_versions));
        // Printed, to compare with the library lowered as one (whose helpers may be numbered apart).
        let printed = |v: &std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>| print_version(v).unwrap();
        let from_images = printed(&on_versions);
        let (a, b) = (printed(&together), printed(&linked));
        let differ = |what: &str, a: &str, b: &str| {
            let (la, lb): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
            let first = la.iter().zip(&lb).position(|(x, y)| x != y).unwrap_or(la.len().min(lb.len()));
            // The lines one side has and the other not, helper numbers aside (`rr_agg169` and `rr_agg167` are one).
            let plain = |l: &str| l.split(|c: char| !(c.is_alphanumeric() || c == '_')).map(|w| if w.starts_with("rr_") { w.trim_end_matches(|c: char| c.is_ascii_digit()) } else { w }).collect::<Vec<_>>().join(" ");
            let (sa, sb): (BTreeSet<String>, BTreeSet<String>) = (la.iter().map(|l| plain(l)).collect(), lb.iter().map(|l| plain(l)).collect());
            let only = |x: &BTreeSet<String>, y: &BTreeSet<String>| x.difference(y).take(30).cloned().collect::<Vec<_>>().join("\n    ");
            panic!("{}: {} files; {what} differs at line {first} ({} lines and {}):\n  {:?}\n  {:?}\n  only the first:\n    {}\n  only the second:\n    {}", dir.display(), paths.len(), la.len(), lb.len(), la.get(first), lb.get(first), only(&sa, &sb), only(&sb, &sa));
        };
        // Line by line, the numbers of what a walk makes aside: a module's walk counts from its own start (decision
        // 0639 step 3), the library lowered as one walk from the library's; and a module-named helper's module digest
        // (`rr_m_up_<digest>`), the library's there.
        let numbered = |l: &str| l.split(|c: char| !(c.is_alphanumeric() || c == '_')).map(|w| match w.strip_prefix("rr_m_") {
            Some(rest) => format!("rr_m_{}", rest.rsplit_once('_').map_or(rest, |(stem, _)| stem)),
            None if w.starts_with("rr_") => w.trim_end_matches(|c: char| c.is_ascii_digit()).to_owned(),
            None => w.to_owned(),
        }).collect::<Vec<_>>().join(" ");
        if a.lines().map(numbered).ne(b.lines().map(numbered)) {
            differ("the library linked from its modules, from the one lowered together,", &a, &b);
        }
        if b != from_images {
            differ("the library linked from its modules' images, from the one linked from its modules,", &b, &from_images);
        }
        checked += 1;
    }
    assert!(checked > 5, "{checked} packs with a library");
}

fn walk_rr(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk_rr(&p));
        } else if p.extension().is_some_and(|x| x == "rerail") {
            out.push(p);
        }
    }
    out
}

/// Decision 0263: a library's relations are closed. A program adds no rules or facts to a relation a library file it
/// includes declares, unless the declaration says `extensible` (`cmp_same_ask`, a request of `python/compare.rerail`).
#[test]
fn a_program_adds_only_to_extensible_library_relations() {
    let closed = "#include \"python/ast.rerail\"\nunparen(a, a) :- kind(a, \"call\").\n";
    let refused = super::read_text(closed).unwrap().lower_version(false).expect_err("unparen is not extensible");
    assert!(refused.to_string().contains("`unparen` is declared in") && refused.to_string().contains("decision 0263"), "{refused}");
    let open = "#include \"python/compare.rerail\"\n.decl hit(a: node)\ncmp_same_ask(a, a) :- kind(a, \"call\").\nhit(a) :- cmp_same(a, a).\n.output hit\n";
    super::read_text(open).unwrap().lower_version(false).expect("cmp_same_ask is extensible");
}

/// Decision 0275: a text's header is kept under its bytes with the library files its includes read, and found again:
/// the header found is the header read, and another text's is its own.
#[test]
fn a_texts_header_is_found_under_its_bytes() {
    let text = "#include \"core.rerail\"\n.policy \"test:header-0275\" revision 1\n.language python\n.decl out(x: symbol)\n.output out\n";
    let header = text_header(text).unwrap();
    let kept = kept_header_found(text).expect("kept by the first read");
    assert_eq!(format!("{kept:?}"), format!("{header:?}"), "the header kept is the header read");
    assert!(ProgramFields::settings(&*kept).iter().any(|(k, v)| k == "policy" && v.contains("test:header-0275")), "{kept:?}");
    assert_eq!(format!("{:?}", text_header(text).unwrap()), format!("{header:?}"), "read again from the entry");
    let other = text.replace("header-0275", "header-0275-b");
    assert!(kept_header_found(&other).is_none(), "another text: its own entry");
}

/// Decision 0275: a rule file's program is kept under its path and sha with the files it read, and found again while
/// each still reads as recorded: by its `stat`, and by its sha where the stat moved. An edit to an included file reads
/// the program again; its bytes put back, the stat moved but the sha holds, and the entry is found again.
#[test]
fn a_rule_files_program_is_found_while_what_it_read_holds() {
    let dir = tempfile::tempdir().unwrap();
    let (main, helper) = (dir.path().join("main.rerail"), dir.path().join("helper_0275.rerail"));
    let original = ".decl helper(x: symbol)\nhelper(\"a\").\n";
    std::fs::write(&helper, original).unwrap();
    std::fs::write(&main, "#include \"helper_0275.rerail\"\n.decl out(x: symbol)\nout(x) :- helper(x).\n.output out\n").unwrap();
    let main = main.canonical().unwrap();
    let found = || kept_program_found(&main);
    assert!(found().is_none(), "nothing kept before the first read");
    let (program, files) = read_file(&main).unwrap();
    let (kept, kept_files) = found().expect("kept by the first read");
    assert_eq!(format!("{kept:?}"), format!("{program:?}"), "the program kept is the program read");
    assert_eq!(kept_files, files, "with the files it read, in reading order");
    std::fs::write(&helper, ".decl helper(x: symbol)\nhelper(\"b\").\n").unwrap();
    assert!(found().is_none(), "an included file edited: read again");
    std::fs::write(&helper, original).unwrap();
    assert!(found().is_some(), "its bytes put back: its sha holds though its stat moved");
}

/// Decision 0089 §3 with 0174: a program that adds to a library relation takes the library's readers of it (its
/// cone), not the on-request rewrite's rules. Through those (`x`'s readers' request for `value`, `value`'s answer) the
/// cone reached every reader of the attribute: `z` here, and 1,300 of the Python pack's library rules in each of its
/// five programs that ask of the library.
#[test]
fn a_cone_is_taken_without_the_attributes_requests() {
    let decls = ".decl kind(n: node, k: symbol)\n.input kind\n.decl value(n: node, v: symbol)\n.input value\n.decl x(n: node)\n.decl y(n: node, v: symbol)\n.decl z(n: node, v: symbol)\n";
    let library = parse(&format!("{decls}x(n) :- kind(n, \"call\").\ny(n, v) :- x(n), value(n, v).\nz(n, v) :- kind(n, \"name\"), value(n, v).\n.output y, z\n")).unwrap();
    assert!(!rules_of(&library, |h| h == "rr_value_req").is_empty(), "the library reads `value` on request: {:?}", shown(&library, |_| true));
    // As `Text::lower_in_version` does: the program's own rewrite undone, the cone added, the rewrite made again over
    // both (`plan::corpus::cone_on_version`).
    let program = parse(&format!("{decls}x(n) :- kind(n, \"attribute\").\n")).unwrap();
    let declared: BTreeSet<String> = ["kind", "value", "x", "y", "z"].into_iter().map(str::to_owned).collect();
    let coned = crate::code_rule_dsl::logic::plan::corpus::cone_on_version(&program, &library, &["x".to_owned()].into(), &declared).unwrap();
    let heads: BTreeSet<String> = coned.rules().map(|r| coned.rel_name(r.head().rel()).to_owned()).collect();
    assert!(heads.contains("y"), "`y` reads `x`: {heads:?}");
    assert!(!heads.contains("z"), "`z` reads only `value`: {heads:?}");
    let asks: Vec<String> = shown(&coned, |h| h == "rr_value_req");
    assert!(asks.len() == 1 && asks[0].ends_with(":- x($n)"), "`y`'s nodes are asked for, by `x`: {asks:?}");
}

/// Decision 0174 §6: an attribute a cross-file reader asks for stays eager. `@rr_node_value` reads the node's document
/// from the chunk's batch, which the cross-file part does not hold: answered there, every reader of `value` saw no
/// value, the file-local ones too (`vals`), as the Python pack's `__all__` and `exports` readers made it.
#[test]
fn an_attribute_a_cross_file_reader_asks_for_is_eager() {
    let rule = r#"
.policy "t:values" revision 1
.decl pick(n: node)
pick(n) :- kind(n, "integer"), kind(m, "integer"), docof(n, d), docof(m, e), d != e.
.decl cv(v: symbol)
cv(v) :- pick(n), value(n, v).
.decl vals(v: symbol)
vals(v) :- kind(n, "integer"), value(n, v).
.decl ends(l: number)
ends(l) :- kind(n, "expression_statement"), last_token(n, t), line(t, l).
.output cv, vals, ends
"#;
    let [cv, vals, ends] = <[Vec<String>; 3]>::try_from(joined(guests_run("cross-value", &[("a.py", "x = 1\n"), ("b.py", "x = 2\n")], rule, &["cv", "vals", "ends"])).into_iter().map(|mut rows| { rows.sort(); rows }).collect::<Vec<_>>()).unwrap();
    assert_eq!(vals, ["1", "2"], "the file-local reader's values");
    assert_eq!(cv, ["1", "2"], "the cross-file reader's values");
    // With `value`'s round undone the program still has core.rerail's shape: its token leaves are made.
    assert_eq!(ends, ["1"], "a token leaf's reader");
}

/// C's directives as facts (decision 0327, mode (a)): a rule reading `c_include`, `c_guard` or `c_define` of a path
/// asks for the file's directives (`c_pp_file_req`), answered inside the fixpoint as a file's lines are. Only the
/// relations the program declares are made. `h.h` is no document (a `.h` file is C++'s): it is asked for in the next
/// wave, once `a.c`'s include names it.
#[test]
fn c_directives_are_answered_by_file() {
    let rule = r#"
.language c
#include "c/preprocess.rerail"
.decl c_file(p: symbol)
c_file(p) :- kind(n, "translation_unit"), docof(n, d), doc(d, p, _).
.decl inc(p: symbol, form: symbol, s: symbol)
inc(p, f, s) :- c_file(p), c_include(p, _, "include", f, s).
.decl seen(p: symbol)
seen(p) :- c_file(p).
seen(h) :- seen(p), c_include(p, _, "include", "quote", s), h = substr(s, 1, strlen(s) - 2).
.decl guard(p: symbol, n: symbol)
guard(p, n) :- seen(p), c_guard(p, _, n).
.decl macro(p: symbol, n: symbol, ps: symbol, v: symbol)
macro(p, n, ps, v) :- seen(p), c_define(p, _, n, 1, ps, v, _, _).
.output inc, guard, macro
"#;
    let a = "#include \"h.h\"\n#include <stdio.h>\n#define MAX(a, b) ((a) > (b) ? (a) : (b))\n#define LOG(fmt, ...) printf(fmt, __VA_ARGS__)\nint main(void) { return MAX(1, 2); }\n";
    let h = "#ifndef H_H\n#define H_H\n#define \\\n  SQ(x) ((x) * (x))\nint f(int);\n#endif\n";
    let [inc, guard, macros] = <[Vec<Vec<String>>; 3]>::try_from(guests_run("cdirectives", &[("a.c", a), ("h.h", h)], rule, &["inc", "guard", "macro"])).unwrap();
    assert_eq!(inc, [shown_row(&["a.c", "angle", "<stdio.h>"]), shown_row(&["a.c", "quote", "\"h.h\""])]);
    assert_eq!(guard, [shown_row(&["h.h", "H_H"])]);
    assert_eq!(macros, [shown_row(&["a.c", "LOG", "fmt", "standard"]), shown_row(&["a.c", "MAX", "a,b", ""]), shown_row(&["h.h", "SQ", "x", ""])]);
}

/// Decision 0327, mode (b): a translation unit preprocessed under each configuration rules give it. The
/// configurations differ by a predefined macro, which takes another branch of the unit's `#ifdef` and of a header's
/// `#if`, and so enters another header, found by the search path (`sys`).
#[test]
fn c_units_are_preprocessed_under_their_configurations() {
    let rule = r#"
.language c
#include "c/preprocess.rerail"
.decl c_file(p: symbol)
c_file(p) :- kind(n, "translation_unit"), docof(n, d), doc(d, p, _).
c_lang("on", "c", "gnu17").
c_lang("off", "c", "gnu17").
c_predefine("on", 0, "FAST", "1").
c_predefine("on", 1, "LEVEL", "2").
c_predefine("off", 1, "LEVEL", "1").
c_include_dir(c, 0, "sys", "angle") :- c_lang(c, _, _).
c_tu(p, c) :- c_file(p), c_lang(c, _, _).
.decl taken(c: symbol, p: symbol, at: number, t: number)
taken(c, p, at, t) :- c_pp_cond_taken(_, s, p, at, t), c_cfg_spec(c, s).
.decl entered(c: symbol, p: symbol, d: number)
entered(c, p, d) :- c_pp_included(_, s, _, _, p, d), c_cfg_spec(c, s).
.decl read(c: symbol, p: symbol)
read(c, p) :- c_pp_read(_, s, p), c_cfg_spec(c, s).
.output taken, entered, read
"#;
    let a = "#include \"h.h\"\n#ifdef FAST\nint fast;\n#else\nint slow;\n#endif\n";
    let h = "#if LEVEL > 1\n#include <deep.h>\n#endif\n";
    let got = joined(guests_run("cunits", &[("a.c", a), ("h.h", h), ("sys/deep.h", "int deep;\n")], rule, &["taken", "entered", "read"]));
    let [taken, entered, read] = <[Vec<String>; 3]>::try_from(got).unwrap();
    assert_eq!(taken, ["off\ta.c\t15\t0", "off\ta.c\t37\t1", "off\th.h\t0\t0", "on\ta.c\t15\t1", "on\ta.c\t37\t0", "on\th.h\t0\t1"]);
    assert_eq!(entered, ["off\th.h\t1", "on\th.h\t1", "on\tsys/deep.h\t2"]);
    assert_eq!(read, ["off\ta.c", "off\th.h", "on\ta.c", "on\th.h", "on\tsys/deep.h"]);
}

/// Decision 0327, `c/includes.rerail`: each `#include` of the files a unit reaches resolves as a driver's search does,
/// over `dir_entry` and mode (a)'s rows. `<x.h>` is the angle directory's though the includer's directory and a
/// system one have one too; its `#include_next` goes on to the system one; a name no search finds is unresolved.
/// The closure is the files mode (b) enters, here where no branch is skipped.
#[test]
fn c_includes_resolve_by_the_search_order() {
    let rule = r#"
.language c
#include "c/includes.rerail"
.decl c_file(p: symbol)
c_file(p) :- kind(n, "translation_unit"), docof(n, d), doc(d, p, _).
c_lang("c", "c", "gnu17").
c_include_dir("c", 0, "sys1", "angle").
c_include_dir("c", 0, "sys2", "system").
c_tu(p, "c") :- c_file(p).
.decl res(f: symbol, p: symbol)
res(f, p) :- c_resolved(_, f, _, _, p).
.decl unres(f: symbol, n: symbol)
unres(f, n) :- c_unresolved(_, f, _, n).
.decl entered(p: symbol)
entered(p) :- c_pp_included(_, _, _, _, p, _).
.output res, unres, entered
"#;
    let a = "#include \"h.h\"\n#include <x.h>\n#include <none.h>\n";
    let files = [("a.c", a), ("h.h", ""), ("x.h", ""), ("sys1/x.h", "#include_next <x.h>\n"), ("sys2/x.h", "")];
    let [res, unres, entered] = <[Vec<String>; 3]>::try_from(joined(guests_run("cincludes", &files, rule, &["res", "unres", "entered"]))).unwrap();
    assert_eq!(res, ["a.c\th.h", "a.c\tsys1/x.h", "sys1/x.h\tsys2/x.h"]);
    assert_eq!(unres, ["a.c\tnone.h"]);
    let closure: Vec<String> = res.iter().map(|r| r.split('\t').nth(1).unwrap().to_owned()).collect();
    assert_eq!(entered, closure);
}

/// Decision 0327: phase 1 (`sources.rerail`) loads the headers a C run's units include, to the closure's fixed point, as
/// library documents, as Python's imports are: a `.h` file is C++'s by its suffix, so no document of a C run until
/// reached. A program that reads `sources__library` has them as its documents (`struct.rerail`'s way). A header no unit
/// includes is none.
#[test]
fn a_c_runs_included_headers_are_its_documents() {
    let rule = r#"
.language c
.decl sources__library(p: symbol, m: symbol, g: symbol, c: symbol)
.decl docs(p: symbol)
docs(p) :- kind(n, "translation_unit"), docof(n, d), doc(d, p, _).
.decl libs(m: symbol)
libs(m) :- sources__library(_, m, "c", _).
.output docs, libs
"#;
    let files = [("a.c", "#include \"inc/h.h\"\nint a;\n"), ("inc/h.h", "#include \"d.h\"\n"), ("inc/d.h", "int d;\n"), ("unused.h", "int u;\n")];
    let [docs, libs] = <[Vec<String>; 2]>::try_from(joined(guests_run("cheaders", &files, rule, &["docs", "libs"]))).unwrap();
    // A library document is named by its path as the engine has it: on Windows, with `\`.
    let docs: Vec<String> = docs.iter().map(|p| p.replace(std::path::MAIN_SEPARATOR, "/")).collect();
    let mut docs: Vec<&str> = docs.iter().map(|p| p.rsplit(".cheaders/").next().unwrap()).collect();
    docs.sort();
    assert_eq!(docs, ["a.c", "inc/d.h", "inc/h.h"], "parsed, each a translation unit");
    assert_eq!(libs, ["inc/d.h", "inc/h.h"]);
}

/// Decision 0327, `c/compile-commands.rerail`: a `compile_commands.json` entry is a unit under a configuration of its
/// own, its `command` split as a shell splits it (`-DFOO="a b"`), its options joined (`-Iinc`) or apart (`-D BAR`),
/// a relative directory its entry's; a compiler profile gives the configuration its first macros and its own search
/// path. Mode (b) then takes the branch the configuration decides.
#[test]
fn c_configurations_come_from_compile_commands_and_a_profile() {
    let rule = r#"
.language c
#include "c/compile-commands.rerail"
#include "c/preprocess.rerail"
.decl pre(n: symbol, b: symbol, o: number)
pre(n, b, o) :- c_predefine(_, o, n, b).
.decl und(n: symbol)
und(n) :- c_config_undef(_, _, n).
.decl dirs(d: symbol, k: symbol)
dirs(d, k) :- c_include_dir(_, _, d, k).
.decl lang(t: symbol, l: symbol, s: symbol)
lang(t, l, s) :- c_tu(t, c), c_lang(c, l, s).
.decl taken(at: number, t: number)
taken(at, t) :- c_pp_cond_taken(_, _, "a.c", at, t).
.output pre, und, dirs, lang, taken
"#;
    let cc = r#"[{"directory": ".", "command": "cc -std=gnu11 -DFOO=\"a b\" -D BAR -UBAZ -Iinc -isystem /abs -c a.c", "file": "a.c"}]"#;
    let profile = "#define PROF 1\n#define F(a, b) a\ndir system sys2\n";
    let a = "#if defined(PROF) && defined(BAR)\nint ok;\n#endif\n";
    let env = "c.compile_commands = compile_commands.json\nc.profile.c = profile.txt\n";
    let files = [("a.c", a), ("compile_commands.json", cc), ("profile.txt", profile), ("rerail.env", env)];
    let [pre, und, dirs, lang, taken] = <[Vec<String>; 5]>::try_from(joined(guests_run("ccommands", &files, rule, &["pre", "und", "dirs", "lang", "taken"]))).unwrap();
    assert_eq!(pre, ["BAR\t1\t1000003", "F(a, b)\ta\t2", "FOO\ta b\t1000002", "PROF\t1\t1"]);
    assert_eq!(und, ["BAZ"]);
    assert_eq!(dirs, ["./inc\tangle", "/abs\tsystem", "sys2\tsystem"]);
    assert_eq!(lang, ["a.c\tc\tgnu11"]);
    assert_eq!(taken, ["0\t1"]);
}

/// A cone that loads nothing reads the exports its requests are keyed by: `f` (located as `f_at`, each row its
/// document) asks for each document's lines. It was left unbound, and the planner had no statistics for it.
#[test]
fn a_request_keyed_by_an_export_needs_no_load() {
    let rule = r#"
.decl f(p: symbol)
f(p) :- kind(n, "module"), docof(n, d), doc(d, p, _).
.decl lines(p: symbol, i: number)
lines(p, i) :- f(p), file_line(p, i, _), i < 2.
.output lines
"#;
    let [lines] = <[Vec<Vec<String>>; 1]>::try_from(guests_run("exportkeyed", &[("a.py", "x = 1\ny = 2\n")], rule, &["lines"])).unwrap();
    assert_eq!(lines, [shown_row(&["a.py", "1"])]);
}

/// A program that adds to an extensible library relation (`cmp_same_ask`, decision 0263) asks the library for its
/// cone (decision 0089 §3): the library rules reading it (`cmp_same`) are copied in. The version the run reads
/// (`lower_in_version`) is the tree's (`lower_in`).
// The check is a debug build's.
#[cfg(debug_assertions)]
#[test]
fn a_cone_copied_onto_the_version_is_the_trees() {
    let src = "#include \"python/compare.rerail\"\n.decl hit(a: node)\ncmp_same_ask(a, a) :- kind(a, \"call\").\nhit(a) :- cmp_same(a, a).\n.output hit\n";
    let files = super::read_text(src).unwrap().files().expect("a modular text");
    let library = super::lower_library(&files, None).unwrap();
    let scoped = library.scoped(&files);
    // A debug build copies the tree's cone beside the version's and checks the two images agree (`lowered_in_version`).
    let v = super::read_text(src).unwrap().lower_in_version(src, &library, &scoped, None, None).unwrap().expect("a modular text");
    let heads: Vec<&str> = v.rules().map(|r| v.rel_name(r.head().rel())).collect();
    assert!(heads.contains(&"cmp_same"), "the cone's rules are copied: {heads:?}");
    // The attributes on request are made again over the cone: its readers ask (decision 0174), and the library's own
    // requests are not copied.
    assert!(heads.contains(&"rr_value_req") && heads.contains(&"value"), "{heads:?}");
    assert!(!heads.contains(&"dunder_all"), "a library reader of `value` outside the cone is not copied: {heads:?}");
}

/// A modular text's program is made a version before `whole()`'s tail, which runs on it (decision 0281's path, step
/// 7b): a context in a negation's cycle (`a_context_in_a_negations_cycle_is_widened`) is widened on the version, and
/// a debug build checks the image against the tree's tail (`link_seeded_version`).
// The check is a debug build's.
#[cfg(debug_assertions)]
#[test]
fn a_context_in_a_cycle_is_widened_on_the_version() {
    let src = concat!(
        "#include \"python/compare.rerail\"\n",
        ".decl e(n: node, x: symbol, s: number)\n.decl seed(n: node)\n.input e, seed\n",
        ".decl ask(n: node)\n.decl cb(n: node)\n.output cb\n",
        "ask(n) :- seed(n).\n",
        "ask(m) :- cb(n), e(n, x, s), e(m, x, s).\n",
        "cb(n) :- ask(n), count : { e(m, \"x\", _), m > n } = 0.\n",
    );
    let files = super::read_text(src).unwrap().files().expect("a modular text");
    let library = super::lower_library(&files, None).unwrap();
    let scoped = library.scoped(&files);
    let v = super::read_text(src).unwrap().lower_in_version(src, &library, &scoped, None, None).unwrap().expect("a modular text");
    let ctx: Vec<String> = v.rules().filter(|r| is_helper(v.rel_name(r.head().rel()), "ctx")).map(|r| v.show_rule(r)).collect();
    assert!(!ctx.is_empty() && ctx.iter().all(|r| !r.contains("ask(")), "{ctx:?}");
}

/// `whole()`'s sets run on a modular text's version (decision 0281's path, step 7b): a literal twice, a rule twice, a
/// rule that is its own body and a fact twice are each once, or gone, as the tree's `lower::sets` makes them; a debug
/// build checks the image against the tree's (`link_seeded_version`).
#[test]
fn the_lowerings_sets_are_made_on_the_version() {
    let src = concat!(
        "#include \"python/compare.rerail\"\n",
        ".decl a(x: number)\n.decl r(x: number)\n.output r\n",
        "a(1). a(1). a(2).\n",
        "r(x) :- a(x), a(x).\n",
        "r(x) :- a(x), a(x).\n",
        "r(x) :- r(x).\n",
    );
    let files = super::read_text(src).unwrap().files().expect("a modular text");
    let library = super::lower_library(&files, None).unwrap();
    let scoped = library.scoped(&files);
    let v = super::read_text(src).unwrap().lower_in_version(src, &library, &scoped, None, None).unwrap().expect("a modular text");
    let r: Vec<String> = v.rules().filter(|x| v.rel_name(x.head().rel()) == "r").map(|x| v.show_rule(x)).collect();
    assert_eq!(r.len(), 1, "one rule of r, its literal once: {r:?}");
    assert_eq!(r[0].matches("a(").count(), 1, "{r:?}");
    assert_eq!(v.fact_rows("a").len(), 2, "a's facts once each: {:?}", v.fact_rows("a"));
}

/// `unit` after aggregate demand runs on a modular text's version (decision 0281's path, step 7b): `count : { b(_) }
/// = 0` made an existence test reads no relation, so its rule reads `rr_unit(0)`, as the tree's `lower::unit`
/// makes it; a debug build checks the image against the tree's (`link_seeded_version`).
#[test]
fn a_rule_left_reading_no_relation_reads_the_unit_on_the_version() {
    let src = concat!(
        "#include \"python/compare.rerail\"\n",
        ".decl b(x: number)\n.input b\n.decl r(x: number)\n.output r\n",
        "r(1) :- count : { b(_) } = 0.\n",
    );
    let files = super::read_text(src).unwrap().files().expect("a modular text");
    let library = super::lower_library(&files, None).unwrap();
    let scoped = library.scoped(&files);
    let v = super::read_text(src).unwrap().lower_in_version(src, &library, &scoped, None, None).unwrap().expect("a modular text");
    let r: Vec<String> = v.rules().filter(|x| v.rel_name(x.head().rel()) == "r").map(|x| v.show_rule(x)).collect();
    assert!(r.iter().all(|x| x.contains("rr_unit(0)")), "{r:?}");
}

/// Aggregate demand runs on a modular text's version (decision 0281's path, step 7b): the aggregate's helper is
/// restricted to the keys its consumer asks (`rr_demand_agg_…`, read first by the helper's rules), and a count of
/// zero becomes an anti-join, as the tree's `aggregate_demand::optimize` makes them; a debug build checks the image
/// against the tree's (`link_seeded_version`).
#[test]
fn aggregate_demand_is_placed_on_the_version() {
    let src = concat!(
        "#include \"python/compare.rerail\"\n",
        ".decl a(x: number)\n.decl b(x: number, y: number)\n.input a, b\n.decl r(x: number)\n.decl s(x: number)\n.output r, s\n",
        "r(x) :- a(x), n = count : { b(x, y), y > x }, n > 1.\n",
        "s(x) :- a(x), count : { b(x, y), y > x } = 0.\n",
    );
    let files = super::read_text(src).unwrap().files().expect("a modular text");
    let library = super::lower_library(&files, None).unwrap();
    let scoped = library.scoped(&files);
    let v = super::read_text(src).unwrap().lower_in_version(src, &library, &scoped, None, None).unwrap().expect("a modular text");
    let rules: Vec<String> = v.rules().map(|x| v.show_rule(x)).collect();
    assert!(rules.iter().any(|x| x.starts_with("rr_demand_agg_")), "a demand is placed: {rules:#?}");
    let s: Vec<&String> = rules.iter().filter(|x| x.starts_with("s(")).collect();
    assert!(s.iter().all(|x| x.contains("!") && !x.contains("count")), "a count of zero is an anti-join: {s:?}");
}

/// The lowering tail's node rewrites run on a modular text's version (decision 0281's path, step 7b): two nodes compared
/// by their preorder keys (`node_order`), a `max` over nodes taken through a helper named by its content (`rr_ordered_<digest>`), as
/// `lowering_tail::OrderNodes` makes them, and a tree axis read declared an input (`lowering_tail::DeclareAxes`); a debug build checks the
/// image against the tree's (`link_seeded_version`).
#[test]
fn finishs_rewrites_run_on_the_version() {
    let src = concat!(
        "#include \"python/compare.rerail\"\n",
        ".decl xs(n: node)\n.decl before(a: node, b: node)\n.decl last(n: node)\n.decl owner(n: node, a: node)\n",
        ".output before, last, owner\n",
        "xs(n) :- kind(n, \"identifier\").\n",
        "before(a, b) :- xs(a), xs(b), a < b.\n",
        "last(m) :- m = max k : { xs(k) }.\n",
        "owner(n, a) :- xs(n), inside(n, a, \"module\").\n",
    );
    assert!(crate::code_rule_dsl::logic::front::orders_nodes(), "node ids per definition");
    let files = super::read_text(src).unwrap().files().expect("a modular text");
    let library = super::lower_library(&files, None).unwrap();
    let scoped = library.scoped(&files);
    let v = super::read_text(src).unwrap().lower_in_version(src, &library, &scoped, None, None).unwrap().expect("a modular text");
    let rules: Vec<String> = v.rules().map(|x| v.show_rule(x)).collect();
    let before = rules.iter().find(|x| x.starts_with("before(")).expect("before's rule");
    assert!(before.contains("node_order("), "{before}");
    assert!(rules.iter().any(|x| x.starts_with("rr_ordered_")), "the max's helper: {rules:#?}");
    let owner = rules.iter().find(|x| x.starts_with("owner(")).expect("owner's rule");
    let axis = owner.split(|c: char| !(c.is_alphanumeric() || c == '_')).find(|w| w.starts_with("rr_inside_")).unwrap_or_else(|| panic!("an axis read: {owner}"));
    assert!(v.input_names().iter().any(|i| i == axis) && v.declares(axis), "the axis {axis} an input: {:?}", v.input_names());
}

/// `whole()`'s last passes before `front::finish_checks_on` run on a modular text's version (decision 0281's path, step 7b): a
/// nullary relation is a set of the unit (a column, a `1` in its atoms), the `.scope` settings set the globs, and a
/// reader of `value` asks for it, as the tree's `lower::whole_back` makes them; a debug build checks the image against the tree's (`link_seeded_version`).
#[test]
fn nullary_relations_and_the_scope_are_made_on_the_version() {
    let src = concat!(
        "#include \"python/compare.rerail\"\n",
        ".scope include \"impl/**/*.py\"\n",
        ".decl flag()\n.decl r(n: node)\n.decl q(n: node)\n.output r, q\n",
        "flag() :- kind(_, \"module\").\n",
        "r(n) :- flag(), kind(n, \"call\").\n",
        "q(n) :- kind(n, \"string\"), value(n, _).\n",
    );
    let files = super::read_text(src).unwrap().files().expect("a modular text");
    let library = super::lower_library(&files, None).unwrap();
    let scoped = library.scoped(&files);
    let v = super::read_text(src).unwrap().lower_in_version(src, &library, &scoped, None, None).unwrap().expect("a modular text");
    let rules: Vec<String> = v.rules().map(|x| v.show_rule(x)).collect();
    assert!(rules.iter().any(|x| x.starts_with("flag(1)")) && rules.iter().any(|x| x.starts_with("r(") && x.contains("flag(1)")), "{rules:#?}");
    assert!(ProgramFields::nullary(&*v).contains("flag") && v.columns_of("flag").is_some_and(|c| c.len() == 1), "{:?}", v.columns_of("flag"));
    let scopes = ProgramFields::doc_scopes(&*v);
    assert!(scopes.as_ref().is_some_and(|s| s.iter().any(|(inc, _)| inc.iter().any(|g| g.contains("impl")))), "{scopes:?}");
    // `value` is answered on request (decision 0174): its reader asks.
    assert!(rules.iter().any(|x| x.starts_with("rr_value_req")), "{rules:#?}");
}

/// The kind sets are made on a modular text's version (decision 0281's path, step 7b): a set of kinds given as facts
/// is a kind set of the program, and a set rules derive is read through helper rules over `child`
/// (`rr_anc_<set>`, `rr_near_<set>`, `rr_in_<set>`), as the tree's `lower::kind_sets` makes them; a debug build
/// checks the image against the tree's (`link_seeded_version`).
#[test]
fn kind_sets_are_made_on_the_version() {
    let src = concat!(
        "#include \"python/compare.rerail\"\n",
        ".decl calls(k: symbol)\ncalls(\"call\").\n",
        ".decl code(k: symbol)\ncode(k) :- kind(_, k), k != \"comment\".\n",
        ".decl r(n: node, a: node)\n.decl s(n: node, a: node)\n.output r, s\n",
        "r(n, a) :- kind(n, \"name\"), nearest(n, a, calls).\n",
        "s(n, a) :- kind(n, \"name\"), inside(n, a, code).\n",
    );
    let files = super::read_text(src).unwrap().files().expect("a modular text");
    let library = super::lower_library(&files, None).unwrap();
    let scoped = library.scoped(&files);
    let v = super::read_text(src).unwrap().lower_in_version(src, &library, &scoped, None, None).unwrap().expect("a modular text");
    let kindsets = ProgramFields::kindsets(&*v);
    assert!(kindsets.get("calls").is_some_and(|k| k == &["call"]), "{kindsets:?}");
    let heads: BTreeSet<&str> = v.rules().map(|x| v.rel_name(x.head().rel())).collect();
    assert!(["rr_anc_code", "rr_near_code", "rr_in_code"].iter().all(|h| heads.contains(h)), "{heads:?}");
}

/// The grammar names are checked on a modular text's version (decision 0281's path, step 7b): a grammar the registry
/// does not have is refused there too; and the built-ins lowering writes out (`nearest`, `earlier`, …) are no relations
/// of the program (`BuiltinsOut`'s own test is `builtins_out_on_a_version_is_the_trees`: the unseeding hides it here), as the tree's
/// `lower::grammar_names` and `lower::builtins_out` make them; a debug build checks the image against the tree's.
#[test]
fn grammar_names_and_builtins_are_checked_on_the_version() {
    let lower = |body: &str| {
        let src = format!("#include \"python/compare.rerail\"\n.decl hit(s: node)\n.output hit\n{body}\n");
        let files = super::read_text(&src).unwrap().files().expect("a modular text");
        let library = super::lower_library(&files, None).unwrap();
        let scoped = library.scoped(&files);
        super::read_text(&src).unwrap().lower_in_version(&src, &library, &scoped, None, None).map(|v| v.expect("a modular text"))
    };
    let refused = lower("hit(s) :- kind(s, \"string\"), parsed(s, \"postgres\", _).").map(|_| ()).unwrap_err().to_string();
    assert!(refused.contains("no grammar \"postgres\""), "{refused}");
    let v = lower("hit(s) :- kind(s, \"name\"), within(s, a), kind(a, \"call\").").unwrap();
    assert!(!["nearest", "earlier", "split_dot", "unset"].iter().any(|b| v.declares(b)), "{:?}", v.declared_names());
}

/// The span patterns are read off a modular text's version (decision 0281's path, step 7b): a `match` over a line
/// (`src_line`) and one over a comment give the program its rx spans, as the tree's
/// `lower::span_patterns` finds them; a debug build checks the image against the tree's.
#[test]
fn span_patterns_are_found_on_the_version() {
    let src = concat!(
        "#include \"python/compare.rerail\"\n",
        ".decl r(l: node)\n.decl c(n: node)\n.output r, c\n",
        "r(l) :- src_line(_, _, l), match(\"TODO.*\", l).\n",
        "c(n) :- kind(n, \"comment\"), match(\"#.*fixme\", n).\n",
    );
    let files = super::read_text(src).unwrap().files().expect("a modular text");
    let library = super::lower_library(&files, None).unwrap();
    let scoped = library.scoped(&files);
    let v = super::read_text(src).unwrap().lower_in_version(src, &library, &scoped, None, None).unwrap().expect("a modular text");
    use crate::code_rule_dsl::logic::front::SpanIn;
    let rx_spans = ProgramFields::rx_spans(&*v);
    assert!(rx_spans.iter().any(|(w, p)| *w == SpanIn::Line && p.contains("TODO")), "{rx_spans:?}");
    assert!(rx_spans.iter().any(|(w, p)| *w == SpanIn::Comment && p.contains("fixme")), "{rx_spans:?}");
}

/// core.rerail's declarations and `node`'s projections are made on a modular text's version (decision 0281's path, step
/// 7b): a rule reading `kind` reads it through the projection `kind(n, k) :- node(_, n, k, _, _)`, the program's
/// first rule, as the tree's `lower::core_and_projections` makes it; a debug build checks the image against the tree's.
#[test]
fn core_declarations_and_projections_are_made_on_the_version() {
    let src = concat!("#include \"python/compare.rerail\"\n", ".decl r(n: node)\n.output r\n", "r(n) :- kind(n, \"call\").\n");
    let files = super::read_text(src).unwrap().files().expect("a modular text");
    let library = super::lower_library(&files, None).unwrap();
    let scoped = library.scoped(&files);
    let v = super::read_text(src).unwrap().lower_in_version(src, &library, &scoped, None, None).unwrap().expect("a modular text");
    let rules: Vec<String> = v.rules().map(|x| v.show_rule(x)).collect();
    assert!(rules.first().is_some_and(|r| r.starts_with("kind($n, $k) :- node(")), "{rules:#?}");
    assert!(!v.input_names().iter().any(|i| i == "kind"), "{:?}", v.input_names());
}

/// The findings of a modular text's `.violation`s are written on its version (decision 0281's path, step 7b): the
/// `violation` rule at the anchor's line and column, the diagnostic with the message's placeholders (a fresh name per
/// value, the lowerer's count handed on), `.on syntax_error`'s rule, and the text's own `violation` rule of five columns
/// widened, as the tree's `lower::findings` writes them;
/// a debug build checks the image against the tree's.
#[test]
fn findings_are_written_on_the_version() {
    let src = concat!(
        "#include \"python/compare.rerail\"\n",
        ".on syntax_error report BAD000 at line 1\n",
        ".violation call_at(at: node) code \"C001\" \"call {at}\"\n",
        "call_at(n) :- kind(n, \"call\").\n",
        "violation(\"p\", \"own\", \"X\", p, 1) :- doc(_, p, _).\n",
    );
    let files = super::read_text(src).unwrap().files().expect("a modular text");
    let library = super::lower_library(&files, None).unwrap();
    let scoped = library.scoped(&files);
    let v = super::read_text(src).unwrap().lower_in_version(src, &library, &scoped, None, None).unwrap().expect("a modular text");
    let rules: Vec<String> = v.rules().map(|x| v.show_rule(x)).collect();
    assert!(rules.iter().any(|r| r.starts_with("violation(") && r.contains("call_at(") && r.contains("\"C001\"")), "{rules:#?}");
    assert!(rules.iter().any(|r| r.starts_with("violation(") && r.contains("syntax_error(")), "{rules:#?}");
    assert!(rules.iter().any(|r| r.contains("$rr_message")), "{rules:#?}");
    // The text's own `violation` rule, in the older five columns, read at column 0.
    assert!(rules.iter().any(|r| r.starts_with("violation(\"p\", \"own\", \"X\", $p, 1, 0)")), "{rules:#?}");
    assert!(v.output_names().iter().any(|o| o == "violation") && v.columns_of("violation").is_some_and(|c| c.len() == 6), "{:?}", v.output_names());
}

/// The three-valued relations are written on a modular text's version (decision 0281's path, step 7b): `p` declared
/// `must, may` has a must side of its rules but the `may` one, what reads `p` is three-valued too, and a negated atom
/// reads the other side, as the tree's `lower::three_valued` writes them; a debug build checks the image against the
/// tree's.
#[test]
fn three_valued_relations_are_written_on_the_version() {
    let src = concat!(
        "#include \"python/compare.rerail\"\n",
        ".decl base(x: symbol)\nbase(\"a\"). base(\"b\").\n.decl known(x: symbol)\nknown(\"a\").\n",
        ".decl p(x: symbol) must, may\nmust p(x) :- known(x).\nmay p(x) :- base(x), !known(x).\n",
        ".decl q(x: symbol)\nq(x) :- base(x), !p(x).\n.output q, rr_must_q\n",
    );
    let files = super::read_text(src).unwrap().files().expect("a modular text");
    let library = super::lower_library(&files, None).unwrap();
    let scoped = library.scoped(&files);
    let v = super::read_text(src).unwrap().lower_in_version(src, &library, &scoped, None, None).unwrap().expect("a modular text");
    let rules: Vec<String> = v.rules().map(|x| v.show_rule(x)).collect();
    let has = |r: &str| rules.iter().any(|x| x == r);
    assert!(has("rr_must_p($x) :- known($x)") && has("p($x) :- known($x)") && has("p($x) :- base($x), !known($x)"), "{rules:#?}");
    assert!(!rules.iter().any(|x| x.starts_with("rr_must_p($x) :- base(")), "{rules:#?}");
    assert!(has("rr_must_q($x) :- base($x), !p($x)") && has("q($x) :- base($x), !rr_must_p($x)"), "{rules:#?}");
    assert!(v.declares("rr_must_p") && v.declares("rr_must_q"), "{:?}", v.declared_names());
}

/// A modular text's module is linked onto its scope's version and checked declared there (decision 0281's path, step
/// 7b): a rule reading a relation no one declares is refused, and so is one reading a library relation with other
/// columns than it declares, as `lowering_tail::Declared` refuses them; a rule reading the library's is linked.
#[test]
fn a_module_is_linked_and_checked_declared_on_the_version() {
    let lower = |body: &str| {
        let src = format!("#include \"python/compare.rerail\"\n.decl hit(n: node)\n.output hit\n{body}\n");
        let files = super::read_text(&src).unwrap().files().expect("a modular text");
        let library = super::lower_library(&files, None).unwrap();
        let scoped = library.scoped(&files);
        super::read_text(&src).unwrap().lower_in_version(&src, &library, &scoped, None, None).map(|v| v.expect("a modular text")).map_err(|e| e.to_string())
    };
    let e = lower("hit(n) :- kind(n, \"call\"), nowhere(n).").map(|_| ()).unwrap_err();
    assert!(e.contains("relation nowhere is read by the rule for hit and not declared"), "{e}");
    let e = lower("hit(n) :- kind(n, \"call\"), child(n).").map(|_| ()).unwrap_err();
    assert!(e.contains("relation child is declared with 3 columns and read with 1"), "{e}");
    let v = lower("hit(n) :- kind(n, \"call\"), child(_, n, _).").unwrap();
    assert!(v.rules().any(|r| v.rel_name(r.head().rel()) == "hit"), "{}", v.show());
}

/// A text's rules are written as a version in its walk (decision 0377 step 3), its header with them.
#[test]
fn a_texts_rules_are_written_as_a_version_in_its_walk() {
    // Two aggregates of two literals under one head make two helpers of one stem and column types (`helper_once`),
    // which the link keys by their rules, read from the version the walk wrote.
    let src = "#include \"python/compare.rerail\"\n.decl hit(n: node)\n.output hit\n.decl near(n: node, m: node)\nnear(n, m) :- child(_, n, _), child(_, m, _), kind(n, \"call\").\nhit(n) :- kind(n, \"call\"), child(_, n, _), !near(n, n).\n.decl sizes(n: node, a: number, b: number)\n.output sizes\nsizes(n, a, b) :- kind(n, \"call\"), a = count : { child(n, c, _), kind(c, \"name\") }, b = count : { child(n, c, _), kind(c, \"call\") }.\n";
    let files = super::read_text(src).unwrap().files().expect("a modular text");
    let library = super::lower_library(&files, None).unwrap();
    let scoped = library.scoped(&files);
    let v = super::read_text(src).unwrap().own_written(&scoped).unwrap();
    assert!(v.rules().count() >= 2, "{}", v.show());
    // Each aggregate's helper, numbered by the link after the scope's: written in the walk, and read by `sizes`.
    let helpers: BTreeSet<&str> = v.rules().map(|r| v.rel_name(r.head().rel())).filter(|h| is_helper(h, "agg")).collect();
    assert_eq!(helpers.len(), 2, "{}", v.show());
}

/// A version of `text` with request record `asked` (decision 0331).
fn with_asked(text: &str, asked: &[(&str, &[&[&str]])]) -> Arc<crate::code_rule_dsl::logic::program_image::Version> {
    let read = parse_version(text).expect("parses");
    let mut b = crate::code_rule_dsl::logic::program_image::Builder::on(&read);
    let asked = asked.iter().map(|(rel, by)| ((*rel).to_owned(), by.iter().map(|r| r.iter().map(|s| (*s).to_owned()).collect()).collect())).collect();
    b.fields().set_asked(&asked);
    b.finish().unwrap().0
}

/// The request record follows the pass that drops repeated rules (`DropRepeated`): the rule kept serves both its
/// own readers and its repeat's, so `Prune` gates it, and a slice cut from the prepared program too (decision 0331).
/// The reader keeps no repeat; the merge makes one, `q`'s two rules reading two relations it makes one.
#[test]
fn the_request_record_follows_repeated_rules() {
    let text = ".decl q(n: number)\n.decl a(n: number)\n.decl b(n: number)\n.decl k(n: number)\n.input k\n.output a, b\n\
        .decl r1(n: number)\n.decl r2(n: number)\nr1(n) :- k(n), n < 3.\nr2(n) :- k(n), n < 3.\n\
        q(n) :- r1(n).\nq(n) :- r2(n).\na(n) :- q(n), n > 0.\nb(n) :- q(n).\n";
    let v = with_asked(text, &[("q", &[&["a"], &["b"]])]);
    let mut p = crate::code_rule_dsl::logic::program_image::manager::Pipeline::new("test", v);
    let (mut repeats, mut merge) = (crate::code_rule_dsl::logic::plan::corpus::preparation::DropRepeated::default(), crate::code_rule_dsl::logic::plan::corpus::preparation::MergeIdentical::default());
    p.program(&mut repeats).program(&mut merge).program(&mut repeats);
    let (v, _) = p.finish().unwrap();
    assert!(merge.merged > 0 && repeats.0 == 1, "{} merged, {} repeats:\n{}", merge.merged, repeats.0, v.show());
    let by = |r: &[&str]| r.iter().map(|s| (*s).to_owned()).collect::<BTreeSet<String>>();
    assert_eq!(v.asked()["q"], vec![by(&["a", "b"])], "{}", v.show());
}

/// A request gated by its readers (`Prune`) is kept while a reader is needed, and while a reader is gone from the
/// program (merged, renamed or inlined since the requests were made: it cannot be asked); a reader `Prune` drops
/// leaves the record, beside one it keeps, so a later gate cannot take it for a gone one (decision 0331).
#[test]
fn a_request_is_kept_for_a_gone_reader() {
    let text = ".decl q(n: number)\n.decl a(n: number)\n.decl b(n: number)\n.decl k(n: number)\n.input k\n.output b\n\
        q(n) :- k(n), n < 3.\nq(n) :- k(n), n > 5.\na(n) :- q(n).\nb(n) :- q(n).\n";
    let by = |r: &[&str]| r.iter().map(|s| (*s).to_owned()).collect::<BTreeSet<String>>();
    let pruned = |v| {
        let mut p = crate::code_rule_dsl::logic::program_image::manager::Pipeline::new("test", v);
        p.program(&mut crate::code_rule_dsl::logic::plan::corpus::preparation::Prune::default());
        p.finish().unwrap().0
    };
    // The first request serves `a`, which `b` does not read; the second a reader no longer in the program.
    let v = pruned(with_asked(text, &[("q", &[&["a"], &["gone"]])]));
    let q: Vec<String> = v.rules().filter(|r| v.rel_name(r.head().rel()) == "q").map(|r| v.show_rule(r)).collect();
    assert_eq!(q.len(), 1, "only the gone reader's request is kept: {q:?}");
    assert!(q[0].contains("> 5"), "{q:?}");
    let v = pruned(with_asked(text, &[("q", &[&["a", "b"], &["b"]])]));
    assert!(v.rules().all(|r| v.rel_name(r.head().rel()) != "a"), "a is not needed");
    assert_eq!(v.asked()["q"], vec![by(&["b"]), by(&["b"])]);
}

/// Decision 0361 step 1: a `.layout` line states facts about a declared relation's rows, its columns by name; several
/// lines add up, each fact once; the facts round-trip through their printed form.
#[test]
fn a_layout_line_states_a_relations_facts_by_its_columns() {
    use crate::code_rule_dsl::logic::front::{Algo, Fan, LayoutFact as F, LayoutSize};
    let text = ".decl e(p: number, c: number, i: number)\n.input e\n\
                .layout e key(c) key(p, i) fanout(p -> c, few) local\n\
                .layout e order(i dense in p) tree(p -> c) size(<= 2 per p) key(c)\n\
                .layout e fanout(c -> p, ~3) by running(i in p)\n\
                .decl o(x: number)\n.output o\no(x) :- e(x, _, _).\n";
    let p = parse(text).unwrap();
    let want = vec![
        F::Key(vec![1]),
        F::Key(vec![0, 2]),
        F::Fanout { from: vec![0], to: vec![1], fan: Fan::Few },
        F::Local,
        F::Order { col: 2, within: vec![0], dense: true },
        F::Tree { from: 0, to: 1 },
        F::Size(LayoutSize::Per { most: 2, col: 0 }),
        F::Fanout { from: vec![1], to: vec![0], fan: Fan::About(3) },
        F::By(Algo::Running { col: 2, within: vec![0] }),
    ];
    let layouts = ProgramFields::layouts(&*p);
    assert_eq!(layouts.get("e"), Some(&want), "{layouts:?}");
    assert_eq!(Fan::About(3).class(), Fan::Few);
    for f in &want {
        assert_eq!(&F::of_placed(&f.placed()).unwrap(), f, "{}", f.placed());
    }
    let names = ["p", "c", "i"];
    assert_eq!(want[2].show(&|c| names[c].to_owned()), "fanout(p -> c, few)");
}

/// A `.layout` line naming a relation no declaration has, a column the relation lacks, or no fact, is a syntax error
/// that names its line (decision 0361, item 1).
#[test]
fn a_layout_line_names_a_declared_relations_columns() {
    let at = |layout: &str| match parse(&format!(".decl e(p: number, c: number)\n.input e\n{layout}\n.decl o(x: number)\n.output o\no(x) :- e(x, _).\n")) {
        Err(Error::Syntax(m)) => m,
        other => panic!("{layout}: {other:?}"),
    };
    assert!(at(".layout f key(p)").contains("line 3") && at(".layout f key(p)").contains("`f` is not declared"), "{}", at(".layout f key(p)"));
    assert!(at(".layout e key(q)").contains("`e` has no column `q`"), "{}", at(".layout e key(q)"));
    assert!(at(".layout e").contains("no fact"), "{}", at(".layout e"));
    assert!(at(".layout e fanout(p -> c, some)").contains("one, few, many or ~N"), "{}", at(".layout e fanout(p -> c, some)"));
    assert!(at(".layout e sorted(p)").contains("`sorted`"), "{}", at(".layout e sorted(p)"));
}

/// The layouts go where the program goes (decision 0361 step 1): into its image's facts and back, into the version's
/// about rows (`About::layouts`), and through `combine`, which renames relations. `core.rerail`'s base relations declare
/// theirs (step 2).
#[test]
fn layouts_are_carried_with_the_program() {
    use crate::code_rule_dsl::logic::front::LayoutFact as F;
    let policy = |name: &str| format!(".policy \"t:{name}\" revision 1\n.language python\n#include \"python/ast.rerail\"\n.violation Fn(at: node) \"a call\"\nFn(c) :- call_name(c, _).\n");
    let p = parse(&policy("a")).unwrap();
    let layouts = ProgramFields::layouts(&*p);
    let child = layouts.get("child").expect("core.rerail declares child's layout");
    assert!(child.contains(&F::Key(vec![1])) && child.contains(&F::Order { col: 2, within: vec![0], dense: true }), "{child:?}");
    assert!(layouts.get("node").is_some_and(|f| f.contains(&F::Key(vec![1]))), "{:?}", layouts.get("node"));
    // The image's facts, read back from its bytes.
    let v = Version::open(p.fold().unwrap().base_bytes().unwrap()).unwrap();
    assert_eq!(ProgramFields::layouts(&*v), layouts);
    let about = About::of(&v);
    let rel = v.rel("child").unwrap();
    assert_eq!(about.layouts.iter().find(|(r, _)| *r == rel).map(|(_, f)| f), Some(child));
    // Combined: each relation keeps its facts under the name it has there.
    let combined = crate::code_rule_dsl::logic::plan::corpus::combine(&[&policy("a"), &policy("b")]).unwrap();
    let layouts = combined.layouts();
    assert_eq!(layouts.get("child"), Some(child), "{:?}", layouts.keys().collect::<Vec<_>>());
}

/// A negated conjunction whose parts bind the rule's variables apart (`!(lit(lo, _), lit(hi, _))`) is bound by the
/// rule's context: alone its helper is every pair of strings in the corpus (eslint-yoda's `yoda_in_order`). One whose
/// literals tie them (`!(link(lo, hi), lit(hi, _))`) needs none. PR #508's 3fe6f8227 (t47).
#[test]
fn a_negated_cross_product_is_bound_by_the_rules_context() {
    let p = parse(concat!(
        ".decl want(a: node, b: node)\n.decl lit(e: node, v: symbol)\n.decl link(a: node, b: node)\n.input want, lit, link\n",
        ".decl apart(a: node, b: node)\n.decl tied(a: node, b: node)\n.output apart, tied\n",
        "apart(lo, hi) :- want(lo, hi), !(lit(lo, _), lit(hi, _)).\n",
        "tied(lo, hi) :- want(lo, hi), !(link(lo, hi), lit(hi, _)).\n",
    ))
    .unwrap();
    analyses::strata(&p, &About::of(&p)).unwrap_or_else(|e| panic!("{e}"));
    let shown: Vec<String> = p.rules().map(|r| p.show_rule(r)).collect();
    let helper = |head: &str| -> String {
        let rule = p.rules().find(|r| p.rel_name(r.head().rel()) == head).unwrap_or_else(|| panic!("{shown:#?}"));
        let not = rule.body().filter_map(|l| l.atom()).find(|a| a.neg() && is_helper(p.rel_name(a.rel()), "not")).unwrap_or_else(|| panic!("{shown:#?}"));
        p.rules().filter(|r| r.head().rel() == not.rel()).map(|r| p.show_rule(r)).collect::<Vec<_>>().join("\n")
    };
    let (apart, tied) = (helper("apart"), helper("tied"));
    let bound = |helper: &str| names_helper(helper, "ctx") || helper.contains("want(");
    assert!(bound(&apart) && apart.contains("lit($lo") && apart.contains("lit($hi"), "{apart}\n{shown:#?}");
    assert!(!bound(&tied), "{tied}");
}

/// The modules `demand_split_child` reads, and the builtin references each holds.
const SPLIT_MODULES: usize = 32;
const SPLIT_REFERENCES: usize = 4;

/// python/calls over `SPLIT_MODULES` modules of `SPLIT_REFERENCES` builtin references each: no demand split's rows grow
/// as the references times the modules (`plan::demand::over::disconnected`). `builtin_ref`'s split with `docof` cut
/// was every reference against every module (8,801 rows of work in its `pre_<i>` here, over the 4,096 of references
/// times modules; 129.7M rows on edx-platform's 4,459 files). The profile is the process's, so the run is a child
/// process of its own (`demand_split_child`), its rows read from its `heads.tsv`.
#[test]
fn a_demand_split_stays_linear_in_the_modules() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("src");
    std::fs::create_dir_all(&root).unwrap();
    // Plain nodes make the program's rarer kinds anchors (fewer than 1/32 of the nodes), so `docof` is a candidate.
    let filler = (1..=300).map(|i| i.to_string()).collect::<Vec<_>>().join(", ");
    let source = format!("x = len([1])\nprint(x)\ny = len(\"ab\")\nprint(y)\nDATA = [{filler}]\n");
    assert_eq!(source.matches("len(").count() + source.matches("print(").count(), SPLIT_REFERENCES);
    for m in 0..SPLIT_MODULES {
        std::fs::write(root.join(format!("m{m}.py")), &source).unwrap();
    }
    let rule = directory.path().join("calls.rerail");
    std::fs::write(&rule, ".policy \"split-linear\" revision 1\n.language python\n.scope include \"**/*.py\"\n#include \"python/calls.rerail\"\n.violation P(at: node) \"c\"\nP(c) :- call_target(c, f), kind(f, \"nothing_has_this_kind\").\n").unwrap();
    let profile = directory.path().join("profile");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap());
    child
        .args(["--exact", "code_rule_dsl::logic::rr::tests::demand_split_child", "--ignored", "--nocapture"])
        .env("RERAIL_PROFILE", &profile)
        .env("RERAIL_TAPE_CACHE", "off")
        .env("DEMAND_SPLIT_RULE", &rule)
        .env("DEMAND_SPLIT_ROOT", &root)
        .env("DEMAND_SPLIT_OUT", directory.path().join("out"));
    let status = crate::code_rule_dsl::logic::plan::corpus::switches::started(|| child.status()).unwrap();
    assert!(status.success());
    // part, head, cpu_s, work (rows into the head's steps), plans
    let heads = std::fs::read_to_string(profile.join("heads.tsv")).unwrap();
    let rows: Vec<(&str, &str, u64)> = heads.lines().skip(1).map(|l| {
        let c: Vec<&str> = l.split('\t').collect();
        (c[0], c[1], c[3].parse().unwrap())
    }).collect();
    // The typing program runs as the run's phases (decision 0083; the job of its own went in 91f31ae830).
    assert!(rows.iter().any(|(part, _, _)| part.contains("phase types: run")), "the typing phases ran: {heads}");
    let product = (SPLIT_MODULES * SPLIT_MODULES * SPLIT_REFERENCES) as u64;
    for (part, head, work) in &rows {
        if head.starts_with("pre_") || head.starts_with("demand_") {
            assert!(*work < product / 4, "{head} ({part}): {work} rows, against {product} for every reference times every module");
        }
    }
}

#[test]
#[ignore = "child process of a_demand_split_stays_linear_in_the_modules and a_run_on_two_threads_ends"]
fn demand_split_child() {
    let var = |k: &str| {
        std::path::PathBuf::from(std::env::var_os(k).unwrap_or_else(|| panic!("{k} unset: run by a_demand_split_stays_linear_in_the_modules or a_run_on_two_threads_ends")))
    };
    crate::code_rule_dsl::tools::run_rr(&var("DEMAND_SPLIT_RULE"), &var("DEMAND_SPLIT_ROOT"), &var("DEMAND_SPLIT_OUT"), &serde_json::Value::Null).unwrap();
}

/// Decision 0470 R5: a string at a place the Python language declares (`markupsafe.Markup`'s argument) is an HTML
/// program of its file, at the string: a Python rule reaches it through `parsed(s, "html", p)`, and its nodes stand at
/// the file's lines. A string at no place is no program: a rule that asks for it gets a tape value, at its own text's
/// lines.
#[test]
fn a_string_at_a_declared_place_is_a_program_of_its_file() {
    let py = "from markupsafe import Markup\n\nx = Markup(\n    \"<p><b>a</b></p>\"\n)\ny = \"<i>not</i>\"\n";
    let rule = r#"
.decl tags(t: symbol, l: number)
tags(t, l) :- kind(s, "string"), parsed(s, "html", p), within(e, p), kind(e, "tag_name"), text(e, t), line(e, l).
.output tags
"#;
    let [tags] = <[Vec<Vec<String>>; 1]>::try_from(guests_run("place-program", &[("a.py", py)], rule, &["tags"])).unwrap();
    let tags: BTreeSet<Vec<String>> = tags.into_iter().collect();
    // The rule asks every string: the one at no place is a tape value (decision 0431), at its own text's line.
    assert_eq!(tags, [["b", "4"], ["i", "1"], ["p", "4"]].iter().map(|r| r.iter().map(|v| (*v).to_owned()).collect()).collect(), "{tags:?}");
}

/// Decision 0470 R5, R7: a run reads the files that hold its language's programs only where the program says
/// `.embedded on` (the user: "possible but not happen by default"). With it, an HTML run sees a Python file's Markup
/// program and a JavaScript run an HTML file's script, at their files' lines, and none of the host files' own nodes;
/// without it, neither.
#[test]
fn a_run_reads_the_files_holding_its_programs_only_when_asked() {
    let py = "from markupsafe import Markup\n\nx = Markup(\n    \"<p><b>a</b></p>\"\n)\ny = \"<i>not</i>\"\n";
    let html = "<p>x</p>\n<script>f(1)</script>\n";
    let set = |rows: Vec<Vec<String>>| rows.into_iter().collect::<BTreeSet<Vec<String>>>();
    let want = |rows: &[&[&str]]| rows.iter().map(|r| r.iter().map(|v| (*v).to_owned()).collect()).collect::<BTreeSet<Vec<String>>>();
    let tags = |embedded: &str| format!(".language html\n{embedded}\n.decl tags(t: symbol, p: symbol, l: number)\ntags(t, p, l) :- kind(e, \"tag_name\"), text(e, t), docof(e, d), doc(d, p, _), line(e, l).\n.output tags\n");
    let [on] = <[Vec<Vec<String>>; 1]>::try_from(guests_run("embedded-on-html", &[("a.py", py), ("b.html", html)], &tags(".embedded on"), &["tags"])).unwrap();
    assert_eq!(set(on), want(&[&["b", "a.py", "4"], &["p", "a.py", "4"], &["p", "b.html", "1"], &["script", "b.html", "2"]]), "an HTML run asked to");
    let [off] = <[Vec<Vec<String>>; 1]>::try_from(guests_run("embedded-off-html", &[("a.py", py), ("b.html", html)], &tags(""), &["tags"])).unwrap();
    assert_eq!(set(off), want(&[&["p", "b.html", "1"], &["script", "b.html", "2"]]), "by default: its own files alone");
    let calls = |embedded: &str| format!(".language javascript\n{embedded}\n.decl calls(x: symbol, p: symbol, l: number)\ncalls(x, p, l) :- kind(c, \"call_expression\"), text(c, x), docof(c, d), doc(d, p, _), line(c, l).\n.output calls\n");
    let [on] = <[Vec<Vec<String>>; 1]>::try_from(guests_run("embedded-on-js", &[("b.html", html)], &calls(".embedded on"), &["calls"])).unwrap();
    assert_eq!(set(on), want(&[&["f(1)", "b.html", "2"]]), "a JavaScript run asked to");
    let [off] = <[Vec<Vec<String>>; 1]>::try_from(guests_run("embedded-off-js", &[("b.html", html)], &calls(""), &["calls"])).unwrap();
    assert!(off.is_empty(), "by default: {off:?}");
}

/// Decision 0495 R1: a rule file's JSON objects are JSON programs (`json-object`). A JSON run that says `.embedded on`
/// reads them at the rule file's lines; without it, it reads its own files alone (here none).
#[test]
fn a_rule_files_json_objects_are_json_programs() {
    let rr = "// rules\n.decl a(x: symbol)\na({\"k\": 1,\n  \"m\": 2}).\n";
    let pairs = |embedded: &str| format!(".language json\n{embedded}\n.decl pairs(x: symbol, p: symbol, l: number)\npairs(x, p, l) :- kind(n, \"pair\"), text(n, x), docof(n, d), doc(d, p, _), line(n, l).\n.output pairs\n");
    let [on] = <[Vec<Vec<String>>; 1]>::try_from(guests_run("rr-json-on", &[("rules/r.rerail", rr)], &pairs(".embedded on"), &["pairs"])).unwrap();
    let on: BTreeSet<Vec<String>> = on.into_iter().collect();
    let want: BTreeSet<Vec<String>> = [["\"k\": 1", "rules/r.rerail", "3"], ["\"m\": 2", "rules/r.rerail", "4"]].iter().map(|r| r.iter().map(|v| (*v).to_owned()).collect()).collect();
    assert_eq!(on, want);
    let [off] = <[Vec<Vec<String>>; 1]>::try_from(guests_run("rr-json-off", &[("rules/r.rerail", rr)], &pairs(""), &["pairs"])).unwrap();
    assert!(off.is_empty(), "{off:?}");
}

/// Decision 0495 R2: the `regex` grammar reads the Rust dialect, the engine's own: its tree's kinds, each node's value
/// where it has one, and what the dialect refuses as an `unsupported` node over its bytes.
#[test]
fn the_regex_grammar_reads_the_rust_dialect() {
    use crate::code_rule_dsl::literal::{PNode, regex};
    use crate::code_rule_dsl::value::Value;
    fn show(n: &PNode) -> String {
        let kind = regex::GRAMMAR.kinds()[usize::from(n.kind) - 1];
        let value = match &n.value {
            Some(Value::Str(v)) => format!(":{v}"),
            _ => String::new(),
        };
        let children: Vec<String> = n.children.iter().map(show).collect();
        if children.is_empty() { format!("{kind}{value}") } else { format!("{kind}{value}({})", children.join(" ")) }
    }
    let cases = [
        ("a|b*", "regex(alternation(literal:a repetition:*(literal:b)))"),
        ("(?P<y>\\d{4})", "regex(group(name:y repetition:{4}(escape:\\d)))"),
        ("[^a-z[:digit:]]", "regex(class:^(range(literal:a literal:z) class_name:digit))"),
        ("(?<=a)b", "regex(concatenation(unsupported:look-around(literal:a) literal:b))"),
        ("(?i)x.+?$", "regex(concatenation(group(flags:i) literal:x repetition:+?(dot) anchor:$))"),
        ("\\1\\.", "regex(concatenation(unsupported:back-reference literal:.))"),
        ("a{2,}b{x}", "regex(concatenation(repetition:{2,}(literal:a) literal:b literal:{ literal:x literal:}))"),
        ("(a", "regex(error:a group is not closed)"),
        ("a)", "regex(error:a `)` closes no group)"),
    ];
    for (text, want) in cases {
        assert_eq!(show(&regex::tree(regex::GRAMMAR, text)), want, "{text}");
    }
    // The spans are the text's bytes.
    let root = regex::tree(regex::GRAMMAR, "(?<=a)b");
    assert_eq!((root.children[0].children[0].start, root.children[0].children[0].end), (0, 6));
}

/// Decision 0495 R2, on the repository's own regexes: every `match("…", s)` string of every rule file that the
/// `regex` crate compiles reads with no `error` node, and one it refuses with no tree that hides the refusal.
#[test]
fn every_rule_files_regex_reads_in_the_regex_grammar() {
    use crate::code_rule_dsl::literal::{PNode, regex};
    fn has(n: &PNode, kind: &str) -> bool {
        regex::GRAMMAR.kinds()[usize::from(n.kind) - 1] == kind || n.children.iter().any(|c| has(c, kind))
    }
    let root = rerail_store::source_tree::native().join("..");
    let (mut read, mut bad) = (0usize, Vec::new());
    let mut todo = vec![root.join("docs/rule-syntax"), root.join("packs")];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                todo.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rerail") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            for (at, _) in text.match_indices("match(\"") {
                let body = &text[at + 7..];
                let mut end = 0;
                let b = body.as_bytes();
                while end < b.len() && b[end] != b'"' {
                    end += if b[end] == b'\\' { 2 } else { 1 };
                }
                let Ok(value) = rerail_rr_syntax::syntax::unescape(&body[..end.min(b.len())]) else { continue };
                if ::regex::Regex::new(&value).is_err() {
                    continue;
                }
                read += 1;
                if has(&regex::tree(regex::GRAMMAR, &value), "error") {
                    bad.push(format!("{}: {value}", path.strip_prefix(&root).unwrap().display()));
                }
            }
        }
    }
    assert!(read > 5000, "{read} regexes read");
    assert!(bad.is_empty(), "{} of {read}: {:#?}", bad.len(), &bad[..bad.len().min(20)]);
}

/// Decision 0495 R3: a `match`'s first string is a regex program of its rule file, at the string's bytes through its
/// escapes. A regex run that says `.embedded on` finds each look-around at the rule file's line, its text the bytes
/// it was written with; without it, it reads its own files alone (here none).
#[test]
fn a_match_regex_is_a_regex_program_at_its_bytes() {
    let rr = ".decl a(x: symbol)\n.decl b(x: symbol)\nb(x) :- a(x), match(\"(?<=q)\\.py\", x).\nb(x) :- a(x), match(\"ok\\\"(?!z)\", x), !match(X, x), X = \"y\".\n";
    let found = |embedded: &str| format!(".language regex\n{embedded}\n.decl la(t: symbol, p: symbol, l: number)\nla(t, p, l) :- kind(n, \"unsupported\"), text(n, t), docof(n, d), doc(d, p, _), line(n, l).\n.decl lits(t: symbol)\nlits(t) :- kind(n, \"literal\"), text(n, t), t = \"\\\"\".\n.output la, lits\n");
    let [la, lits] = <[Vec<Vec<String>>; 2]>::try_from(guests_run("rr-regex-on", &[("rules/r.rerail", rr)], &found(".embedded on"), &["la", "lits"])).unwrap();
    let la: BTreeSet<Vec<String>> = la.into_iter().collect();
    let want: BTreeSet<Vec<String>> = [["(?<=q)", "rules/r.rerail", "3"], ["(?!z)", "rules/r.rerail", "4"]].iter().map(|r| r.iter().map(|v| (*v).to_owned()).collect()).collect();
    assert_eq!(la, want);
    assert_eq!(lits.len(), 1, "the escaped quote is the regex's `\"`: {lits:?}");
    let [off] = <[Vec<Vec<String>>; 1]>::try_from(guests_run("rr-regex-off", &[("rules/r.rerail", rr)], &found(""), &["la"])).unwrap();
    assert!(off.is_empty(), "{off:?}");
}

/// Decision 0495 R4: Python's pattern fragment reads a rule's pattern as a module whose metavariables are names; a
/// file's parse refuses the same text, so only a pattern reads `$`.
#[test]
fn a_python_pattern_reads_metavariables_as_names() {
    let src = b"subprocess.run($CMD, shell=True)\nf($$$ARGS, $_, $$$)\n";
    let tape = rerail_python_lean::tape_from(src, crate::code_rule_dsl::embed::rule_pattern::START).expect("a pattern reads");
    let text = rerail_python_lean::tape_text(&tape);
    assert!(text.starts_with("module"), "{text}");
    assert!(!text.contains("ERROR"), "{text}");
    // `subprocess`, `run`, `$CMD`, `shell`, `f`, `$$$ARGS`, `$_`, `$$$`: each a name.
    assert_eq!(text.lines().filter(|l| l.trim_start().starts_with("identifier")).count(), 8, "{text}");
    assert!(rerail_python_lean::tape(src).is_err(), "a file is not a pattern");
    assert!(crate::code_rule_dsl::language::fragments().iter().any(|&(name, d, start, _)| name == "python-pattern" && d == crate::code_rule_dsl::language::GrammarDialect::Python && start == "pattern_module"));
}

/// Decision 0495 R5: a `.language python` rule file's patterns are Python programs. A Python run that says
/// `.embedded on` reads each one at the rule file's line, a fence of two backticks without its spaces; a pattern in
/// a rule file of a language with no pattern fragment (TypeScript) is none; without `.embedded on`, the run reads
/// its own files alone (here none).
#[test]
fn a_rule_files_patterns_are_programs_of_its_language() {
    let py = ".language python\n.decl call(c: node)\ncall(c) :- c ~ `subprocess.run($CMD, shell=True)`.\ncall(c) :- c ~ `` os.system($X) ``.\n";
    let ts = ".language typescript\n.decl call(c: node)\ncall(c) :- c ~ `eval($X)`.\n";
    let calls = |embedded: &str| format!(".language python\n{embedded}\n.decl calls(x: symbol, p: symbol, l: number)\ncalls(x, p, l) :- kind(c, \"call\"), text(c, x), docof(c, d), doc(d, p, _), line(c, l).\n.output calls\n");
    // The Python rule file holds two programs, the TypeScript one none.
    let held = |src: &str| {
        let grammar = crate::code_rule_dsl::tape::Grammar::embedded(crate::code_rule_dsl::language::GrammarDialect::Rr).unwrap();
        let image = crate::code_rule_dsl::tape::TapeImage::parse(&grammar, src, &|| false).expect("parsed").expect("encoded");
        image.derived.iter().filter(|d| d.via == crate::code_rule_dsl::language::Via::Pattern).count()
    };
    assert_eq!((held(py), held(ts)), (2, 0));
    let files = [("rules/p.rerail", py), ("rules/t.rerail", ts)];
    let [on] = <[Vec<Vec<String>>; 1]>::try_from(guests_run("rr-pattern-on", &files, &calls(".embedded on"), &["calls"])).unwrap();
    let on: BTreeSet<Vec<String>> = on.into_iter().collect();
    let want: BTreeSet<Vec<String>> = [["subprocess.run($CMD, shell=True)", "rules/p.rerail", "3"], ["os.system($X)", "rules/p.rerail", "4"]].iter().map(|r| r.iter().map(|v| (*v).to_owned()).collect()).collect();
    assert_eq!(on, want);
    let [off] = <[Vec<Vec<String>>; 1]>::try_from(guests_run("rr-pattern-off", &files, &calls(""), &["calls"])).unwrap();
    assert!(off.is_empty(), "{off:?}");
}

/// Decision 0470 step 2, as 0512 builds it: a run under `.embedded on` reads a host file for its programs and sees
/// none of the file's own nodes (the file's tree is a guest of the file in its own grammar, in the hidden keys), but
/// the file itself is shown as any file is: its `doc` row, its lines and their text, so a finding at a program's node
/// is reported at the file's line, and a rule can read the line it is on (Q385).
#[test]
fn a_host_files_own_nodes_are_not_shown() {
    let rr = ".decl a(x: symbol)\na({\"k\": 1}).\n";
    let rule = ".language json\n.embedded on\n.decl pairs(x: symbol, p: symbol, l: number)\npairs(x, p, l) :- kind(n, \"pair\"), text(n, x), docof(n, d), doc(d, p, _), line(n, l).\n.decl own(k: symbol)\nown(k) :- kind(_, k), (k = \"source_file\" ; k = \"clause\").\n.decl lines(p: symbol, l: number, t: symbol)\nlines(p, l, t) :- src_line(d, l, sp), text(sp, t), doc(d, p, _).\n.output pairs, own, lines\n";
    let [pairs, own, lines] = <[Vec<Vec<String>>; 3]>::try_from(guests_run("host-own-nodes", &[("rules/r.rerail", rr)], rule, &["pairs", "own", "lines"])).unwrap();
    assert_eq!(pairs, [["\"k\": 1", "rules/r.rerail", "2"]], "the program is shown, at its file's line");
    assert!(own.is_empty(), "the host file's own nodes are not: {own:?}");
    // The run's own rule file (`g.rerail`) is a file of another grammar in its root too, shown as this one is.
    let lines: Vec<&Vec<String>> = lines.iter().filter(|r| r[0] == "rules/r.rerail").collect();
    // Every line, the empty one after the final terminator too (decision 0612: `src_line` is the lines `line` numbers).
    assert_eq!(lines, [&["rules/r.rerail", "1", ".decl a(x: symbol)"], &["rules/r.rerail", "2", "a({\"k\": 1})."], &["rules/r.rerail", "3", ""]], "the host file's lines and their text are shown");
}

/// Q324 (decision 0431 §4, "a program is named by its file and place"): where a program declares `docof` over nodes
/// as T's programs do, it is `node`'s projection, and `node`'s document column is a program's file, so `docof` of a
/// script's node is the HTML file that holds it, as core.rerail's `docof` is.
#[test]
fn a_t_programs_docof_of_a_programs_node_is_its_file() {
    let html = "<p>x</p>\n<script>f(1)</script>\n";
    let rule = r#"
.language javascript
.embedded on
.decl doc(d: number, path: symbol)
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl text(n: node, s: symbol)
.input doc, node, text
.decl docof(n: node, d: number)
docof(n, d) :- node(d, n, _, _, _).
.decl calls(x: symbol, p: symbol)
calls(x, p) :- node(_, i, "identifier", _, _), text(i, x), docof(i, d), doc(d, p).
.decl keys(i: node, d: number)
keys(i, d) :- node(_, i, "identifier", _, _), docof(i, d).
.output calls, keys
"#;
    let [calls, keys] = <[Vec<Vec<String>>; 2]>::try_from(guests_run("t-docof-program", &[("b.html", html)], rule, &["calls", "keys"])).unwrap();
    assert_eq!(calls, [["f", "b.html"]], "a script's name is its HTML file's");
    // The node's id keeps the program's own key underneath; its document is the file's, not that key.
    assert_eq!(keys.len(), 1, "{keys:?}");
    assert!(!keys[0][0].starts_with(&format!("guest{}:", keys[0][1])), "docof is the file's key, not the program's: {keys:?}");
}

/// python/calls over four modules, with its cone of requests and its typing run, on a pool of 2 threads: the run
/// ends. The cone (`threads::beside`) was a job that held one of the pool's threads while it waited for its answers,
/// and the typing run's main thread, itself a job, waited on its bell running none, so no thread was left to build
/// the chunks the run waited for: it never ended (netbox at 4 threads; any corpus at 2). The pool is the process's,
/// so the run is a child process of its own (`demand_split_child`), killed if it has not ended in 120 s.
#[test]
fn a_run_on_two_threads_ends() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("src");
    std::fs::create_dir_all(&root).unwrap();
    for m in 0..4 {
        std::fs::write(root.join(format!("m{m}.py")), format!("def f{m}(x):\n    return len(x)\n\n\ndef g{m}():\n    print(f{m}([1]))\n")).unwrap();
    }
    let rule = directory.path().join("calls.rerail");
    std::fs::write(&rule, ".policy \"two-threads\" revision 1\n.language python\n.scope include \"**/*.py\"\n#include \"python/calls.rerail\"\n.violation P(at: node) \"c\"\nP(c) :- call_target(c, f), kind(f, \"nothing_has_this_kind\").\n").unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap());
    child
        .args(["--exact", "code_rule_dsl::logic::rr::tests::demand_split_child", "--ignored", "--nocapture"])
        .env("RERAIL_THREADS", "2")
        .env("RERAIL_TAPE_CACHE", "off")
        .env("DEMAND_SPLIT_RULE", &rule)
        .env("DEMAND_SPLIT_ROOT", &root)
        .env("DEMAND_SPLIT_OUT", directory.path().join("out"));
    let running = crate::code_rule_dsl::logic::plan::corpus::switches::started(|| child.spawn()).unwrap();
    let status = ended_within(running, std::time::Duration::from_secs(120));
    assert!(status.is_some_and(|s| s.success()), "the run on 2 threads did not end well within 120 s: {status:?}");
}

/// `child`'s exit, or `None` (and the child killed) when it has not ended within `limit`.
fn ended_within(mut child: std::process::Child, limit: std::time::Duration) -> Option<std::process::ExitStatus> {
    let start = std::time::Instant::now();
    while start.elapsed() < limit {
        if let Some(status) = child.try_wait().unwrap() {
            return Some(status);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    None
}

/// A policy whose program does not load names the policies its own text declares, read without its includes or
/// its lowering: a setting its lowering refuses still leaves its `.policy` readable.
#[test]
fn a_texts_declared_policies_read_without_lowering() {
    let src = "#include \"core.rerail\"\n.policy \"bad\" revision 1\n.language python\n.references {\"unknownParameter\":true}\n.violation V(at: node) \"edge\"\nV(n) :- kind(n, \"call\").\n";
    assert_eq!(declared_policies(src), ["bad"]);
    assert_eq!(declared_policies(".policy \"one\" revision 1\n.policy \"two\" revision 1\n"), ["one", "two"]);
    assert!(declared_policies(".violation V(at: node) \"x\"\nV(n) :- kind(n, \"call\").\n").is_empty());
    assert!(declared_policies(".policy \"unclosed").is_empty(), "a text that does not parse declares none");
}

/// Every native step's answer is declared in the library (`.answer`, t33 K3), and the library declares no other:
/// the engine's registry (`rr::WORLD`, and `load`'s `loaded`) and the library's declarations are one list. Here, in
/// the binary booted as the engine boots (its languages' answerers installed), since the crate split.
#[test]
fn every_native_answer_is_declared_in_the_library() {
    use std::collections::{BTreeMap, BTreeSet};
    let mut engine: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (rel, req) in super::WORLD.iter().copied().chain([("loaded", "load")]) {
        engine.entry(req.to_owned()).or_default().insert(rel.to_owned());
    }
    let mut library: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut dirs = vec![super::library()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "rerail") {
                for line in std::fs::read_to_string(&path).unwrap().lines() {
                    let Some(text) = line.trim().strip_prefix(".answer ") else { continue };
                    let answer = crate::code_rule_dsl::logic::front::answers::parse(text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                    library.entry(answer.req).or_default().extend(answer.produces);
                }
            }
        }
    }
    assert_eq!(library, engine, "the library's `.answer`s and the engine's answerers");
}

/// A run's call opens the image store once and holds it until it returns (`held_image_store`, `HoldImages`): opened at
/// each ask, its every close made SQLite delete and make again its write-ahead log. A call after it opens its own, and
/// none holds it once returned; outside a call, each ask opens it.
#[test]
fn a_run_opens_the_image_store_once() {
    let opened = std::sync::atomic::AtomicUsize::new(0);
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("held-image-store");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let open = || {
        opened.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        crate::content_store::ContentStore::open_images(&dir.join("images.sqlite")).ok().map(std::sync::Arc::new)
    };
    let count = || opened.load(std::sync::atomic::Ordering::Relaxed);
    super::held_image_store(open);
    super::held_image_store(open);
    assert_eq!(count(), 2, "outside a run, each ask opens it");
    let _run = crate::code_rule_dsl::logic::profile::test_run();
    for call in 1..=2 {
        {
            let _call = super::HoldImages::enter();
            let _inner = super::HoldImages::enter();
            let a = super::held_image_store(open).expect("the store");
            drop(_inner);
            let b = super::held_image_store(open).expect("the store");
            assert!(std::sync::Arc::ptr_eq(&a, &b), "call {call}: one store, past an inner call");
            assert_eq!(count(), 2 + call, "call {call} opened it once");
        }
        // The call returned: nothing holds the store open (a test's store is checked so after its run).
        assert_eq!(crate::content_store::holding(&dir.join("images.sqlite")), (0, 0), "call {call} let it go");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Spike for decision 0639 (a program linked from its library's modules and its own module). Measures, per program of
/// the packs named (`RERAIL_SPIKE_PACKS`, comma-separated pack directories, `phases` for the engine's phase programs;
/// `RERAIL_SPIKE_N` programs a pack at most):
/// - E1: whether `contexts::passes(link_library_versions(library modules ++ [the text's own module]))` prints as
///   `parse_version` does, and each side's time on this thread;
/// - E2: whether a library's lowering depends on its files' order (the include order against a canonical order: by
///   the count of files each reaches, then by digest), whole and linked from modules;
/// - E3: whether a module lowered against its scope in two orders is one image;
/// - E4: the module images' bytes, the decode's time, and helpers that read themselves.
#[test]
#[ignore = "spike for decision 0639: RERAIL_SPIKE_PACKS=python[,phases] RERAIL_SPIKE_N=<programs a pack>"]
fn spike_0639_programs_linked_from_modules() {
    use std::collections::{BTreeMap, HashMap};
    use std::time::Instant;
    let only: BTreeSet<String> = std::env::var("RERAIL_SPIKE_PACKS").unwrap_or_else(|_| "python".into()).split(',').map(str::to_owned).collect();
    let per: usize = std::env::var("RERAIL_SPIKE_N").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
    let lib = crate::code_rule_dsl::logic::rr::library();
    let mut programs: Vec<(String, PathBuf)> = Vec::new();
    for name in &only {
        let dir = if name == "phases" { lib.join("../../../native/src/code_rule_dsl/typing/programs") } else { lib.join("../packs").join(name) };
        let mut files = walk_rr(&dir);
        files.sort();
        programs.extend(files.into_iter().take(per).map(|p| (name.clone(), p)));
    }
    let srcs: Vec<String> = programs.iter().map(|(_, p)| std::fs::read_to_string(p).unwrap()).collect();
    let found = super::modules_of(&srcs.iter().map(String::as_str).collect::<Vec<_>>());
    let no_includes = |s: Vec<from_tape::Stmt>| -> Vec<from_tape::Stmt> { s.into_iter().filter(|s| !matches!(s.what(), Some(from_tape::S::Include(_)))).collect() };
    let printed = |v: &Arc<Version>| print_version(v).unwrap();
    // Lines one side has and the other not, helper numbers aside.
    let plain = |l: &str| l.split(|c: char| !(c.is_alphanumeric() || c == '_')).map(|w| if w.starts_with("rr_") { w.trim_end_matches(|c: char| c.is_ascii_digit()) } else { w }).collect::<Vec<_>>().join(" ");
    let differs = |a: &str, b: &str| -> String {
        let (sa, sb): (BTreeSet<String>, BTreeSet<String>) = (a.lines().map(plain).collect(), b.lines().map(plain).collect());
        let only = |x: &BTreeSet<String>, y: &BTreeSet<String>| x.difference(y).take(6).cloned().collect::<Vec<_>>().join("\n      ");
        let first = a.lines().zip(b.lines()).position(|(x, y)| x != y).unwrap_or(0);
        format!("{} lines and {}, first differing line {first}; numbers aside {}:\n    only the first:\n      {}\n    only the second:\n      {}", a.lines().count(), b.lines().count(), if sa == sb { "the same lines" } else { "other lines" }, only(&sa, &sb), only(&sb, &sa))
    };
    // E1. `RERAIL_SPIKE_PROFILE=1`: the spans of both paths, by name (decision 0442's run scope around them).
    let profiling = std::env::var_os("RERAIL_SPIKE_PROFILE").is_some();
    let _run = profiling.then(|| crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env));
    if profiling {
        crate::code_rule_dsl::logic::profile::set(true);
        crate::code_rule_dsl::logic::profile::reset();
    }
    let mut libraries: HashMap<Vec<PathBuf>, Vec<lower::Module>> = HashMap::new();
    let mut pack_of: HashMap<Vec<PathBuf>, String> = HashMap::new();
    let (mut equal, mut differ, mut same_lines, mut inline_err, mut module_err) = (0, 0, 0, 0, 0);
    let (mut t_inline, mut t_own, mut t_link, mut t_lib) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let mut shown = 0;
    // E11: the guest context passes (0597) by module. Per library, its link alone: its rules as printed, the relations
    // they head, and whether it declares a pass. Per pack: programs, those whose library declares passes, those whose
    // own file declares one (`.context`, `.from`), library pass rules the program's link lacks, and the program's rules
    // over its library's of each kind: a pass instance's (a completion), its own reading an instance (renamed), and one
    // heading a relation the library's rules head (rules added across modules).
    let mut e11_lib: HashMap<Vec<PathBuf>, (BTreeSet<String>, BTreeSet<String>, bool)> = HashMap::new();
    let mut e11: BTreeMap<String, [usize; 7]> = BTreeMap::new();
    let mut e11_shown = 0;
    for (((pack, path), src), m) in programs.iter().zip(&srcs).zip(found) {
        let Ok(Some(m)) = m else { continue };
        let at = Instant::now();
        let inline = parse_version(src);
        let inline_s = at.elapsed().as_secs_f64();
        let inline = match inline {
            Ok(v) => v,
            Err(e) => {
                inline_err += 1;
                println!("E1 {pack} {}: fails inline: {}", path.display(), e.to_string().chars().take(200).collect::<String>());
                continue;
            }
        };
        let paths: Vec<PathBuf> = m.files.iter().map(|(p, _, _)| p.clone()).collect();
        if !libraries.contains_key(&paths) {
            let at = Instant::now();
            let modules = library_modules(&paths).unwrap_or_else(|e| panic!("{}: the library's modules: {e}", path.display()));
            t_lib += at.elapsed().as_secs_f64();
            libraries.insert(paths.clone(), modules);
            pack_of.insert(paths.clone(), pack.clone());
        }
        let modules = &libraries[&paths];
        let own = no_includes(read(path).unwrap().0);
        let at = Instant::now();
        let scope: Vec<&lower::Module> = modules.iter().collect();
        let own = lower::lower_module_in_written(own, &scope);
        let own_s = at.elapsed().as_secs_f64();
        let at = Instant::now();
        let linked = own.and_then(|own| {
            let all: Vec<lower::Module> = {
                let _s = crate::code_rule_dsl::logic::profile::span(|| "spike: the library's modules cloned for a link".into());
                modules.iter().cloned().chain([own]).collect()
            };
            super::link_library_versions(all)
        }).and_then(|v| {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "spike: contexts::passes after the link".into());
            crate::code_rule_dsl::logic::contexts::passes(&v)
        });
        let link_s = at.elapsed().as_secs_f64();
        let linked = match linked {
            Ok(v) => v,
            Err(e) => {
                module_err += 1;
                if shown < 8 {
                    shown += 1;
                    println!("E1 {pack} {}: the module path fails: {e}", path.display());
                }
                continue;
            }
        };
        t_inline += inline_s;
        t_own += own_s;
        t_link += link_s;
        {
            use crate::code_rule_dsl::logic::passes_made::PREFIX;
            let pass_rule = |v: &Version, r: crate::code_rule_dsl::logic::program_image::RuleRef<'_>| v.rel_name(r.head().rel()).starts_with(PREFIX) || r.reads().any(|a| v.rel_name(a.rel()).starts_with(PREFIX));
            let (lib_rules, lib_heads, lib_ctx) = e11_lib.entry(paths.clone()).or_insert_with(|| {
                let v = super::link_library_versions(modules.clone()).and_then(|v| crate::code_rule_dsl::logic::contexts::passes(&v)).unwrap();
                let ctx = crate::code_rule_dsl::logic::contexts::declared(&v).is_ok_and(|d| !d.contexts.is_empty());
                (v.rules().map(|r| v.show_rule(r)).collect(), v.rules().map(|r| v.rel_name(r.head().rel()).to_owned()).collect(), ctx)
            });
            let own_decl = src.lines().any(|l| l.trim_start().starts_with(".context") || l.trim_start().starts_with(".from"));
            let all: BTreeSet<String> = linked.rules().map(|r| linked.show_rule(r)).collect();
            let lib_pass_missing = lib_rules.iter().filter(|t| t.contains(PREFIX) && !all.contains(*t)).count();
            let (mut completion, mut renamed, mut across) = (0, 0, 0);
            for r in linked.rules() {
                let t = linked.show_rule(r);
                if lib_rules.contains(&t) {
                    continue;
                }
                let head = linked.rel_name(r.head().rel());
                if head.starts_with(PREFIX) {
                    completion += 1;
                } else if pass_rule(&linked, r) {
                    renamed += 1;
                }
                if lib_heads.contains(head) {
                    across += 1;
                    if e11_shown < 6 && (head.starts_with(PREFIX) || pass_rule(&linked, r)) {
                        e11_shown += 1;
                        println!("E11 {pack} {}: a pass rule across modules: {}", path.display(), t.chars().take(200).collect::<String>());
                    }
                }
            }
            let e = e11.entry(pack.clone()).or_default();
            for (i, x) in [1, usize::from(*lib_ctx), usize::from(own_decl), lib_pass_missing, completion, renamed, across].into_iter().enumerate() {
                e[i] += x;
            }
        }
        let (a, b) = (printed(&inline), printed(&linked));
        if a == b {
            equal += 1;
        } else {
            differ += 1;
            let d = differs(&a, &b);
            same_lines += usize::from(d.contains("the same lines"));
            if shown < 8 {
                shown += 1;
                println!("E1 {pack} {}: {d}", path.display());
            }
        }
    }
    for (pack, [n, lib_ctx, own_decl, missing, completion, renamed, across]) in &e11 {
        println!("E11 {pack}: {n} programs, {lib_ctx} with passes in their library, {own_decl} declaring one; library pass rules a program's link lacks {missing}; the programs' rules over their library's: {completion} of a pass instance, {renamed} reading one, {across} heading a library relation");
    }
    println!("E1: {equal} print the same, {differ} differ ({same_lines} of them by order or numbers alone), {module_err} fail on the module path, {inline_err} fail inline");
    if profiling {
        let mut by: BTreeMap<String, (f64, f64, u64)> = BTreeMap::new();
        for (path, t) in crate::code_rule_dsl::logic::profile::spans() {
            let leaf = path.rsplit('/').next().unwrap_or("").chars().map(|c| if c.is_ascii_digit() { '#' } else { c }).collect::<String>();
            let e = by.entry(leaf).or_default();
            e.0 += t.own;
            e.1 += t.total;
            e.2 += t.calls;
        }
        let mut all: Vec<_> = by.into_iter().collect();
        all.sort_by(|a, b| b.1.0.total_cmp(&a.1.0));
        for (leaf, (own, total, calls)) in all.into_iter().take(45) {
            println!("P {own:8.3} own {total:8.3} total {calls:7} calls  {}", leaf.chars().take(110).collect::<String>());
        }
        crate::code_rule_dsl::logic::profile::set(false);
    }
    println!("E1 time on this thread, over the programs compared: inline {t_inline:.3} s; own module {t_own:.3} s, link + tail + passes {t_link:.3} s; the libraries' modules lowered once {t_lib:.3} s ({} libraries)", libraries.len());
    // E2, E3, E4 over each pack's whole library, as a run of the pack lists it.
    let mut by_pack: HashMap<String, Vec<PathBuf>> = HashMap::new();
    for paths in libraries.keys() {
        let pack = by_pack.entry(pack_of[paths].clone()).or_default();
        for p in paths {
            if !pack.contains(p) {
                pack.push(p.clone());
            }
        }
    }
    let mut packs: Vec<(String, Vec<PathBuf>)> = by_pack.into_iter().collect();
    packs.sort();
    for (pack, dfs) in packs {
        let core = lib.join("core.rerail").canonical().ok();
        let read_all: Vec<(Vec<from_tape::Stmt>, [u8; 32])> = dfs.iter().map(|p| {
            let (s, f) = read(p).unwrap();
            (s, f.sha256.unwrap_or_default())
        }).collect();
        let at = |p: &Path| dfs.iter().position(|q| q.canonical().ok().as_deref() == Some(p));
        let n = dfs.len();
        let mut reach: Vec<BTreeSet<usize>> = (0..n).map(|i| {
            let mut mine: BTreeSet<usize> = core.as_deref().and_then(at).filter(|&c| c != i).into_iter().collect();
            for s in &read_all[i].0 {
                if let Some(from_tape::S::Include(name)) = s.what() {
                    let found = [lib.join(name.as_str()), dfs[i].parent().unwrap().join(name.as_str())].into_iter().find_map(|p| p.canonical().ok());
                    mine.extend(found.as_deref().and_then(at));
                }
            }
            mine.remove(&i);
            mine
        }).collect();
        loop {
            let mut grew = false;
            for i in 0..n {
                let more: BTreeSet<usize> = reach[i].iter().flat_map(|&j| reach[j].iter().copied()).collect();
                let before = reach[i].len();
                reach[i].extend(more);
                grew |= reach[i].len() != before;
            }
            if !grew {
                break;
            }
        }
        let cycles = (0..n).filter(|&i| reach[i].contains(&i)).count();
        let hex = |d: &[u8; 32]| d.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by_key(|&i| (reach[i].len() + usize::from(!reach[i].contains(&i)), hex(&read_all[i].1)));
        let topological = order.iter().enumerate().all(|(k, &i)| reach[i].iter().all(|&j| order.iter().position(|&o| o == j).unwrap() < k || reach[j].contains(&i)));
        let files = |ix: &[usize]| -> Files { ix.iter().map(|&i| (dfs[i].clone(), no_includes(read_all[i].0.clone()))).collect() };
        let whole = |f: &Files| printed(&super::lower_statements_version(f.iter().flat_map(|(_, s)| s.iter().cloned()).collect()).unwrap());
        let (in_dfs, canonical) = (files(&(0..n).collect::<Vec<_>>()), files(&order));
        let (w_dfs, w_canon) = (whole(&in_dfs), whole(&canonical));
        println!("E2 {pack}: {n} files, {cycles} in include cycles, the canonical order topological: {topological}; whole in include order against canonical: {}", if w_dfs == w_canon { "identical".to_owned() } else { differs(&w_dfs, &w_canon) });
        let ordered: Vec<PathBuf> = order.iter().map(|&i| dfs[i].clone()).collect();
        match library_modules(&ordered).and_then(super::link_library_versions) {
            Ok(v) => {
                let l = printed(&v);
                println!("E2 {pack}: linked from modules in canonical order against whole in canonical order: {}", if l == w_canon { "identical".to_owned() } else { differs(&w_canon, &l) });
            }
            Err(e) => println!("E2 {pack}: linked in canonical order fails: {e}"),
        }
        // E5: the module plan as rules (decision 0639 §1) over the library's include edges and digests, against the plan
        // above: the same modules, scopes and ranks, and keys that do not depend on the order the facts are stated in.
        let direct: Vec<BTreeSet<usize>> = (0..n).map(|i| {
            let mut mine: BTreeSet<usize> = core.as_deref().and_then(at).filter(|&c| c != i).into_iter().collect();
            for s in &read_all[i].0 {
                if let Some(from_tape::S::Include(name)) = s.what() {
                    let found = [lib.join(name.as_str()), dfs[i].parent().unwrap().join(name.as_str())].into_iter().find_map(|p| p.canonical().ok());
                    mine.extend(found.as_deref().and_then(at));
                }
            }
            mine.remove(&i);
            mine
        }).collect();
        let plan_rules = r#"#include "core.rerail"
.decl p0_include(from: symbol, at: number, name: symbol, path: symbol)
.decl p0_digest(path: symbol, d: symbol)
.decl p0_build(b: symbol)
.decl p0_edge(a: symbol, b: symbol)
p0_edge(a, b) :- p0_include(a, _, _, b), p0_digest(a, _), p0_digest(b, _).
.decl p0_reaches(a: symbol, b: symbol)
p0_reaches(a, b) :- p0_edge(a, b).
p0_reaches(a, c) :- p0_reaches(a, b), p0_edge(b, c).
.decl p0_member(p: symbol, q: symbol)
p0_member(p, p) :- p0_digest(p, _).
p0_member(p, q) :- p0_reaches(p, q), p0_reaches(q, p).
.decl p0_module_of(p: symbol, m: symbol)
p0_module_of(p, m) :- p0_digest(p, _), m = join d by d sep "+" : { p0_member(p, q), p0_digest(q, d) }.
.decl p0_closure(m: symbol, q: symbol)
p0_closure(m, q) :- p0_module_of(p, m), p0_member(p, q).
p0_closure(m, q) :- p0_module_of(p, m), p0_reaches(p, q).
.decl p0_scope(m: symbol, s: symbol)
p0_scope(m, s) :- p0_closure(m, q), p0_module_of(q, s), s != m.
.decl p0_module_rank(m: symbol, reached: number)
p0_module_rank(m, k) :- p0_module_of(_, m), k = count : { p0_closure(m, _) }.
.decl p0_closure_digests(m: symbol, ds: symbol)
p0_closure_digests(m, ds) :- p0_module_rank(m, _), ds = join d by d sep "," : { p0_closure(m, q), p0_digest(q, d) }.
.decl p0_edge_text(m: symbol, e: symbol)
p0_edge_text(m, e) :- p0_closure(m, a), p0_edge(a, b), p0_digest(a, da), p0_digest(b, db), e = cat(da, ">", db).
.decl p0_closure_edges(m: symbol, es: symbol)
p0_closure_edges(m, es) :- p0_module_rank(m, _), es = join e by e sep "," : { p0_edge_text(m, e) }.
.decl p0_module_key(m: symbol, key: symbol)
p0_module_key(m, @sha256(cat(b, "|", m, "|", ds, "|", es))) :- p0_build(b), p0_closure_digests(m, ds), p0_closure_edges(m, es).
.output p0_module_of, p0_scope, p0_module_rank, p0_module_key
"#;
        let name_of = |i: usize| dfs[i].display().to_string();
        let facts = |reverse: bool| -> String {
            let mut lines: Vec<String> = vec!["p0_build(\"spike\").".to_owned()];
            for i in 0..n {
                lines.push(format!("p0_digest({:?}, {:?}).", name_of(i), hex(&read_all[i].1)));
                for (k, &j) in direct[i].iter().enumerate() {
                    lines.push(format!("p0_include({:?}, {k}, \"x\", {:?}).", name_of(i), name_of(j)));
                }
            }
            if reverse {
                lines.reverse();
            }
            lines.join("\n")
        };
        type Plan = (BTreeMap<String, String>, BTreeSet<(String, String)>, BTreeMap<String, i64>, BTreeMap<String, String>);
        let plan_of = |reverse: bool| -> Plan {
            let t = Instant::now();
            let plan = guests_run_with("p0plan", &[("a.py", "x = 1\n")], &format!("{plan_rules}\n{}\n", facts(reverse)), |o| {
                let rows = |rel: &str| o.get(rel).map(|r| r.shown()).unwrap_or_default().into_iter().map(|r| r.into_iter().map(|c| c.trim_matches('"').to_owned()).collect::<Vec<String>>()).collect::<Vec<_>>();
                (
                    rows("p0_module_of").into_iter().map(|r| (r[0].clone(), r[1].clone())).collect(),
                    rows("p0_scope").into_iter().map(|r| (r[0].clone(), r[1].clone())).collect(),
                    rows("p0_module_rank").into_iter().map(|r| (r[0].clone(), r[1].parse().unwrap())).collect(),
                    rows("p0_module_key").into_iter().map(|r| (r[0].clone(), r[1].clone())).collect(),
                )
            });
            println!("E5 {pack}: the plan's run {:.3} s", t.elapsed().as_secs_f64());
            plan
        };
        let (of, scope, rank, key) = plan_of(false);
        let (_, _, _, key_rev) = plan_of(true);
        // The plan in Rust, by the same names: a module by its members' digests, joined in order.
        let module_name = |i: usize| {
            let mut ds: Vec<String> = (0..n).filter(|&j| j == i || (reach[i].contains(&j) && reach[j].contains(&i))).map(|j| hex(&read_all[j].1)).collect();
            ds.sort();
            ds.join("+")
        };
        let rust_of: BTreeMap<String, String> = (0..n).map(|i| (name_of(i), module_name(i))).collect();
        let rust_scope: BTreeSet<(String, String)> = (0..n).flat_map(|i| reach[i].iter().map(move |&j| (i, j))).map(|(i, j)| (module_name(i), module_name(j))).filter(|(a, b)| a != b).collect();
        let rust_rank: BTreeMap<String, i64> = (0..n).map(|i| (module_name(i), (reach[i].len() + usize::from(!reach[i].contains(&i))) as i64)).collect();
        println!(
            "E5 {pack}: rules against Rust: modules {}, scopes {}, ranks {}; {} keys, {} distinct, stated in reverse {}",
            if of == rust_of { "equal" } else { "differ" },
            if scope == rust_scope { "equal".to_owned() } else { format!("differ ({} and {})", scope.len(), rust_scope.len()) },
            if rank == rust_rank { "equal" } else { "differ" },
            key.len(),
            key.values().collect::<BTreeSet<_>>().len(),
            if key == key_rev { "the same keys" } else { "other keys" }
        );
        // E3 and E4: files that reach each other are one module, its files in canonical order; each module against its
        // scope forward and reversed.
        let mut unit_of: Vec<usize> = vec![usize::MAX; n];
        let mut units: Vec<Vec<usize>> = Vec::new();
        for &i in &order {
            if unit_of[i] != usize::MAX {
                continue;
            }
            let members: Vec<usize> = order.iter().copied().filter(|&j| j == i || (reach[i].contains(&j) && reach[j].contains(&i))).collect();
            for &j in &members {
                unit_of[j] = units.len();
            }
            units.push(members);
        }
        let mut built: Vec<lower::Module> = Vec::new();
        let (mut same, mut other, mut bytes, mut decode_s, mut helpers_self) = (0, 0, 0usize, 0.0f64, 0usize);
        for (u, members) in units.iter().enumerate() {
            let scope_ix: Vec<usize> = (0..u).filter(|&v| members.iter().any(|&i| reach[i].contains(&units[v][0]))).collect();
            let fwd: Vec<&lower::Module> = scope_ix.iter().map(|&v| &built[v]).collect();
            let rev: Vec<&lower::Module> = scope_ix.iter().rev().map(|&v| &built[v]).collect();
            let st: Vec<from_tape::Stmt> = members.iter().flat_map(|&i| no_includes(read_all[i].0.clone())).collect();
            let name = dfs[members[0]].display().to_string();
            let a = lower::lower_module_in_written(st.clone(), &fwd).unwrap_or_else(|e| panic!("{name}: {e}"));
            let b = lower::lower_module_in_written(st, &rev).unwrap_or_else(|e| panic!("{name}: reversed: {e}"));
            let (ia, ib) = (module_image::encode(&a).unwrap(), module_image::encode(&b).unwrap());
            if ia.bytes() == ib.bytes() {
                same += 1;
            } else {
                other += 1;
                println!("E3 {pack} {name}: the scope's order changes the image ({} bytes and {}): {}", ia.bytes().len(), ib.bytes().len(), differs(&printed(a.version()), &printed(b.version())));
            }
            bytes += ia.bytes().len();
            for &i in members {
                let file = dfs[i].file_name().unwrap().to_string_lossy().into_owned();
                let stem = dfs[i].file_stem().unwrap().to_string_lossy().into_owned();
                let has = |needle: &str| ia.bytes().windows(needle.len()).any(|w| w == needle.as_bytes());
                if has(&dfs[i].display().to_string()) || has(&file) || (stem.len() > 6 && has(&stem)) {
                    println!("E4 {pack} {}: its image holds its path, name or stem ({}, {}, {})", dfs[i].display(), has(&dfs[i].display().to_string()), has(&file), has(&stem));
                }
            }
            let t = Instant::now();
            let _ = module_image::decode(Arc::new(ia)).unwrap();
            decode_s += t.elapsed().as_secs_f64();
            let text = printed(a.version());
            for line in text.lines() {
                let Some((head, body)) = line.split_once(":-") else { continue };
                let name: String = head.trim().chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                if name.starts_with("rr_") && body.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| w == name) {
                    helpers_self += 1;
                }
            }
            built.push(a);
        }
        println!("E3 {pack}: {same} modules one image under both scope orders, {other} not");
        println!("E4 {pack}: {} module images, {bytes} bytes; decoded in {decode_s:.4} s; {helpers_self} helper rules read their own head", built.len());
        // E7: the fields `append_fields` merges in link order, over the modules (helpers aside): a key two modules give
        // different values (where "a later module's replaces" decides), and the ordered lists more than one module adds to
        // (where the link's order is the list's order).
        {
            let plain = |r: &String| !r.starts_with("rr_");
            let mut conflicts: Vec<String> = Vec::new();
            let mut cols: BTreeMap<String, (usize, Vec<(String, String)>)> = BTreeMap::new();
            let mut natives: BTreeMap<String, usize> = BTreeMap::new();
            let mut sets: BTreeMap<String, (usize, Vec<String>)> = BTreeMap::new();
            let mut inputs_from: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
            let mut facts_from: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
            let mut settings_from: BTreeSet<usize> = BTreeSet::new();
            let mut demand_from: BTreeSet<usize> = BTreeSet::new();
            for (u, m) in built.iter().enumerate() {
                let v = m.version();
                for rel in v.declared_names().into_iter().filter(plain) {
                    let c = v.columns_of(&rel).unwrap_or_default();
                    match cols.get(&rel) {
                        Some((w, d)) if *d != c => conflicts.push(format!("{rel} declared by modules {w} and {u} with other columns")),
                        Some(_) => {}
                        None => { cols.insert(rel, (u, c)); }
                    }
                }
                for (rel, n) in v.natives() {
                    if let Some(w) = natives.get(&rel) {
                        if built[*w].version().natives().get(&rel) != Some(&n) {
                            conflicts.push(format!("native {rel} of modules {w} and {u} differ"));
                        }
                    } else {
                        natives.insert(rel, u);
                    }
                }
                for (set, kinds) in v.kindsets() {
                    match sets.get(&set) {
                        Some((w, k)) if *k != kinds => conflicts.push(format!("kind set {set} of modules {w} and {u} differ")),
                        Some(_) => {}
                        None => { sets.insert(set, (u, kinds)); }
                    }
                }
                for rel in v.input_names() {
                    inputs_from.entry(rel).or_default().insert(u);
                }
                for rel in v.fact_names().into_iter().filter(|r| v.has_fact_rows(r)) {
                    facts_from.entry(rel).or_default().insert(u);
                }
                if v.settings().is_ok_and(|s| !s.is_empty()) {
                    settings_from.insert(u);
                }
                if !v.demand().is_empty() {
                    demand_from.insert(u);
                }
            }
            let shared_facts: Vec<&String> = facts_from.iter().filter(|(_, us)| us.len() > 1).map(|(r, _)| r).collect();
            let shared_inputs = inputs_from.values().filter(|us| us.len() > 1).count();
            println!(
                "E7 {pack}: {} conflicts {:?}; inputs stated by more than one module {shared_inputs} of {}; relations with facts from more than one module {:?}; modules with settings {}, with demands {}",
                conflicts.len(), conflicts.iter().take(5).collect::<Vec<_>>(), inputs_from.len(), shared_facts.iter().take(8).collect::<Vec<_>>(), settings_from.len(), demand_from.len()
            );
        }
        // E6: helpers named by their content at each walk's end (`lower::content_names`), each module alone, then the
        // modules' rules as a set (a link that only unites), against the whole lowering of the same list, and against
        // the link (which unites by name).
        let named = |m: &lower::Module, _: &str| -> (Arc<Version>, Vec<Vec<String>>) { (Arc::clone(m.version()), Vec::new()) };
        let whole_m = lower::lower_module_in_written(canonical.iter().flat_map(|(_, s)| s.iter().cloned()).collect(), &[]).unwrap();
        let (w, w_cycles) = named(&whole_m, "whole");
        let mut union: BTreeSet<(String, [u8; 32])> = BTreeSet::new();
        let mut m_cycles = 0;
        let mut module_rules = 0;
        for (u, m) in built.iter().enumerate() {
            let (v, c) = named(m, &format!("module {u}"));
            m_cycles += c.len();
            module_rules += v.rules().count();
            union.extend(lower::rule_digests(&v));
        }
        let whole_rules = lower::rule_digests(&w);
        let today = lower::link_library(built.clone()).unwrap().with(None).unwrap();
        let today_rules: usize = today.version.rules().count();
        let helpers = |rules: &BTreeSet<(String, [u8; 32])>| rules.iter().map(|(h, _)| h).filter(|h| h.starts_with("rr_")).collect::<BTreeSet<_>>().len();
        let today_helpers: BTreeSet<String> = today.version.rules().map(|r| today.version.rel_name(r.head().rel()).to_owned()).filter(|h| h.starts_with("rr_")).collect();
        let whole_today_helpers: BTreeSet<String> = whole_m.version().rules().map(|r| whole_m.version().rel_name(r.head().rel()).to_owned()).filter(|h| h.starts_with("rr_")).collect();
        let only = |a: &BTreeSet<(String, [u8; 32])>, b: &BTreeSet<(String, [u8; 32])>| a.difference(b).map(|(h, _)| h.clone()).collect::<BTreeSet<_>>().into_iter().take(6).collect::<Vec<_>>().join(", ");
        let strip = |l: &str| -> String {
            l.split(|c: char| !(c.is_alphanumeric() || c == '_')).map(|w| if w.starts_with("rr_") { if w.len() > 17 && w.as_bytes()[w.len() - 17] == b'_' && w[w.len() - 16..].bytes().all(|b| b.is_ascii_hexdigit()) { w[..w.len() - 17].to_owned() } else { w.trim_end_matches(|c: char| c.is_ascii_digit()).to_owned() } } else { w.to_owned() }).collect::<Vec<_>>().join(" ")
        };
        let today_lines: BTreeSet<String> = today.version.rules().map(|r| strip(&today.version.show_rule(r))).collect();
        let content_lines: BTreeSet<String> = built.iter().enumerate().flat_map(|(u, m)| {
            let (v, _) = named(m, &format!("module {u}"));
            v.rules().map(|r| strip(&v.show_rule(r))).collect::<Vec<_>>()
        }).collect();
        println!("E6 {pack}: today's link against the content-named modules, helper names aside: {}", if today_lines == content_lines { "the same rules".to_owned() } else { format!("{} and {} lines; only today's: {:?}; only content's: {:?}", today_lines.len(), content_lines.len(), today_lines.difference(&content_lines).take(3).collect::<Vec<_>>(), content_lines.difference(&today_lines).take(3).collect::<Vec<_>>()) });
        println!(
            "E6 {pack}: content-named modules united: {} rules ({} written by the modules), {} helpers; content-named whole: {} rules, {} helpers; {}; today: link {} rules, {} helpers, whole {} helpers; cycles of helpers: modules {m_cycles}, whole {}",
            union.len(), module_rules, helpers(&union), whole_rules.len(), helpers(&whole_rules),
            if union == whole_rules { "the same rules".to_owned() } else { format!("other rules (only the united: {}; only the whole: {})", only(&union, &whole_rules), only(&whole_rules, &union)) },
            today_rules, today_helpers.len(), whole_today_helpers.len(), w_cycles.len()
        );
        // E8: what each module could finish alone. A relation is final in its module when nothing it reads, itself
        // included and to any depth over the united rules, has a rule from a module outside the module's scope and
        // itself: neither a later module adding rules to it (an `extensible` relation) nor one it reads.
        {
            use crate::code_rule_dsl::logic::program_image::handles::Lit as HL;
            let scope_of: Vec<BTreeSet<usize>> = (0..units.len()).map(|u| (0..u).filter(|&v| units[u].iter().any(|&i| reach[i].contains(&units[v][0]))).chain([u]).collect()).collect();
            // A relation's rules, by module, as digests: a helper two modules make alike (a `once` merge) is one
            // relation each module holds whole, not one a later module adds to.
            let mut by_module: BTreeMap<String, BTreeMap<usize, BTreeSet<[u8; 32]>>> = BTreeMap::new();
            let mut heads: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
            let mut reads: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
            let mut rules_of: Vec<Vec<String>> = vec![Vec::new(); units.len()];
            for (u, m) in built.iter().enumerate() {
                let (v, _) = named(m, &format!("module {u}"));
                for (h, d) in lower::rule_digests(&v) {
                    by_module.entry(h).or_default().entry(u).or_default().insert(d);
                }
                for r in v.rules() {
                    let h = v.rel_name(r.head().rel()).to_owned();
                    heads.entry(h.clone()).or_default().insert(u);
                    rules_of[u].push(h.clone());
                    let into = reads.entry(h).or_default();
                    for l in r.body() {
                        if let HL::Atom(a) | HL::Agg { atom: a, .. } = l.view() {
                            into.insert(v.rel_name(a.rel()).to_owned());
                        }
                    }
                }
            }
            // Alike in every module: any module holding it holds all of it.
            let mut alike_in: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
            for (h, per) in &by_module {
                let mut sets = per.values();
                let first = sets.next();
                if per.len() > 1 && sets.all(|s| Some(s) == first) {
                    heads.remove(h);
                    alike_in.insert(h.clone(), per.keys().copied().collect());
                }
            }
            let open: Vec<&String> = heads.iter().filter(|(_, us)| us.len() > 1).map(|(r, _)| r).collect();
            let alike = alike_in.len();
            let (mut final_rules, mut all_rules, mut final_rels, mut all_rels) = (0usize, 0usize, 0usize, 0usize);
            let mut why: BTreeMap<String, usize> = BTreeMap::new();
            for (u, hs) in rules_of.iter().enumerate() {
                let distinct: BTreeSet<&String> = hs.iter().collect();
                for h in distinct {
                    let (mut seen, mut todo, mut outside) = (BTreeSet::new(), vec![h.as_str()], None);
                    while let Some(x) = todo.pop() {
                        if !seen.insert(x) {
                            continue;
                        }
                        if let Some(us) = alike_in.get(x) && !us.iter().any(|v| scope_of[u].contains(v)) {
                            outside = Some(x.to_owned());
                            break;
                        }
                        if let Some(us) = heads.get(x) && !us.is_subset(&scope_of[u]) {
                            outside = Some(x.to_owned());
                            break;
                        }
                        todo.extend(reads.get(x).into_iter().flatten().map(String::as_str));
                    }
                    let n = hs.iter().filter(|x| *x == h).count();
                    all_rels += 1;
                    all_rules += n;
                    match outside {
                        None => {
                            final_rels += 1;
                            final_rules += n;
                        }
                        Some(x) => *why.entry(x).or_default() += 1,
                    }
                }
            }
            let mut why: Vec<_> = why.into_iter().collect();
            why.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
            println!("E8 {pack}: {alike} relations made alike by several modules; {} with different rules from more than one module ({:?}); final in their module: {final_rels} of {all_rels} relations, {final_rules} of {all_rules} rules; the first relation outside, most often: {:?}", open.len(), open.iter().take(8).collect::<Vec<_>>(), why.iter().take(6).collect::<Vec<_>>());
        }
        // E9: a module against its scope's header alone. Each scope module lowered again from its statements less its
        // rules (facts and declarations kept), and the module lowered against those, compared with its image.
        {
            let mut header_only: Vec<lower::Module> = Vec::new();
            let (mut same, mut other, mut failed) = (0, 0, 0);
            for (u, members) in units.iter().enumerate() {
                let scope_ix: Vec<usize> = (0..u).filter(|&v| members.iter().any(|&i| reach[i].contains(&units[v][0]))).collect();
                let headers: Vec<&lower::Module> = scope_ix.iter().map(|&v| &header_only[v]).collect();
                let st: Vec<from_tape::Stmt> = members.iter().flat_map(|&i| no_includes(read_all[i].0.clone())).collect();
                let bare: Vec<from_tape::Stmt> = st.iter().filter(|s| !s.has_body()).cloned().collect();
                let name = dfs[members[0]].display().to_string();
                let h = lower::lower_module_in_written(bare, &headers).unwrap_or_else(|e| panic!("{name}: its header alone: {e}"));
                if !scope_ix.is_empty() {
                    match lower::lower_module_in_written(st, &headers) {
                        Ok(m) => {
                            let (a, b) = (module_image::encode(&built[u]).unwrap(), module_image::encode(&m).unwrap());
                            if a.bytes() == b.bytes() {
                                same += 1;
                            } else {
                                other += 1;
                                println!("E9 {pack} {name}: against its scope's headers: {}", differs(&printed(built[u].version()), &printed(m.version())).chars().take(600).collect::<String>());
                            }
                        }
                        Err(e) => {
                            failed += 1;
                            println!("E9 {pack} {name}: against its scope's headers fails: {}", e.to_string().chars().take(300).collect::<String>());
                        }
                    }
                }
                header_only.push(h);
            }
            println!("E9 {pack}: modules with a scope: {same} the same image against the scope's headers alone, {other} not, {failed} failed");
        }
        // E10: a downward analysis as rules. The tail's passes before `KindSets` replayed on the library's link, then
        // the pass; its sets dropped as unread against `ks_gone` of rules over the same input's reads.
        {
            use crate::code_rule_dsl::logic::contexts::{CrossingCycles, Passes};
            use crate::code_rule_dsl::logic::plan::corpus::preparation::lowering_tail as lt;
            use crate::code_rule_dsl::logic::program_image::handles::Lit as HL;
            use crate::code_rule_dsl::logic::program_image::manager::Pipeline;
            let linked = lower::link_library(built.clone()).unwrap().with(None).unwrap();
            let lower::Lowered { sets, rewritten, docof_of_locs, findings, three, modal, .. } = linked.lowered;
            let mut p = Pipeline::new("spike: the tail to kind sets", linked.version);
            p.program(&mut Passes).program(&mut CrossingCycles);
            p.program(&mut lt::ThreeValued { three, modal });
            p.program(&mut lt::Findings { pending: findings, library: None });
            p.program(&mut lt::CoreDecls { docof_of_locs }).program(&mut lt::SpanPatterns { library: None }).program(&mut lt::GrammarNames).program(&mut lt::BuiltinsOut { rewritten });
            let input = Arc::clone(p.current().unwrap());
            p.program(&mut lt::KindSets { sets: sets.clone(), library: None });
            let output = Arc::clone(p.current().unwrap());
            let (in_keys, out_keys): (BTreeSet<String>, BTreeSet<String>) = (input.kindset_map().unwrap().into_keys().collect(), output.kindset_map().unwrap().into_keys().collect());
            let there = |v: &Version, s: &str| v.declares(s) || v.fact_names().iter().any(|f| f == s);
            let cand: BTreeSet<String> = in_keys.iter().cloned().chain(sets.iter().filter(|s| out_keys.contains(*s) || (there(&input, s) && !there(&output, s))).cloned()).collect();
            // The pass takes an unread set out as a relation (undeclared, its facts dropped); it stays a kind set.
            let rust_gone: BTreeSet<String> = cand.iter().filter(|s| there(&input, s) && !there(&output, s)).cloned().collect();
            let mut facts: Vec<String> = cand.iter().map(|s| format!("ks_cand({s:?}).")).collect();
            let mut reads: BTreeSet<(String, String)> = BTreeSet::new();
            for r in input.rules() {
                for l in r.body() {
                    if let HL::Atom(a) | HL::Agg { atom: a, .. } = l.view() {
                        reads.insert((input.rel_name(r.head().rel()).to_owned(), input.rel_name(a.rel()).to_owned()));
                    }
                }
            }
            facts.extend(reads.iter().map(|(h, r)| format!("ks_reads({h:?}, {r:?}).")));
            // A set the program holds as a relation (declared, facts, or rules): only such a set is taken out.
            facts.extend(cand.iter().filter(|s| there(&input, s) || input.rel(s).is_some_and(|r| input.heads(r).next().is_some())).map(|s| format!("ks_rel({s:?}).")));
            let rules = ".decl ks_cand(s: symbol)\n.decl ks_reads(h: symbol, r: symbol)\n.decl ks_live(s: symbol)\n.decl ks_gone(s: symbol)\n.decl ks_rel(s: symbol)\n\
                ks_live(s) :- ks_cand(s), !ks_rel(s).\n\
                ks_live(s) :- ks_cand(s), ks_reads(h, s), h != s, !ks_cand(h).\n\
                ks_live(s) :- ks_cand(s), ks_reads(h, s), h != s, ks_live(h).\n\
                ks_gone(s) :- ks_cand(s), !ks_live(s).\n.output ks_gone\n";
            let rules_gone: BTreeSet<String> = guests_run_with("ksgone", &[("a.py", "x = 1\n")], &format!("{rules}{}\n", facts.join("\n")), |o| {
                o.get("ks_gone").map(|r| r.shown()).unwrap_or_default().into_iter().map(|r| r[0].trim_matches('"').to_owned()).collect()
            });
            for g in rules_gone.difference(&rust_gone).take(3) {
                use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;
                let readers: Vec<String> = input.rules().map(|r| input.show_rule(r)).filter(|t| t.contains(g.as_str())).take(2).map(|t| t.chars().take(160).collect()).collect();
                println!("E10 {pack} {g}: declares {}, states facts {}, fact_names {}, rules {}, in the input's kind sets {}, the output's {}; rules naming it: {readers:?}",
                    input.declares(g), ProgramFields::states_facts(&*input, g), input.fact_names().iter().any(|f| f == g), input.rel(g).is_some_and(|r| input.heads(r).next().is_some()), in_keys.contains(g), out_keys.contains(g));
            }
            println!(
                "E10 {pack}: kind sets: {} candidates over {} reads; the pass drops {}, the rules {}: {}",
                cand.len(), reads.len(), rust_gone.len(), rules_gone.len(),
                if rust_gone == rules_gone { "the same sets".to_owned() } else { format!("only the pass: {:?}; only the rules: {:?}", rust_gone.difference(&rules_gone).take(6).collect::<Vec<_>>(), rules_gone.difference(&rust_gone).take(6).collect::<Vec<_>>()) }
            );
        }
    }
}

/// T6 (decision 0639 §4): a library's modules, their helpers named at each walk's end, linked by name, are the whole
/// list lowered as one module: the same rules, each helper no walk merges aside by its module's digest (`rr_m_`), the
/// same helpers named by content (`rr_h_`), as many of each. No module's image holds its file's path or name
/// (Collisions 9).
#[test]
fn modules_named_by_content_link_to_the_whole_lowering() {
    let lib = crate::code_rule_dsl::logic::rr::library();
    let no_includes = |s: Vec<from_tape::Stmt>| -> Vec<from_tape::Stmt> { s.into_iter().filter(|s| !matches!(s.what(), Some(from_tape::S::Include(_)))).collect() };
    // A rule's line, each module-named helper by its stem and each variable the walk numbered by its stem (a rule's
    // variables are by slot in its template: their names name nothing).
    let plain = |l: &str| -> String {
        l.split_inclusive(|c: char| !(c.is_alphanumeric() || c == '_'))
            .map(|w| {
                let (word, end) = w.split_at(w.len() - w.chars().last().filter(|c| !(c.is_alphanumeric() || *c == '_')).map_or(0, char::len_utf8));
                // A module's helper's digest, in its name or a name made from it (`rr_demand_agg_rr_m_…`).
                let word = if word.contains("rr_m_") && word.len() > 33 && word.as_bytes()[word.len() - 33] == b'_' && word[word.len() - 32..].bytes().all(|b| b.is_ascii_hexdigit()) {
                    &word[..word.len() - 33]
                } else if word.starts_with("rr_") && !word.starts_with("rr_h_") {
                    word.trim_end_matches(|c: char| c.is_ascii_digit())
                } else {
                    word
                };
                format!("{word}{end}")
            })
            .collect()
    };
    for pack in ["python", "rust"] {
        let mut programs = walk_rr(&lib.join("../packs").join(pack));
        programs.sort();
        // The pack's first program that includes the library.
        let srcs: Vec<String> = programs.iter().map(|p| std::fs::read_to_string(p).unwrap()).collect();
        let m = super::modules_of(&srcs.iter().map(String::as_str).collect::<Vec<_>>()).into_iter().find_map(|m| m.ok().flatten()).unwrap_or_else(|| panic!("{pack}: no program includes the library"));
        let paths: Vec<PathBuf> = m.files.iter().map(|(p, _, _)| p.clone()).collect();
        let (modules, members, _) = super::library_modules_planned(&paths).unwrap();
        let whole: Vec<from_tape::Stmt> = members.iter().flatten().flat_map(|p| no_includes(read(p).unwrap().0)).collect();
        for (u, files) in members.iter().enumerate() {
            let image = module_image::encode(&modules[u]).unwrap();
            for path in files {
                let has = |needle: &str| image.bytes().windows(needle.len()).any(|w| w == needle.as_bytes());
                let file = path.file_name().unwrap().to_string_lossy();
                assert!(!has(&path.display().to_string()) && !has(&file), "{pack}: the image of {} holds its path or name", path.display());
            }
        }
        let linked = lower::link_library(modules).unwrap().with(None).unwrap();
        let versions: Vec<&Arc<Version>> = vec![&linked.version];
        let whole = lower::lower_module_in_written(whole, &[]).unwrap();
        let lines = |vs: &[&Arc<Version>]| -> BTreeSet<String> { vs.iter().flat_map(|v| v.rules().map(|r| plain(&v.show_rule(r))).collect::<Vec<_>>()).collect() };
        let helpers = |vs: &[&Arc<Version>], tier: &str| -> BTreeSet<String> { vs.iter().flat_map(|v| v.declared_names()).filter(|r| r.starts_with(tier)).collect() };
        let (a, b) = (lines(&versions), lines(&[whole.version()]));
        assert!(a == b, "{pack}: {} lines and {}; only the link's: {:#?}\nonly the whole's: {:#?}", a.len(), b.len(), a.difference(&b).take(4).collect::<Vec<_>>(), b.difference(&a).take(4).collect::<Vec<_>>());
        assert_eq!(helpers(&versions, "rr_h_"), helpers(&[whole.version()], "rr_h_"), "{pack}: the helpers named by content");
        assert_eq!(helpers(&versions, "rr_m_").len(), helpers(&[whole.version()], "rr_m_").len(), "{pack}: the helpers named by module");
        assert!(!helpers(&versions, "rr_h_").is_empty(), "{pack}: a library makes helpers named by content");
    }
}

/// A printed program's rule lines as a set, each helper named by its module (`rr_m_`) by its stem, and each variable
/// or helper a walk numbered (`$rr_agg17`, `rr_ord3`) by its stem: what decision 0639's module path and the whole
/// lowering of the same text share (T1).
fn rule_lines_names_aside(printed: &str) -> BTreeSet<String> {
    let plain = |l: &str| -> String {
        l.split_inclusive(|c: char| !(c.is_alphanumeric() || c == '_'))
            .map(|w| {
                let (word, end) = w.split_at(w.len() - w.chars().last().filter(|c| !(c.is_alphanumeric() || *c == '_')).map_or(0, char::len_utf8));
                // A module's helper's digest, in its name or a name made from it (`rr_demand_agg_rr_m_…`).
                let word = if word.contains("rr_m_") && word.len() > 33 && word.as_bytes()[word.len() - 33] == b'_' && word[word.len() - 32..].bytes().all(|b| b.is_ascii_hexdigit()) {
                    &word[..word.len() - 33]
                } else if word.starts_with("rr_") && !word.starts_with("rr_h_") {
                    word.trim_end_matches(|c: char| c.is_ascii_digit())
                } else {
                    word
                };
                format!("{word}{end}")
            })
            .collect()
    };
    printed.lines().map(plain).collect()
}

/// T1 (decision 0639): a program that includes the library, read by `parse_version` (its library linked from its
/// modules' images and its own text one more module), is the program its library files and its text lowered as one
/// module make (the whole lowering, which a text read before decision 0639 step 4): the same rules and as many helpers,
/// names aside (`rule_lines_names_aside`), and the same inputs. The first `RERAIL_T1_N` (2) such programs of every pack and of the phase
/// programs; the spike (`spike_0639_programs_linked_from_modules`) runs them all.
#[test]
fn a_program_linked_from_its_modules_is_its_whole_lowering() {
    let lib = crate::code_rule_dsl::logic::rr::library();
    let per: usize = std::env::var("RERAIL_T1_N").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
    let no_includes = |s: Vec<from_tape::Stmt>| -> Vec<from_tape::Stmt> { s.into_iter().filter(|s| !matches!(s.what(), Some(from_tape::S::Include(_)))).collect() };
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(lib.join("../packs")).unwrap().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    dirs.push(crate::source_tree::native().join("src/code_rule_dsl/typing/programs"));
    let mut compared = 0;
    for dir in dirs {
        let mut programs = walk_rr(&dir);
        programs.sort();
        let mut taken = 0;
        for path in programs {
            if taken == per {
                break;
            }
            let Ok(src) = std::fs::read_to_string(&path) else { continue };
            let Ok(Some(m)) = super::modules(&src) else { continue };
            // A program the whole lowering refuses is not compared (the spike's seven, open items).
            let mut all: Vec<from_tape::Stmt> = m.files.iter().flat_map(|(p, _, _)| no_includes(read(p).unwrap().0)).collect();
            all.extend(no_includes(from_tape::RrFile::read(src.as_str()).unwrap().statements()));
            let Ok(whole) = super::lower_statements_version(all).and_then(|v| crate::code_rule_dsl::logic::contexts::passes(&v)) else { continue };
            let linked = parse_version(&src).unwrap_or_else(|e| panic!("{}: the module path: {e}", path.display()));
            let helpers = |v: &Arc<Version>| v.declared_names().into_iter().filter(|r| r.starts_with("rr_")).count();
            // The `.input` list a relation a line: past the inputs a program states, its order is the order the lowering
            // and the tail's steps added them, which the module path adds module by module (a library module's tail
            // its `node_order` before a later module's lowering its `tape`).
            let lines = |v: &Arc<Version>| -> BTreeSet<String> {
                rule_lines_names_aside(&print_version(v).unwrap()).into_iter().flat_map(|l| match l.strip_prefix(".input ") {
                    Some(rest) => rest.split(", ").map(|r| format!(".input {}", r.trim())).collect::<Vec<_>>(),
                    None => vec![l],
                }).collect()
            };
            let (a, b) = (lines(&linked), lines(&whole));
            assert!(a == b, "{}: {} lines and {}; only the module path's: {:#?}\nonly the whole lowering's: {:#?}", path.display(), a.len(), b.len(), a.difference(&b).take(6).collect::<Vec<_>>(), b.difference(&a).take(6).collect::<Vec<_>>());
            assert_eq!(helpers(&linked), helpers(&whole), "{}: the helpers", path.display());
            taken += 1;
            compared += 1;
        }
    }
    assert!(compared > 10, "{compared} programs compared");
}

/// Decision 0639 §4: a program's union extends its library's, which is made once for its modules: two programs of one
/// library's modules make it once, and each is linked from it with only its own module added.
#[test]
fn programs_of_one_library_share_its_union() {
    let mut programs = walk_rr(&crate::code_rule_dsl::logic::rr::library().join("../packs/python"));
    programs.sort();
    let mut by_files: std::collections::BTreeMap<Vec<PathBuf>, Vec<String>> = Default::default();
    for path in programs {
        let Ok(src) = std::fs::read_to_string(&path) else { continue };
        let Ok(Some(m)) = super::modules(&src) else { continue };
        by_files.entry(m.files.into_iter().map(|(p, _, _)| p).collect()).or_default().push(src);
    }
    let two = by_files.into_values().find(|s| s.len() >= 2).expect("two programs of one library's modules");
    let before = super::library_unions_made();
    parse_version(&two[0]).unwrap();
    assert_eq!(super::library_unions_made(), before + 1, "the first program makes its library's union");
    parse_version(&two[1]).unwrap();
    assert_eq!(super::library_unions_made(), before + 1, "the second is linked onto it");
}

/// T9 (decision 0639 §4): a module depends on its scope's interface, not its scope's rules. Each module of every
/// pack's library and of the phase programs, lowered against its scope's modules lowered from their statements less
/// their rules (their headers), is its image: the declarations its clauses' lowering makes it keeps as its own.
#[test]
fn every_module_lowers_alike_against_its_scopes_headers() {
    let lib = crate::code_rule_dsl::logic::rr::library();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(lib.join("../packs")).unwrap().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    dirs.push(crate::source_tree::native().join("src/code_rule_dsl/typing/programs"));
    let no_includes = |s: Vec<from_tape::Stmt>| -> Vec<from_tape::Stmt> { s.into_iter().filter(|s| !matches!(s.what(), Some(from_tape::S::Include(_)))).collect() };
    // The library files every pack's programs include, planned once: a module shared by packs is lowered once.
    let mut paths: BTreeSet<PathBuf> = BTreeSet::new();
    for dir in dirs {
        for path in walk_rr(&dir) {
            let Ok(src) = std::fs::read_to_string(&path) else { continue };
            if let Ok(Some(m)) = super::modules(&src) {
                paths.extend(m.files.into_iter().map(|(p, _, _)| p));
            }
        }
    }
    let plan = super::planned_library(&paths.into_iter().collect::<Vec<_>>()).unwrap();
    let at: std::collections::HashMap<&str, usize> = plan.iter().enumerate().map(|(i, m)| (m.key.as_str(), i)).collect();
    let (mut compared, mut differ) = (0, Vec::new());
    let mut headers: Vec<lower::Module> = Vec::with_capacity(plan.len());
    for m in &plan {
        let statements: Vec<from_tape::Stmt> = m.files.lines().filter(|f| !f.is_empty()).flat_map(|f| no_includes(read(Path::new(f)).unwrap().0)).collect();
        let bare: Vec<from_tape::Stmt> = statements.iter().filter(|s| !s.has_body()).cloned().collect();
        let scope: Vec<&lower::Module> = m.scope.split(',').filter(|k| !k.is_empty()).map(|k| &headers[at[k]]).collect();
        let name = m.files.lines().next().unwrap_or_default().to_owned();
        let header = lower::lower_module_in_written(bare, &scope).unwrap_or_else(|e| panic!("{name}: its header alone: {e}"));
        if !scope.is_empty() {
            let alone = lower::lower_module_in_written(statements, &scope).unwrap_or_else(|e| panic!("{name}: against its scope's headers: {e}"));
            let image = m.module().unwrap();
            compared += 1;
            if module_image::encode(&image).unwrap().bytes() != module_image::encode(&alone).unwrap().bytes() {
                let (a, b) = (rule_lines_names_aside(&print_version(image.version()).unwrap()), rule_lines_names_aside(&print_version(alone.version()).unwrap()));
                differ.push(format!("{name}: only its image's: {:?}; only against the headers: {:?}", a.difference(&b).take(4).collect::<Vec<_>>(), b.difference(&a).take(4).collect::<Vec<_>>()));
            }
        }
        headers.push(header);
    }
    assert!(compared > 50, "{compared} modules compared");
    assert!(differ.is_empty(), "{} of {compared} modules differ against their scope's headers:\n{}", differ.len(), differ.join("\n"));
}

/// T8 (decision 0639 §4): the link is a union with nothing to decide, so two modules that give one relation two
/// declarations (`link_conflict`), or one helper's name two sets of rules (`helper_conflict`), fail it; modules that
/// agree link. That no library has either is T1 and T2: every program links.
#[test]
fn a_link_of_modules_that_clash_fails() {
    let module = |src: &str| lower::lower_module_in_written(from_tape::RrFile::read(src).unwrap().statements(), &[]).unwrap_or_else(|e| panic!("{e}\n{src}"));
    let agree = super::link_library_versions(vec![module(".decl shared(x: symbol)\nshared(\"a\").\n"), module(".decl shared(x: symbol)\nshared(\"b\").\n")]);
    agree.expect("one declaration in two modules");
    let clash = super::link_library_versions(vec![module(".decl shared(x: symbol)\n"), module(".decl shared(x: number)\n")]).expect_err("two declarations");
    assert!(clash.to_string().contains("link_conflict") && clash.to_string().contains("shared"), "{clash}");
    // Two of the Python library's modules that make helpers: one's renamed to the other's name, they are two helpers
    // under one name.
    let mut programs = walk_rr(&crate::code_rule_dsl::logic::rr::library().join("../packs/python"));
    programs.sort();
    let m = programs.iter().find_map(|p| super::modules(&std::fs::read_to_string(p).ok()?).ok().flatten()).expect("a program that includes the library");
    let paths: Vec<PathBuf> = m.files.into_iter().map(|(p, _, _)| p).collect();
    let mut modules: Vec<lower::Module> = super::library_modules(&paths).unwrap().into_iter().filter(|m| !lower::helper_names(m).is_empty()).collect();
    assert!(modules.len() >= 2, "two modules make helpers");
    let b = modules.pop().unwrap();
    let a = modules.remove(0);
    let ha = lower::helper_names(&a)[0].clone();
    let hb = lower::helper_names(&b).iter().find(|h| !lower::helper_names(&a).contains(h)).cloned().expect("a helper of its own");
    let b = lower::with_helper_renamed(b, &hb, &ha).unwrap();
    let clash = super::link_library_versions(vec![a, b]).expect_err("one name, two helpers");
    assert!(clash.to_string().contains("helper_conflict"), "{clash}");
}

/// Decision 0639 §4, stage B: a library's link is a union over its modules' versions, as their images hold them: each
/// part is its module's version, no rule copied, and the union reads as its fold (the same rules and digest). An edit
/// stacks on it as on any version, and a fold of the edit is the edit.
#[test]
fn a_linked_library_is_a_union_over_its_modules_images() {
    use crate::code_rule_dsl::logic::program_image::Builder;
    let mut programs = walk_rr(&crate::code_rule_dsl::logic::rr::library().join("../packs/python"));
    programs.sort();
    let m = programs.iter().find_map(|p| super::modules(&std::fs::read_to_string(p).ok()?).ok().flatten()).expect("a program that includes the library");
    let paths: Vec<PathBuf> = m.files.into_iter().map(|(p, _, _)| p).collect();
    let modules = super::library_modules(&paths).unwrap();
    let held: Vec<Arc<Version>> = modules.iter().map(|m| Arc::clone(m.version())).collect();
    let linked = lower::link_library(modules).unwrap().with(None).unwrap();
    let v = &linked.version;
    let (header, parts) = v.union_parts().expect("the link is a union");
    assert_eq!(header.size(), 0, "its header holds no rules");
    let parts: Vec<_> = parts.collect();
    assert_eq!(parts.len(), held.len(), "a part a module");
    assert!(parts.iter().zip(&held).all(|(p, m)| Arc::ptr_eq(&p.version, m)), "each part is its module's version, uncopied");
    let folded = v.fold().unwrap();
    assert_eq!((folded.size(), folded.digest()), (v.size(), v.digest()), "the union and its fold: {}", crate::code_rule_dsl::logic::program_image::differences(v, &folded));
    let shown = |x: &Version| x.rules().map(|r| x.show_rule(r)).collect::<Vec<_>>();
    assert_eq!(shown(v), shown(&folded), "its rules, in order");
    // An edit of it: its first rule left out.
    let first = v.rules().next().unwrap().id();
    let mut b = Builder::on(v);
    b.remove(first);
    let (edited, _) = b.finish().unwrap();
    assert_eq!(edited.size(), v.size() - 1);
    let refolded = edited.fold().unwrap();
    assert_eq!((refolded.size(), refolded.digest()), (edited.size(), edited.digest()), "an edit of the union and its fold");
    // A view of it numbers as it does, so a rule added on the view is past every rule of the union's.
    assert_eq!(edited.rule_end(), v.rule_end(), "a view of a union numbers its rules as the union");
    let mut b = Builder::on(&edited);
    let added = b.copy(v, v.rule(first).unwrap());
    let (over, _) = b.finish().unwrap();
    assert!(added.0 >= v.rule_end() && over.rule(added).is_some(), "a rule added on a view of a union is its own");
    assert!(over.rules().all(|r| over.rule(r.id()).is_some()), "each rule of an edit of a union is found by its id");
    assert_eq!(over.digest(), v.digest(), "the rule left out and added back");
}

/// T7 (decision 0639, Collisions 4): a helper's content name holds every header row that names it, its facts, and
/// whether it is an input or an output: two helpers of the same stem, columns and rules are one name, and two that
/// differ only in one of these are two.
#[test]
fn a_helpers_content_name_holds_every_field_that_names_it() {
    let names = |src: &str| -> (String, String) {
        let v = parse(src).unwrap_or_else(|e| panic!("{e}\n{src}"));
        let (names, _) = lower::content_names(&v, &[("agg".to_owned(), 91), ("agg".to_owned(), 92)], &[("rr_agg91".to_owned(), "agg".to_owned(), 1), ("rr_agg92".to_owned(), "agg".to_owned(), 1)]).unwrap();
        (names["rr_agg91"].clone(), names["rr_agg92"].clone())
    };
    let program = |second: &str, outputs: &str, facts: &str| format!(".decl e(x: number)\ne(1).\n.decl rr_agg91(x: number)\nrr_agg91(x) :- e(x).\n.decl rr_agg92(x: number){second}\nrr_agg92(x) :- e(x).\n{facts}.output {outputs}\n");
    let (a, b) = names(&program("", "rr_agg91, rr_agg92", ""));
    assert_eq!(a, b, "the same stem, columns and rules");
    assert!(a.starts_with("rr_h_agg_") && a.len() == "rr_h_agg_".len() + 32, "{a}");
    for (what, src) in [
        ("a qualifier", program(" inline", "rr_agg91, rr_agg92", "")),
        ("an output", program("", "rr_agg91", "")),
        ("a fact", program("", "rr_agg91, rr_agg92", "rr_agg92(5).\n")),
    ] {
        let (a, b) = names(&src);
        assert_ne!(a, b, "two helpers differing in {what} are two");
    }
}

/// Collisions 3 (decision 0639): a template's bytes are length-prefixed, and read back as its rule's nodes, each kind
/// and size; two rules of other templates write other bytes.
#[test]
fn a_templates_bytes_read_back_as_its_rule() {
    let v = parse(".decl e(x: number, s: symbol)\ne(1, \"a\").\n.decl f(x: number)\nf(x) :- e(x, \"a\"), x < 100000000000.\n.decl g(s: symbol)\ng(s) :- e(_, s), !f(3).\n.output f, g\n").unwrap();
    let mut seen = BTreeSet::new();
    for r in v.rules() {
        let bytes = lower::template_bytes(&v, r);
        let nodes = lower::template_nodes(&bytes).expect("a template's bytes");
        let ns = r.nodes();
        assert_eq!(nodes.len(), (ns.end - ns.start - 1) as usize, "{}", v.show_rule(r));
        for (k, (kind, size, _)) in (ns.start + 1..).zip(&nodes) {
            let (kd, sz, _) = r.image().node(k);
            assert_eq!((*kind, *size), (kd, sz), "{}: node {k}", v.show_rule(r));
        }
        assert!(seen.insert((v.rel_name(r.head().rel()).to_owned(), bytes)), "{}", v.show_rule(r));
    }
    assert!(lower::template_nodes(&[1, 0, 0, 0, 0, 9, 0, 0, 0]).is_none(), "a length past the end");
}

/// Every pack's library and the phase programs' (decision 0639 §6a's tests): the pack, its library's plan, and each
/// program with its text and its library files.
#[allow(clippy::type_complexity)]
fn tail_libraries() -> Vec<(String, Vec<super::PlannedModule>, Vec<(PathBuf, String, Vec<PathBuf>)>)> {
    let lib = crate::code_rule_dsl::logic::rr::library();
    let mut packs: Vec<(String, PathBuf)> = std::fs::read_dir(lib.join("../packs")).unwrap().flatten().filter(|e| e.path().is_dir()).map(|e| (e.file_name().to_string_lossy().into_owned(), e.path())).collect();
    packs.push(("phases".into(), crate::source_tree::native().join("src/code_rule_dsl/typing/programs")));
    packs.sort();
    let mut out = Vec::new();
    for (pack, dir) in packs {
        let mut programs = walk_rr(&dir);
        programs.sort();
        let mut paths: BTreeSet<PathBuf> = BTreeSet::new();
        let mut texts = Vec::new();
        for path in programs {
            let Ok(src) = std::fs::read_to_string(&path) else { continue };
            if let Ok(Some(m)) = super::modules(&src) {
                let files: Vec<PathBuf> = m.files.into_iter().map(|(p, _, _)| p).collect();
                paths.extend(files.iter().cloned());
                texts.push((path, src, files));
            }
        }
        if paths.is_empty() {
            continue;
        }
        let plan = super::planned_library(&paths.into_iter().collect::<Vec<_>>()).unwrap_or_else(|e| panic!("{pack}: {e}"));
        out.push((pack, plan, texts));
    }
    out
}

/// A version's lines with names aside (`rule_lines_names_aside`), each `.input` and `.output` list a line a relation
/// (a module states its own), and variables and columns renamed by place (`alpha`).
fn tail_lines(v: &Arc<Version>) -> BTreeSet<String> {
    rule_lines_names_aside(&print_version(v).unwrap())
        .into_iter()
        .flat_map(|l| match l.split_once(' ') {
            Some((d @ (".input" | ".output"), rest)) => rest.split(", ").map(|r| format!("{d} {}", r.trim())).collect::<Vec<_>>(),
            _ => vec![l],
        })
        .map(|l| alpha(&l))
        .collect()
}

/// The relation a printed line is of: a rule's head, a declaration's or a list's relation.
fn line_relation(l: &str) -> String {
    let l = l.trim_start_matches(".decl ").trim_start_matches(".input ").trim_start_matches(".output ");
    l.split(['(', ' ', ',']).next().unwrap_or_default().trim().to_owned()
}

/// T10 (decision 0639 §6a, E10; decision 0662 §2): the kind set rules (`rr/kind-sets.rerail`, over the image's view
/// `rr/view.rerail`) take out what `KindSets` takes out, every pack: the same sets, the same declarations undeclared
/// and the same facts dropped. Each library is tailed to just before `KindSets`, its view stated from that image (its
/// relations named by pack, so one run holds all packs) and the rules run once; the pass runs on the same image.
#[test]
fn kind_set_rules_take_out_what_the_pass_takes_out() {
    use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;
    use crate::code_rule_dsl::logic::program_image::handles::Lit as HL;
    // The test's store bound on its thread, which the tails on the pool's threads carry (T12).
    let _store = crate::code_rule_dsl::logic::plan::corpus::use_store(crate::code_rule_dsl::logic::plan::corpus::tape_cache_path().expect("the test's store"));
    let libraries = tail_libraries();
    let at = super::TAIL.iter().position(|s| *s == super::TailStep::KindSets).unwrap();
    // Per pack: the view's facts of the image before the pass, and what the pass takes out (sets, undeclared, facts).
    type Out = (Vec<String>, [BTreeSet<String>; 3]);
    let per_pack: Vec<Out> = crate::code_rule_dsl::logic::profile::on_cores(|| {
        use rayon::prelude::*;
        libraries.par_iter().map(|(pack, plan, _)| {
            let l = lower::link_library(plan.iter().map(|m| m.module().unwrap()).collect()).unwrap().with(None).unwrap();
            let sets = l.lowered.sets.clone();
            let input = super::tail_steps(l.version, false, l.lowered, None, &super::TAIL[..at]).unwrap_or_else(|e| panic!("{pack}: the tail before kind sets: {e}"));
            let mut p = crate::code_rule_dsl::logic::program_image::manager::Pipeline::new("T10: kind sets", Arc::clone(&input));
            p.program(&mut crate::code_rule_dsl::logic::plan::corpus::preparation::lowering_tail::KindSets { sets, library: None });
            let output = Arc::clone(p.current().unwrap_or_else(|e| panic!("{pack}: kind sets: {e}")));
            let n = |rel: &str| format!("{:?}", format!("{pack}/{rel}"));
            let sets = output.kindset_map().unwrap();
            let mut facts: Vec<String> = sets.keys().map(|s| format!("img_kindset({}).", n(s))).collect();
            for r in input.rules() {
                let id = n(&format!("{:?}", r.id()));
                facts.push(format!("img_rule({id}, {}).", n(input.rel_name(r.head().rel()))));
                for (i, l) in r.body().enumerate() {
                    let (what, a) = match l.view() {
                        HL::Atom(a) => ("atom", a),
                        HL::Agg { atom: a, .. } => ("agg", a),
                        _ => continue,
                    };
                    facts.push(format!("img_lit({id}, {i}, {what:?}, {}, {}).", n(input.rel_name(a.rel())), u8::from(a.neg())));
                }
            }
            for d in ProgramFields::declared_names(&*input) {
                let cols = ProgramFields::columns_of(&*input, &d).unwrap_or_default().iter().map(|(c, t)| format!("{c}:{t}")).collect::<Vec<_>>().join(",");
                facts.push(format!("img_decl({}, {cols:?}).", n(&d)));
            }
            facts.extend(ProgramFields::fact_names(&*input).into_iter().filter(|f| ProgramFields::states_facts(&*input, f)).map(|f| format!("img_facts({}).", n(&f))));
            // What the pass took out of the kind sets: held as a relation before, not after.
            let named = |s: &String| format!("{pack}/{s}");
            let undeclared: BTreeSet<String> = sets.keys().filter(|s| ProgramFields::declares(&*input, s) && !ProgramFields::declares(&*output, s)).map(named).collect();
            let dropped: BTreeSet<String> = sets.keys().filter(|s| ProgramFields::states_facts(&*input, s) && !ProgramFields::states_facts(&*output, s)).map(named).collect();
            // A set taken out is undeclared and its facts dropped: stated before, not after.
            let stated = |v: &Version, s: &str| ProgramFields::declares(v, s) || ProgramFields::states_facts(v, s);
            let out: BTreeSet<String> = sets.keys().filter(|s| stated(&input, s) && !stated(&output, s)).map(named).collect();
            (facts, [out, undeclared, dropped])
        }).collect()
    });
    let facts: Vec<&String> = per_pack.iter().flat_map(|(f, _)| f).collect();
    // The rules pass's program, its view's rows stated after it.
    let program = std::fs::read_to_string(crate::code_rule_dsl::logic::rr::library().join("rr/kind-sets.rerail")).unwrap();
    let text = format!("{program}\n{}\n", facts.iter().map(|f| f.as_str()).collect::<Vec<_>>().join("\n"));
    let rules: [BTreeSet<String>; 3] = guests_run_with("kindsets", &[("a.py", "x = 1\n")], &text, |o| {
        ["img_kindset_out", "img_undeclare", "img_drop_facts"].map(|rel| o.get(rel).map(|r| r.shown()).unwrap_or_default().into_iter().map(|r| r[0].trim_matches('"').to_owned()).collect())
    });
    let mut failures = Vec::new();
    for (i, what) in ["sets taken out", "declarations undeclared", "facts dropped"].iter().enumerate() {
        let pass: BTreeSet<&String> = per_pack.iter().flat_map(|(_, o)| &o[i]).collect();
        let by_rules: BTreeSet<&String> = rules[i].iter().collect();
        if pass != by_rules {
            failures.push(format!("{what}: only the pass {:?}; only the rules {:?}", pass.difference(&by_rules).take(8).collect::<Vec<_>>(), by_rules.difference(&pass).take(8).collect::<Vec<_>>()));
        }
    }
    let taken: usize = per_pack.iter().map(|(_, o)| o[0].len()).sum();
    assert!(taken > 0, "no pack's kind sets were taken out: the test compares nothing");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Each module of `plan` tailed in plan order (`rr::tail_module`), each over its scope's tails.
fn tailed_modules(plan: &[super::PlannedModule]) -> Vec<lower::Module> {
    let at: std::collections::HashMap<&str, usize> = plan.iter().enumerate().map(|(i, m)| (m.key.as_str(), i)).collect();
    let mut tailed: Vec<lower::Module> = Vec::with_capacity(plan.len());
    for m in plan {
        let mut scope: Vec<usize> = m.scope.split(',').filter(|k| !k.is_empty()).map(|k| at[k]).collect();
        scope.sort();
        let lowered: Vec<lower::Module> = scope.iter().map(|&j| plan[j].module().unwrap()).collect();
        let tails: Vec<lower::Module> = scope.iter().map(|&j| tailed[j].clone()).collect();
        let name = m.files.lines().next().unwrap_or_default().to_owned();
        tailed.push(super::tail_module(&lowered, &tails, m.module().unwrap()).unwrap_or_else(|e| panic!("{name}: its tail: {e}")));
    }
    tailed
}

/// T11 (decision 0639 §6a, E12 and E12b): every library's modules, each tailed at module time over its scope's tails
/// (`rr::tail_module`), unite to the library's tail, core.rerail's declarations made over the union as a program makes
/// them (`rr::program_steps`). A line only the union has is a list setting a module states for
/// its own (a context's `.pass`, the demand's `.pragma`), or of a relation no rule of the union reads but its own: a
/// host's copy a context leaves where its other declaration exports it (Vue's `script`), which `Prune` drops. The
/// module steps over the union leave it as it was.
#[test]
fn modules_tailed_at_module_time_unite_to_the_librarys_tail() {
    let libraries = tail_libraries();
    let failures: Vec<String> = crate::code_rule_dsl::logic::profile::on_cores(|| {
        use rayon::prelude::*;
        libraries.par_iter().flat_map(|(pack, plan, _)| {
            let mut bad = Vec::new();
            let modules: Vec<lower::Module> = plan.iter().map(|m| m.module().unwrap()).collect();
            let linked = lower::link_library(modules).unwrap().with(None).unwrap();
            let whole = tail_lines(&super::tail_steps(linked.version, false, linked.lowered, None, &super::module_steps()).unwrap());
            let tailed = tailed_modules(plan);
            // The union as a program reads it: core.rerail's declarations of what none of its modules declares made
            // over it first (`program_steps`), which the modules' tails leave to it.
            let link = || lower::link_library(tailed.clone()).unwrap().with(None).unwrap();
            let united = {
                let l = link();
                super::tail_steps(l.version, false, l.lowered, None, &[super::TailStep::CoreDecls]).unwrap()
            };
            let union = tail_lines(&united);
            let only_whole: Vec<&String> = whole.difference(&union).take(4).collect();
            if !only_whole.is_empty() {
                bad.push(format!("{pack}: lines only the library's tail has: {only_whole:#?}"));
            }
            let list_setting = |l: &str| l.starts_with(".pass ") || l.starts_with(".pragma \"magic-transform\"");
            let reads: BTreeSet<(String, String)> = united.rules().flat_map(|r| {
                let h = united.rel_name(r.head().rel()).to_owned();
                r.reads().map(|a| (h.clone(), united.rel_name(a.rel()).to_owned())).collect::<Vec<_>>()
            }).collect();
            // The relations of the lines only the union has, less each that a rule of another relation reads, to a
            // fixed point: what is left nothing else reads, which `Prune` drops.
            let extra: Vec<&String> = union.difference(&whole).filter(|l| !list_setting(l)).collect();
            let mut dead: BTreeSet<String> = extra.iter().map(|l| line_relation(l)).collect();
            loop {
                let read: BTreeSet<String> = dead.iter().filter(|r| reads.iter().any(|(h, b)| b == *r && h != *r && !dead.contains(h))).cloned().collect();
                if read.is_empty() {
                    break;
                }
                dead.retain(|r| !read.contains(r));
            }
            let live: Vec<&&String> = extra.iter().filter(|l| !dead.contains(&line_relation(l))).take(4).collect();
            if !live.is_empty() {
                bad.push(format!("{pack}: lines only the union has, of relations its rules read: {live:#?}"));
            }
            let again = super::tail_steps(Arc::clone(&united), false, link().lowered, None, &super::module_steps()).unwrap();
            let twice = tail_lines(&again);
            if twice != union {
                bad.push(format!("{pack}: the module steps over the union: {:?} gone, {:?} made", union.difference(&twice).take(3).collect::<Vec<_>>(), twice.difference(&union).take(3).collect::<Vec<_>>()));
            }
            bad
        }).collect()
    });
    assert!(libraries.len() > 10, "{} libraries", libraries.len());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A program's version pruned (`Prune`), as its plan prunes it.
fn pruned(v: Arc<Version>) -> Arc<Version> {
    use crate::code_rule_dsl::logic::program_image::manager::Pipeline;
    let mut p = Pipeline::new("T12: pruned", v);
    p.program(&mut crate::code_rule_dsl::logic::plan::corpus::preparation::Prune::default());
    p.finish().unwrap().0
}

/// What two versions of one program differ by, as T12 compares them: their lines, a made pass's `.pass` among them
/// (stated once, `contexts::PassesOnce`), and their contexts as `contexts::declared` reads them.
fn program_difference(a: &Arc<Version>, b: &Arc<Version>) -> Option<String> {
    let (la, lb) = (tail_lines(a), tail_lines(b));
    let only_b: Vec<&String> = lb.difference(&la).take(12).collect();
    let only_a: Vec<&String> = la.difference(&lb).take(4).collect();
    // Each context's exports that the program holds: an export's instance derived, or the export's own rules where
    // the context is not made yet.
    let contexts = |v: &Arc<Version>| {
        let derived = |x: &str| v.rel(x).is_some_and(|id| v.heads(id).next().is_some());
        crate::code_rule_dsl::logic::contexts::declared(v)
            .map(|d| d.contexts.into_iter().map(|(n, c)| {
                let held: BTreeSet<String> = c.exports.into_iter().filter(|e| derived(&crate::code_rule_dsl::logic::contexts::instance(&n, e)) || (!c.made && derived(e))).collect();
                (n, c.host, held)
            }).filter(|(_, _, held)| !held.is_empty()).collect::<Vec<_>>())
            .map_err(|e| e.to_string())
    };
    let (ca, cb) = (contexts(a), contexts(b));
    // The rules of `b` that read the relation of its first line `a` lacks: what keeps it.
    let readers: Vec<String> = only_b.first().map(|l| line_relation(l)).map(|rel| b.rules().filter(|r| b.rel_name(r.head().rel()) != rel && r.reads().any(|x| b.rel_name(x.rel()) == rel)).map(|r| b.show_rule(r)).take(3).collect()).unwrap_or_default();
    (!only_a.is_empty() || !only_b.is_empty() || ca != cb).then(|| format!("only the whole tail's {only_a:#?}; only the product's {only_b:#?}{}; read in the product by {readers:#?}", if ca != cb { format!("; contexts {ca:?} / {cb:?}") } else { String::new() }))
}

/// T12 (decision 0639 §6a, E13 on the product path): every pack program's version (`rr::parse_version`), linked from
/// its library's tailed modules and its own module tailed, then the downward steps and the steps after them, is the
/// version of the whole library and its own module tailed by `TAIL`, both pruned. A program with nothing to output
/// is compared unpruned. The texts' graphs come from one run of phase 0, as a run's texts' do (`phase0::graphs`).
#[test]
fn programs_linked_from_tailed_modules_plan_as_whole_tails_do() {
    let per: usize = 6;
    // Planned on the test's thread, in its store, which the programs' tasks are bound to: they find its graphs and
    // images there and run no phase 0 of their own (two runs at once in one store wait on its lease).
    let _store = crate::code_rule_dsl::logic::plan::corpus::use_store(crate::code_rule_dsl::logic::plan::corpus::tape_cache_path().expect("the test's store"));
    let libraries = tail_libraries();
    let seeds: Vec<crate::code_rule_dsl::logic::plan::corpus::phase0::Seed> = libraries.iter().flat_map(|(_, _, texts)| texts.iter().take(per)).filter_map(|(_, src, _)| super::text_seed(src).unwrap()).collect();
    crate::code_rule_dsl::logic::plan::corpus::phase0::graphs(&seeds).unwrap();
    let failures: Vec<String> = crate::code_rule_dsl::logic::profile::on_cores(|| {
        use rayon::prelude::*;
        libraries.par_iter().flat_map(|(pack, _, texts)| texts.par_iter().take(per).filter_map(|(path, src, _)| {
            // Today's path over the product's own plan: its library's modules as lowered and its own module, linked,
            // then all of `TAIL`.
            let (plan, own) = match super::text_plan(src) {
                Ok(planned) => planned?,
                Err(e) => return Some(format!("{pack}: {}: its plan: {e}", path.display())),
            };
            let reference = plan.iter().map(|m| m.module()).collect::<Result<Vec<lower::Module>, _>>().and_then(|ms| {
                let scope: Vec<&lower::Module> = ms.iter().collect();
                let own = lower::lower_module_in_written(own, &scope)?;
                let linked = lower::link_library(ms.clone())?.with(Some(own))?;
                super::tail_steps(linked.version, false, linked.lowered, None, &super::TAIL)
            }).and_then(|v| crate::code_rule_dsl::logic::contexts::passes(&v));
            let product = super::parse_version(src);
            match (reference, product) {
                (Ok(a), Ok(b)) => {
                    let (a, b) = if a.outputs().next().is_some() { (pruned(a), pruned(b)) } else { (a, b) };
                    program_difference(&a, &b).map(|d| format!("{pack}: {}: {d}", path.display()))
                }
                (Err(a), Err(b)) if a.to_string() == b.to_string() => None,
                (a, b) => Some(format!("{pack}: {}: {:?} / {:?}", path.display(), a.err().map(|e| e.to_string()), b.err().map(|e| e.to_string()))),
            }
        }).collect::<Vec<_>>()).collect()
    });
    assert!(failures.is_empty(), "{} programs:\n{}", failures.len(), failures.join("\n"));
}

/// T13 (decision 0639 §6a, E16): over every pack program's link, each step after the downward ones that is not an
/// across step changes only the relations of the edit so far: those the downward steps or the program's own module
/// changed, and those an across step before it changed. So it runs on that edit alone. An across step (`Place::Across`)
/// rewrites what other modules hold by its readers, per program (0639's Collisions 6): its changes join the edit.
#[test]
fn the_steps_after_the_downward_ones_change_only_what_those_and_the_own_module_change() {
    use std::collections::BTreeMap;
    let per: usize = 6;
    // Planned on the test's thread, in its store, as T12's are.
    let _store = crate::code_rule_dsl::logic::plan::corpus::use_store(crate::code_rule_dsl::logic::plan::corpus::tape_cache_path().expect("the test's store"));
    let libraries = tail_libraries();
    // The programs whose every step's changes are read off, whose checks are of their own components.
    let read_off_programs = std::sync::atomic::AtomicUsize::new(0);
    let failures: Vec<String> = crate::code_rule_dsl::logic::profile::on_cores(|| {
        use rayon::prelude::*;
        libraries.par_iter().flat_map(|(pack, _, texts)| texts.par_iter().take(per).filter_map(|(path, src, files)| {
            // The files' graphs are `tail_libraries`'s: planning them runs no phase 0 again.
            let ms = match super::library_modules_planned(files) {
                Ok((ms, _, _)) => ms,
                Err(e) => return Some(format!("{pack}: {}: its library's plan: {e}", path.display())),
            };
            let own: Vec<from_tape::Stmt> = from_tape::RrFile::read(src.as_str()).unwrap().statements().into_iter().filter(|s| !matches!(s.what(), Some(from_tape::S::Include(_)))).collect();
            let scope: Vec<&lower::Module> = ms.iter().collect();
            let own = lower::lower_module_in_written(own, &scope).ok()?;
            let own_heads: BTreeSet<String> = own.version().rules().map(|r| own.version().rel_name(r.head().rel()).to_owned()).collect();
            let fresh = || lower::link_library(ms.clone()).unwrap().with(Some(own.clone())).unwrap();
            let m = super::tail_steps(fresh().version, true, fresh().lowered, None, &super::module_steps()).ok()?;
            let d = super::tail_steps(Arc::clone(&m), true, fresh().lowered, None, &super::downward_steps()).ok()?;
            let after: Vec<super::TailStep> = super::program_steps().into_iter().filter(|s| s.place() != super::Place::Downward).collect();
            let heads = |v: &Arc<Version>| -> BTreeMap<String, BTreeSet<String>> {
                let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
                for l in tail_lines(v) {
                    out.entry(line_relation(&l)).or_default().insert(l);
                }
                out
            };
            let changed = |x: &BTreeMap<String, BTreeSet<String>>, y: &BTreeMap<String, BTreeSet<String>>| -> BTreeSet<String> { x.keys().chain(y.keys()).filter(|k| x.get(*k) != y.get(*k)).cloned().collect() };
            let (hm, hd) = (heads(&m), heads(&d));
            let mut edit: BTreeSet<String> = changed(&hm, &hd).into_iter().chain(own_heads).collect();
            let (mut v, mut hv) = (d, hd);
            for step in after {
                let next = super::tail_steps(Arc::clone(&v), true, fresh().lowered, None, &[step]).ok()?;
                let hn = heads(&next);
                let made = changed(&hv, &hn);
                if step.place() == super::Place::Across {
                    edit.extend(made);
                } else if let Some(r) = made.iter().find(|r| !edit.contains(*r)) {
                    let empty = BTreeSet::new();
                    let (was, now) = (hv.get(r).unwrap_or(&empty), hn.get(r).unwrap_or(&empty));
                    return Some(format!("{pack}: {}: {step:?} changes {:?} outside the edit; {r}'s lines gone {:?}, new {:?}", path.display(), made.iter().filter(|r| !edit.contains(*r)).take(6).collect::<Vec<_>>(), was.difference(now).take(2).collect::<Vec<_>>(), now.difference(was).take(2).collect::<Vec<_>>()));
                }
                (v, hv) = (next, hn);
            }
            // The program's checks are of the components its steps changed, read off the versions they made
            // (`rr::changed_since`, step by step): where a step's are read off, they cover every relation whose rules
            // (by their contents' hashes) or declaration it changed. A step that made a new base is read as changing
            // everything: the program is then checked whole.
            let content = |v: &Version| -> BTreeMap<String, (Vec<u64>, Option<u64>)> {
                let mut out: BTreeMap<String, (Vec<u64>, Option<u64>)> = BTreeMap::new();
                for r in v.rules() {
                    out.entry(v.rel_name(r.head().rel()).to_owned()).or_default().0.push(r.hash());
                }
                for d in v.decls() {
                    out.entry(v.text(d.name()).to_owned()).or_default().1 = Some(d.hash());
                }
                out.values_mut().for_each(|(h, _)| h.sort_unstable());
                out
            };
            let mut whole = false;
            let mut at = (Arc::clone(&m), content(&m));
            for step in super::program_steps() {
                let next = super::tail_steps(Arc::clone(&at.0), true, fresh().lowered, None, &[step]).ok()?;
                let cn = content(&next);
                if let Some(read_off) = super::changed_since(&at.0, &next) {
                    let made: BTreeSet<&String> = at.1.keys().chain(cn.keys()).filter(|r| at.1.get(*r) != cn.get(*r)).collect();
                    if let Some(r) = made.iter().find(|r| !read_off.contains(**r)) {
                        return Some(format!("{pack}: {}: {step:?} changes {r}, which the version it made does not show", path.display()));
                    }
                } else {
                    whole = true;
                }
                at = (next, cn);
            }
            if !whole {
                read_off_programs.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            None
        }).collect::<Vec<_>>()).collect()
    });
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(read_off_programs.into_inner() > 0, "no program's steps were read off: every program's checks are the whole program's");
}

/// The checks of a program linked from tailed modules are of the components its own module's relations or its steps'
/// changes are in (decision 0639 §6a, E14): an unbounded value cycle of its own module beside a library it includes is
/// refused, and one a library relation bounds is not.
#[test]
fn a_programs_checks_refuse_its_own_unbounded_value_cycle() {
    let unbounded = ".language typescript\n#include \"tsx/ts-b.rerail\"\n\
                     .decl num(x: node, f: float)\nnum(x, to_float(v)) :- kind(x, \"number\"), value(x, v).\n\
                     num(x, f + 1.0) :- num(x, f).\n.output num\n";
    match super::parse_version(unbounded) {
        Err(Error::UnboundedValueCycle { component, .. }) => assert!(component.iter().any(|r| r.contains("num")), "{component:?}"),
        other => panic!("an unbounded value cycle of the own module: {:?}", other.map(|_| "planned")),
    }
    let bounded = ".language typescript\n#include \"tsx/ts-b.rerail\"\n\
                   .decl ua(x: node, a: node)\nua(x, a) :- kind(x, \"unary_expression\"), field(x, \"argument\", e), deparen(e, a).\n\
                   .decl num(x: node, f: float)\nnum(x, to_float(v)) :- kind(x, \"number\"), value(x, v).\n\
                   num(x, 0.0 - f) :- ua(x, a), num(a, f).\n.output num\n";
    super::parse_version(bounded).unwrap_or_else(|e| panic!("a value cycle a library relation bounds: {e}"));
}

/// Spike for decision 0639 step 6 (the tail by module), over the packs named (`RERAIL_SPIKE_PACKS`, comma-separated
/// pack directories or `all`; `phases` for the engine's phase programs) and `RERAIL_SPIKE_N` programs a pack at most:
/// - E12: whether the tail's steps less the downward ones (`rr::downward_steps`), run on each module in its scope, give
///   together what they give on the whole library (the union of the parts' lines against the whole's), and where not,
///   the first step that makes the difference (by halving the steps run);
/// - E13: whether a program's tail with the module steps first, then the downward ones, then the steps that follow
///   them in `TAIL`, is its tail in `TAIL`'s order;
/// - E14: the strongly connected components of a library's relations (the module steps' output) that hold relations
///   two modules give rules to, or relations of two modules;
/// - E15: a program's tail time on this thread, by part: all of `TAIL`, the module steps, the downward steps alone
///   and the steps after them, each over the module steps' output.
#[test]
#[ignore = "spike for decision 0639 step 6: RERAIL_SPIKE_PACKS=python[,phases|all] RERAIL_SPIKE_N=<programs a pack>"]
fn spike_0639_tail_by_module() {
    use super::{TAIL, TailStep};
    let down = super::downward_steps();
    use std::collections::{BTreeMap, HashMap};
    use std::time::Instant;
    let lib = crate::code_rule_dsl::logic::rr::library();
    let per: usize = std::env::var("RERAIL_SPIKE_N").ok().and_then(|v| v.parse().ok()).unwrap_or(4);
    let asked = std::env::var("RERAIL_SPIKE_PACKS").unwrap_or_else(|_| "python".into());
    let mut packs: Vec<(String, PathBuf)> = Vec::new();
    for e in std::fs::read_dir(lib.join("../packs")).unwrap().flatten().filter(|e| e.path().is_dir()) {
        packs.push((e.file_name().to_string_lossy().into_owned(), e.path()));
    }
    packs.push(("phases".into(), crate::source_tree::native().join("src/code_rule_dsl/typing/programs")));
    packs.sort();
    packs.retain(|(n, _)| asked == "all" || asked.split(',').any(|a| a == n));
    let no_includes = |s: Vec<from_tape::Stmt>| -> Vec<from_tape::Stmt> { s.into_iter().filter(|s| !matches!(s.what(), Some(from_tape::S::Include(_)))).collect() };
    let module_steps: Vec<TailStep> = TAIL.iter().copied().filter(|s| !down.contains(s)).collect();
    let first_down = TAIL.iter().position(|s| down.contains(s)).unwrap();
    let after: Vec<TailStep> = TAIL[first_down..].iter().copied().filter(|s| !down.contains(s)).collect();
    let module_first: Vec<TailStep> = module_steps.iter().chain(down.iter()).chain(after.iter()).copied().collect();
    let tail = |modules: &[lower::Module], own: Option<lower::Module>, declared: bool, steps: &[TailStep]| -> Result<Arc<Version>, String> {
        let linked = lower::link_library(modules.to_vec()).map_err(|e| e.to_string())?.with(own).map_err(|e| e.to_string())?;
        super::tail_steps(linked.version, declared, linked.lowered, None, steps).map_err(|e| e.to_string())
    };
    // A program's lines, each `.input` and `.output` list a line a relation: a module states its own.
    let lines = |v: &Arc<Version>| -> BTreeSet<String> {
        rule_lines_names_aside(&print_version(v).unwrap())
            .into_iter()
            .flat_map(|l| match l.split_once(' ') {
                Some((d @ (".input" | ".output"), rest)) => rest.split(", ").map(|r| format!("{d} {}", r.trim())).collect::<Vec<_>>(),
                _ => vec![l],
            })
            .collect()
    };
    let sample = |a: &BTreeSet<String>, b: &BTreeSet<String>| a.difference(b).take(5).map(|l| l.chars().take(240).collect::<String>()).collect::<Vec<_>>();
    for (pack, dir) in &packs {
        let mut programs = walk_rr(dir);
        programs.sort();
        let mut paths: BTreeSet<PathBuf> = BTreeSet::new();
        let mut texts = Vec::new();
        for path in &programs {
            let Ok(src) = std::fs::read_to_string(path) else { continue };
            if let Ok(Some(m)) = super::modules(&src) {
                let files: Vec<PathBuf> = m.files.into_iter().map(|(p, _, _)| p).collect();
                paths.extend(files.iter().cloned());
                texts.push((path.clone(), src, files));
            }
        }
        if paths.is_empty() {
            let why: Vec<String> = programs.iter().take(3).map(|p| format!("{}: {:?}", p.display(), std::fs::read_to_string(p).map_err(|e| e.to_string()).map(|s| super::modules(&s).map(|m| m.map(|m| m.files.len())).map_err(|e| e.to_string())))).collect();
            eprintln!("E12 {pack}: {} programs, none with library files: {why:#?}", programs.len());
            continue;
        }
        let plan = match super::planned_library(&paths.into_iter().collect::<Vec<_>>()) {
            Ok(p) => p,
            Err(e) => { eprintln!("{pack}: no plan: {e}"); continue; }
        };
        let at: HashMap<&str, usize> = plan.iter().enumerate().map(|(i, m)| (m.key.as_str(), i)).collect();
        let modules: Vec<lower::Module> = plan.iter().map(|m| m.module().unwrap()).collect();
        let scope_of = |i: usize| -> Vec<lower::Module> {
            let mut s: Vec<usize> = plan[i].scope.split(',').filter(|k| !k.is_empty()).map(|k| at[k]).collect();
            s.sort();
            s.into_iter().map(|j| modules[j].clone()).chain([modules[i].clone()]).collect()
        };
        // E12: the module steps on each module in its scope, against the whole library: lines with their variables and
        // columns renamed (`alpha`), as two modules spell one content-named helper each their own way.
        let composes = |steps: &[TailStep]| -> Result<(BTreeSet<String>, BTreeSet<String>), String> {
            let whole = lines(&tail(&modules, None, false, steps)?).iter().map(|l| alpha(l)).collect();
            let mut parts = BTreeSet::new();
            for i in 0..modules.len() {
                parts.extend(lines(&tail(&scope_of(i), None, false, steps)?).iter().map(|l| alpha(l)));
            }
            Ok((whole, parts))
        };
        // A list a setting states once for the program (a context's `.pass`, the demand's `.pragma`), which each
        // module states for its own: not a difference of the rules.
        let list_setting = |l: &str| l.starts_with(".pass ") || l.starts_with(".pragma \"magic-transform\"");
        let composed = |w: &BTreeSet<String>, p: &BTreeSet<String>| w.is_subset(p) && p.difference(w).all(|l| list_setting(l));
        if let Ok((w, p)) = composes(&[]) {
            eprintln!("E12 {pack}: the union alone: {}", if w == p { "composes".to_owned() } else { format!("{} lines only whole, {} only composed", w.difference(&p).count(), p.difference(&w).count()) });
        }
        let t = Instant::now();
        match composes(&module_steps) {
            Err(e) => eprintln!("E12 {pack}: {} modules: failed: {e}", modules.len()),
            Ok((whole, parts)) if composed(&whole, &parts) => eprintln!("E12 {pack}: {} modules: the parts compose to the whole ({} lines; {} list settings of a module's own; {:.1} s)", modules.len(), whole.len(), parts.difference(&whole).count(), t.elapsed().as_secs_f64()),
            Ok((whole, parts)) => {
                let others: BTreeSet<String> = parts.difference(&whole).filter(|l| !list_setting(l)).cloned().collect();
                eprintln!("E12 {pack}: {} modules: {} lines whole, {} composed; {} only whole {:#?}; {} only composed, not list settings {:#?}", modules.len(), whole.len(), parts.len(), whole.difference(&parts).count(), sample(&whole, &parts), others.len(), others.iter().take(5).collect::<Vec<_>>());
                // Each context instance only the whole makes: the relation instanced, the modules giving it rules, and
                // the modules whose own tail makes an instance of that context.
                let name = |i: usize| plan[i].files.lines().next().unwrap_or_default().rsplit('/').next().unwrap_or_default().to_owned();
                let mut by_module: Vec<BTreeSet<String>> = Vec::new();
                for i in 0..modules.len() {
                    by_module.push(tail(&scope_of(i), None, false, &module_steps).map(|v| v.declared_names().into_iter().filter(|r| r.starts_with("rr_pass_")).collect()).unwrap_or_default());
                }
                let whole_v = tail(&modules, None, false, &module_steps).unwrap();
                let missing: Vec<String> = whole_v.declared_names().into_iter().filter(|r| r.starts_with("rr_pass_") && !by_module.iter().any(|b| b.contains(r))).collect();
                eprintln!("E12 {pack}: {} context instances only the whole makes", missing.len());
                // The composed-only lines' relations: each a host relation the whole leaves to a context (declares its
                // instance `rr_pass_<context>_<r>` and not `r`), or not.
                let whole_names = whole_v.declared_names();
                let head = |l: &str| -> String { l.trim_start_matches(".decl ").split('(').next().unwrap_or_default().trim().to_owned() };
                let (mut left, mut other) = (BTreeSet::new(), Vec::new());
                for l in others.iter() {
                    let r = head(l);
                    if !whole_names.contains(&r) && whole_names.iter().any(|w| w.starts_with("rr_pass_") && w.ends_with(&format!("_{r}"))) {
                        left.insert(r);
                    } else {
                        other.push(l.clone());
                    }
                }
                eprintln!("E12 {pack}: composed-only lines of host relations the whole leaves to a context: {} relations {:?}; other lines: {}", left.len(), left, other.len());
                for r in left.iter().take(2) {
                    let keep: Vec<String> = (0..modules.len()).filter(|&i| tail(&scope_of(i), None, false, &module_steps).is_ok_and(|v| v.declares(r))).map(|i| format!("{} (scope {})", name(i), plan[i].scope.split(',').filter(|k| !k.is_empty()).map(|k| name(at[k])).collect::<Vec<_>>().join(" "))).collect();
                    let ctx: Vec<String> = (0..modules.len()).filter(|&i| ProgramFields::settings(&**modules[i].version()).iter().any(|(k, t)| k == "context" && t.starts_with("script "))).map(name).collect();
                    eprintln!("E12 {pack}:   {r} kept as the host's in {keep:?}; `.context script` declared in {ctx:?}");
                }
                for l in other.iter().take(6) {
                    eprintln!("E12 {pack}:   other: {}", l.chars().take(240).collect::<String>());
                }
                for r in missing.iter().take(6) {
                    let rest = &r["rr_pass_".len()..];
                    let (ctx, base) = rest.split_once('_').unwrap_or((rest, ""));
                    let owners: Vec<String> = modules.iter().enumerate().filter(|(_, m)| m.version().rules().any(|x| m.version().rel_name(x.head().rel()) == base)).map(|(i, _)| name(i)).collect();
                    let readers: Vec<String> = modules.iter().enumerate().filter(|(_, m)| m.version().rules().any(|x| x.reads().any(|a| m.version().rel_name(a.rel()) == base))).map(|(i, _)| name(i)).collect();
                    let makers: Vec<String> = (0..modules.len()).filter(|&i| by_module[i].iter().any(|x| x.starts_with(&format!("rr_pass_{ctx}_")))).map(name).collect();
                    eprintln!("E12 {pack}:   {r}: rules of {base} in {owners:?}; read in {readers:?}; the context's instances made at {makers:?}");
                }
                // The first step whose prefix no longer composes.
                let (mut good, mut bad) = (0, module_steps.len());
                while bad - good > 1 {
                    let mid = (good + bad) / 2;
                    match composes(&module_steps[..mid]) {
                        Ok((w, p)) if composed(&w, &p) => good = mid,
                        _ => bad = mid,
                    }
                }
                eprintln!("E12 {pack}: first step that does not compose: {:?} (step {bad} of {})", module_steps[bad - 1], module_steps.len());
            }
        }
        // E12b: the module steps again over their own output (a module's run over its scope's tailed parts).
        if let Ok(once) = tail(&modules, None, false, &module_steps) {
            let linked = lower::link_library(modules.clone()).unwrap().with(None).unwrap();
            match super::tail_steps(Arc::clone(&once), false, linked.lowered, None, &module_steps) {
                Ok(twice) => {
                    let (a, b): (BTreeSet<String>, BTreeSet<String>) = (lines(&once).iter().map(|l| alpha(l)).collect(), lines(&twice).iter().map(|l| alpha(l)).collect());
                    eprintln!("E12b {pack}: the module steps twice: {}", if a == b { "as once".to_owned() } else { format!("{} lines only once {:?}, {} only twice {:?}", a.difference(&b).count(), sample(&a, &b), b.difference(&a).count(), sample(&b, &a)) });
                }
                Err(e) => eprintln!("E12b {pack}: the module steps twice: failed: {}", e.to_string().chars().take(300).collect::<String>()),
            }
        }
        // E14: components over two modules' relations.
        if let Ok(whole) = tail(&modules, None, false, &module_steps) {
            let mut owners: HashMap<String, BTreeSet<usize>> = HashMap::new();
            for (i, m) in modules.iter().enumerate() {
                let v = m.version();
                for r in v.rules() {
                    owners.entry(v.rel_name(r.head().rel()).to_owned()).or_default().insert(i);
                }
            }
            let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
            for r in whole.rules() {
                let h = whole.rel_name(r.head().rel()).to_owned();
                let e = edges.entry(h).or_default();
                for a in r.reads() {
                    e.insert(whole.rel_name(a.rel()).to_owned());
                }
            }
            let comps = sccs(&edges);
            let multi: Vec<String> = owners.iter().filter(|(_, o)| o.len() > 1).map(|(r, _)| r.clone()).collect();
            let mut crossing = 0;
            for c in comps.iter().filter(|c| c.len() > 1 || c.iter().any(|r| edges.get(r).is_some_and(|e| e.contains(r)))) {
                let mods: BTreeSet<usize> = c.iter().flat_map(|r| owners.get(r).into_iter().flatten().copied()).collect();
                if mods.len() > 1 {
                    crossing += 1;
                    eprintln!("E14 {pack}: a component over {} modules: {:?}", mods.len(), c.iter().take(8).collect::<Vec<_>>());
                }
            }
            eprintln!("E14 {pack}: {} components, {crossing} over two modules or more; relations with rules from two modules: {:?}", comps.len(), multi);
        }
        // E13 and E15 over the pack's programs.
        let (mut same, mut differ, mut failed) = (0, 0, 0);
        let mut times: [f64; 5] = [0.0; 5];
        let mut timed = 0;
        for (path, src, files) in texts.iter().take(per) {
            let Ok((ms, _, _)) = super::library_modules_planned(files) else { failed += 1; continue };
            let own: Vec<from_tape::Stmt> = no_includes(from_tape::RrFile::read(src.as_str()).unwrap().statements());
            let scope: Vec<&lower::Module> = ms.iter().collect();
            let Ok(own) = lower::lower_module_in_written(own, &scope) else { failed += 1; continue };
            let t0 = Instant::now();
            let a = tail(&ms, Some(own.clone()), true, &TAIL);
            let t1 = t0.elapsed().as_secs_f64();
            let b = tail(&ms, Some(own.clone()), true, &module_first);
            match (a, b) {
                (Ok(a), Ok(b)) => {
                    let (la, lb) = (lines(&a), lines(&b));
                    if la == lb {
                        same += 1;
                    } else {
                        differ += 1;
                        if differ <= 3 {
                            eprintln!("E13 {pack}: {}: only TAIL's order {:#?}; only module first {:#?}", path.display(), sample(&la, &lb), sample(&lb, &la));
                        }
                    }
                }
                (a, b) => {
                    failed += 1;
                    eprintln!("E13 {pack}: {}: {:?} / {:?}", path.display(), a.err(), b.err());
                }
            }
            // E15: the parts' times, each over a fresh link.
            let t = Instant::now();
            let moduled = tail(&ms, Some(own.clone()), true, &module_steps);
            let t2 = t.elapsed().as_secs_f64();
            if let Ok(m) = moduled {
                let linked = lower::link_library(ms.clone()).unwrap().with(Some(own.clone())).unwrap();
                let t = Instant::now();
                let down = super::tail_steps(Arc::clone(&m), true, linked.lowered, None, &down);
                let t3 = t.elapsed().as_secs_f64();
                if let Ok(d) = down {
                    let linked = lower::link_library(ms.clone()).unwrap().with(Some(own)).unwrap();
                    let t = Instant::now();
                    let _ = super::tail_steps(d, true, linked.lowered, None, &after);
                    let t4 = t.elapsed().as_secs_f64();
                    for (k, x) in [t1, t2, t3, t4, 0.0].into_iter().enumerate() {
                        times[k] += x;
                    }
                    timed += 1;
                }
            }
        }
        eprintln!("E13 {pack}: {same} the same, {differ} differ, {failed} not compared");
        // E16: each step's time over a program's union (TAIL's order, one step a run on the last's output), and whether
        // the steps after the downward ones change only the relations the downward ones or the program's own module
        // touch.
        let mut step_ms: BTreeMap<String, f64> = BTreeMap::new();
        let (mut local_after, mut wider_after, mut sampled) = (0, 0, 0);
        for (_, src, files) in texts.iter().take(per) {
            let Ok((ms, _, _)) = super::library_modules_planned(files) else { continue };
            let own_st: Vec<from_tape::Stmt> = no_includes(from_tape::RrFile::read(src.as_str()).unwrap().statements());
            let scope: Vec<&lower::Module> = ms.iter().collect();
            let Ok(own) = lower::lower_module_in_written(own_st, &scope) else { continue };
            let own_heads: BTreeSet<String> = own.version().rules().map(|r| own.version().rel_name(r.head().rel()).to_owned()).collect();
            let linked = lower::link_library(ms.clone()).unwrap().with(Some(own.clone())).unwrap();
            let mut v = linked.version;
            let mut ok = true;
            for (k, step) in TAIL.iter().enumerate() {
                let linked = lower::link_library(ms.clone()).unwrap().with(Some(own.clone())).unwrap();
                let t = Instant::now();
                match super::tail_steps(Arc::clone(&v), true, linked.lowered, None, &[*step]) {
                    Ok(n) => v = n,
                    Err(_) => { ok = false; break; }
                }
                *step_ms.entry(format!("{k:02} {step:?}")).or_default() += t.elapsed().as_secs_f64() * 1000.0;
            }
            if !ok {
                continue;
            }
            // The downward steps' changes over the module steps' output, and the after steps' over theirs.
            let heads = |v: &Arc<Version>| -> BTreeMap<String, BTreeSet<String>> {
                let mut m: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
                for l in lines(v) {
                    let h = l.trim_start_matches(".decl ").split(['(', ' ']).next().unwrap_or_default().to_owned();
                    m.entry(h).or_default().insert(alpha(&l));
                }
                m
            };
            let fresh = || lower::link_library(ms.clone()).unwrap().with(Some(own.clone())).unwrap();
            let Ok(m) = super::tail_steps(fresh().version, true, fresh().lowered, None, &module_steps) else { continue };
            let Ok(d) = super::tail_steps(Arc::clone(&m), true, fresh().lowered, None, &down) else { continue };
            let Ok(a) = super::tail_steps(Arc::clone(&d), true, fresh().lowered, None, &after) else { continue };
            let (hm, hd, ha) = (heads(&m), heads(&d), heads(&a));
            let changed = |x: &BTreeMap<String, BTreeSet<String>>, y: &BTreeMap<String, BTreeSet<String>>| -> BTreeSet<String> { x.keys().chain(y.keys()).filter(|k| x.get(*k) != y.get(*k)).cloned().collect() };
            let down = changed(&hm, &hd);
            let aft = changed(&hd, &ha);
            let outside: Vec<&String> = aft.iter().filter(|r| !down.contains(*r) && !own_heads.contains(*r)).collect();
            sampled += 1;
            if outside.is_empty() { local_after += 1 } else {
                wider_after += 1;
                if wider_after <= 2 {
                    eprintln!("E16 {pack}: downward changed {} relations, after {}; after outside them and the own module's: {:?}", down.len(), aft.len(), outside.iter().take(8).collect::<Vec<_>>());
                }
            }
        }
        if sampled > 0 {
            eprintln!("E16 {pack}: {sampled} programs: after steps within the downward's and the own module's relations in {local_after}, wider in {wider_after}");
            let mut top: Vec<(&String, &f64)> = step_ms.iter().collect();
            top.sort_by(|a, b| b.1.total_cmp(a.1));
            eprintln!("E16 {pack}: ms a program by step: {}", top.iter().take(8).map(|(k, x)| format!("{k} {:.1}", **x / sampled as f64)).collect::<Vec<_>>().join("; "));
        }
        if timed > 0 {
            let ms = |x: f64| x * 1000.0 / timed as f64;
            eprintln!("E15 {pack}: {timed} programs, ms a program: TAIL {:.1}; module steps {:.1}; downward alone {:.1}; the steps after them {:.1}", ms(times[0]), ms(times[1]), ms(times[2]), ms(times[3]));
        }
    }
}

/// The strongly connected components of `edges` (a relation to the relations its rules read), Tarjan's.
fn sccs(edges: &std::collections::BTreeMap<String, BTreeSet<String>>) -> Vec<Vec<String>> {
    struct T<'a> { e: &'a std::collections::BTreeMap<String, BTreeSet<String>>, index: std::collections::HashMap<&'a str, (usize, usize)>, stack: Vec<&'a str>, on: std::collections::HashSet<&'a str>, n: usize, out: Vec<Vec<String>> }
    fn visit<'a>(t: &mut T<'a>, v: &'a str) {
        t.index.insert(v, (t.n, t.n));
        t.n += 1;
        t.stack.push(v);
        t.on.insert(v);
        if let Some(ws) = t.e.get(v) {
            for w in ws {
                let w = w.as_str();
                if !t.index.contains_key(w) {
                    visit(t, w);
                    let lw = t.index[w].1;
                    let e = t.index.get_mut(v).unwrap();
                    e.1 = e.1.min(lw);
                } else if t.on.contains(w) {
                    let iw = t.index[w].0;
                    let e = t.index.get_mut(v).unwrap();
                    e.1 = e.1.min(iw);
                }
            }
        }
        let (i, l) = t.index[v];
        if i == l {
            let mut c = Vec::new();
            while let Some(w) = t.stack.pop() {
                t.on.remove(w);
                c.push(w.to_owned());
                if w == v {
                    break;
                }
            }
            t.out.push(c);
        }
    }
    let mut t = T { e: edges, index: Default::default(), stack: Vec::new(), on: Default::default(), n: 0, out: Vec::new() };
    for v in edges.keys() {
        if !t.index.contains_key(v.as_str()) {
            visit(&mut t, v);
        }
    }
    t.out
}

/// A printed line with each variable or column named by its place of first use (`v0`, `v1`, …): a name followed by `(`
/// is a relation or a function, one in quotes a string, kept.
fn alpha(line: &str) -> String {
    let mut out = String::new();
    let mut names: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            let start = i;
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(chars.len());
            out.extend(&chars[start..i]);
            continue;
        }
        if c.is_alphabetic() || c == '_' || c == '$' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '$') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let next = chars[i..].iter().find(|c| !c.is_whitespace()).copied();
            let keyword = start > 0 && chars[start - 1] == '.' || start > 0 && chars[start - 1] == '@';
            let type_name = start > 1 && chars[start - 2] == ':';
            if next == Some('(') || keyword || type_name || word.chars().next().is_some_and(char::is_uppercase) {
                out.push_str(&word);
            } else {
                let n = names.len();
                let k = *names.entry(word).or_insert(n);
                out.push_str(&format!("v{k}"));
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}
