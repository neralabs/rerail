//! The converter's run over the data-layer pack, and checks of what it writes.

use super::*;
use crate::code_rule_dsl::CodeRuleDslRuntime;
use crate::code_rule_dsl::model::{ClauseKind, Query};
use crate::code_rule_dsl::test_support::CAPABILITIES;
use std::path::{Path, PathBuf};

/// The rules converted: the pack as rewritten by Pulling-it-all-together (`rules-ast`, decision 0037 step 1).
const RULES: &str = "rules-ast";

fn pack() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../gate/data-layer/packs/data-layer/candidates")
}

/// What one rule's conversion gave: its program, and per clause whether it converted and why not.
struct Converted {
    program: String,
    clauses: Vec<(String, Result<(), Gap>)>,
    approx: BTreeSet<String>,
}

fn scope_globs(document: &serde_json::Value, key: &str) -> Vec<String> {
    document.get("scope").and_then(|s| s.get(key)).and_then(|v| v.as_array()).into_iter().flatten().filter_map(|g| g.as_str().map(str::to_owned)).collect()
}

/// How a replay event adjusts a rule: its scope, and which clauses it keeps.
#[derive(Default)]
struct Adjust {
    scope: Option<(Vec<String>, Vec<String>)>,
    keep: Option<Box<dyn Fn(&str) -> bool>>,
}

/// One rule file as a program: its scope as path globs and each forbid clause's matcher as rules deriving
/// `violation(rule, clause, code, path, line)`.
fn convert(path: &Path, adjust: &Adjust) -> Converted {
    let text = std::fs::read_to_string(path).unwrap();
    let document: serde_json::Value = serde_json::from_str(&text).unwrap();
    let id = document["id"].as_str().unwrap().to_owned();
    let compiled = CodeRuleDslRuntime::compile(text, "json".to_owned(), CAPABILITIES.to_owned()).unwrap_or_else(|e| panic!("{id}: {e:?}"));
    let mut e = Emitter::new(GrammarDialect::Python);
    let (include, exclude) = adjust.scope.clone().unwrap_or_else(|| (scope_globs(&document, "include"), scope_globs(&document, "exclude")));
    // The scope: a document in an include glob and in no exclude glob.
    for g in &include {
        e.rule("in_scope($d)", "d: int", vec!["doc($d, $p, _)".into(), format!("@glob($p, {})", quote(g))]);
    }
    for g in &exclude {
        e.rule("out_scope($d)", "d: int", vec!["doc($d, $p, _)".into(), format!("@glob($p, {})", quote(g))]);
    }
    if include.is_empty() {
        e.decls.insert("in_scope".into(), "d: int".into());
    }
    let excludes = !exclude.is_empty();
    e.decls.insert("violation".into(), "rule: sym, clause: sym, code: sym, path: sym, line: int".into());
    let mut clauses = Vec::new();
    for clause in &compiled.inner.clauses {
        if adjust.keep.as_ref().is_some_and(|keep| !keep(&clause.id)) {
            continue;
        }
        let result = (|| -> Res<()> {
            let matchers = match &clause.kind {
                ClauseKind::Forbid(Query::Ast { matchers, .. }) => matchers,
                ClauseKind::Forbid(Query::Regex { .. }) => return gap("a regex over file content or paths"),
                ClauseKind::Forbid(Query::SelectWhen { .. }) => return gap("select/when"),
                ClauseKind::RequireEach { .. } => return gap("require each (define)"),
                ClauseKind::Budget { .. } => return gap("budget"),
                _ => return gap("another clause kind"),
            };
            let Some(grammar) = matchers.iter().find(|m| m.language == GrammarDialect::Python) else { return gap("no Python matcher") };
            if !grammar.applies {
                return Ok(());
            }
            if grammar.document.is_some() {
                return gap("an embedded program");
            }
            e.captures.clear();
            let mut body = e.body(&grammar.matcher, "$n")?;
            body = nonempty(body, "$n");
            if let Some(file) = &grammar.file {
                if file.module_name.is_some() || file.package.is_some() || file.in_package.is_some() || file.manifest.is_some() || file.declaration.is_some() {
                    return gap("a file condition on module names or packages");
                }
                body.push("doc($d, $p, $m)".into());
                if let Some(c) = &file.path {
                    body.extend(e.condition(c, "$p")?);
                }
                if let Some(c) = &file.module {
                    body.extend(e.condition(c, "$m")?);
                }
            }
            // Where the violation is: the witness's first line, a capture's, or the file's first.
            let at = match &clause.violation.anchor {
                crate::code_rule_dsl::model::Anchor::Capture(name) => match e.captures.get(name) {
                    Some(v) => v.clone(),
                    None => return gap("an anchor on a capture the rule does not bind"),
                },
                _ => "$n".to_owned(),
            };
            body.extend(["node($d, $n, _, _, _)".to_owned(), "in_scope($d)".to_owned()]);
            // `syntax`: whether the file has a syntax error (an `ERROR` node; missing tokens are not nodes).
            if let Some(invalid) = grammar.invalid_syntax {
                body.push(if invalid { "syntax_error($d)".into() } else { "!syntax_error($d)".into() });
            }
            if excludes {
                body.push("!out_scope($d)".into());
            }
            body.push("doc($d, $p, _)".to_owned());
            let line = if clause.violation.anchor == crate::code_rule_dsl::model::Anchor::FileStart {
                "1".to_owned()
            } else {
                body.push(format!("line({at}, $l)"));
                "$l".to_owned()
            };
            e.rules.push(format!("violation({}, {}, {}, $p, {line}) :- {}.", quote(&id), quote(&clause.id), quote(&clause.violation.code), body.join(", ")));
            Ok(())
        })();
        clauses.push((clause.id.clone(), result));
    }
    Converted { program: e.program(&["violation"]), clauses, approx: e.approx.clone() }
}

fn rule_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(pack().join(RULES)).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    files.sort();
    files
}

/// The converter's run: every rule of the pack as a program in `candidates/datalog/<rule>.rdl`, and the
/// coverage (`coverage.txt`): clauses converted, the gaps by kind, the approximations.
#[test]
#[ignore = "writes the converted rules into the pack"]
fn convert_the_data_layer_pack() {
    let out = pack().join("datalog");
    std::fs::create_dir_all(&out).unwrap();
    let (mut total, mut done) = (0, 0);
    let mut gaps = BTreeMap::<String, usize>::new();
    let mut approx = BTreeMap::<String, usize>::new();
    let mut per_rule = Vec::new();
    for path in rule_files() {
        let id = path.file_stem().unwrap().to_string_lossy().into_owned();
        let c = convert(&path, &Adjust::default());
        crate::code_rule_dsl::logic::front::parse(&c.program).unwrap_or_else(|e| panic!("{id}: {e:?}\n{}", c.program));
        std::fs::write(out.join(format!("{id}.rdl")), &c.program).unwrap();
        let ok = c.clauses.iter().filter(|(_, r)| r.is_ok()).count();
        total += c.clauses.len();
        done += ok;
        for (_, r) in &c.clauses {
            if let Err(Gap(g)) = r {
                // Gaps by kind: the text before any detail.
                let kind = if g.starts_with("a regex on the text") { g.clone() } else { g.split(" of ").next().unwrap_or(g).to_owned() };
                *gaps.entry(kind).or_default() += 1;
            }
        }
        for a in &c.approx {
            *approx.entry(a.clone()).or_default() += 1;
        }
        per_rule.push(format!("{id}: {ok}/{}", c.clauses.len()));
        for (clause, r) in &c.clauses {
            if let Err(Gap(g)) = r {
                let g: String = g.chars().take(160).collect();
                per_rule.push(format!("    {clause}: {}", g.replace('\n', " ")));
            }
        }
    }
    let mut report = format!("{done}/{total} clauses converted\n\ngaps (clauses):\n");
    let mut by_count: Vec<_> = gaps.into_iter().collect();
    by_count.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (g, n) in by_count {
        report.push_str(&format!("  {n:5}  {g}\n"));
    }
    report.push_str("\napproximations (rules):\n");
    for (a, n) in approx {
        report.push_str(&format!("  {n:5}  {a}\n"));
    }
    report.push_str("\nper rule:\n");
    for r in per_rule {
        report.push_str(&format!("  {r}\n"));
    }
    std::fs::write(out.join("coverage.txt"), &report).unwrap();
    println!("{report}");
}

#[test]
fn regexes_become_names_and_string_tests() {
    let parse = |s: &str| regex_syntax::ParserBuilder::new().build().parse(s).unwrap();
    assert!(matches!(shape(&parse("(?s).*")), Some(Shape::Anything)));
    assert!(matches!(shape(&parse(r"\b\B")), Some(Shape::Nothing)));
    let Some(Shape::Names(n)) = shape(&parse("^(?:lazy|pure_lazy)$")) else { panic!() };
    assert_eq!(n.into_iter().collect::<Vec<_>>(), ["lazy", "pure_lazy"]);
    let Some(Shape::Tests(t)) = shape(&parse("_lazy$")) else { panic!() };
    assert_eq!(t, [("suffix", "_lazy".to_owned())]);
    let Some(Shape::Tests(t)) = shape(&parse("^_")) else { panic!() };
    assert_eq!(t, [("prefix", "_".to_owned())]);
    let Some(Shape::Tests(t)) = shape(&parse("Connection|Transaction")) else { panic!() };
    assert_eq!(t, [("contains", "Connection".to_owned()), ("contains", "Transaction".to_owned())]);
    let Some(Shape::Tests(t)) = shape(&parse("(?i)validat")) else { panic!() };
    assert_eq!(t, [("icontains", "validat".to_owned())]);
    let Some(Shape::Names(n)) = shape(&parse(r"^(asyncio\.)?(run|to_thread)$")) else { panic!() };
    assert_eq!(n.into_iter().collect::<Vec<_>>(), ["asyncio.run", "asyncio.to_thread", "run", "to_thread"]);
    assert!(shape(&parse(r"Callable\s*\[")).is_none());
}

/// Regexes become globs exactly: on sample strings each glob set agrees with its regex (decision 0037).
#[test]
fn regexes_become_exact_globs() {
    let parse = |s: &str| regex_syntax::ParserBuilder::new().build().parse(s).unwrap();
    let cases: [(&str, Text, Option<&[&str]>); 13] = [
        (r"^[^_].*_", Text::Name, Some(&["[!_]*_*"])),
        (r"^apply_.*_effect$", Text::Name, Some(&["apply_*_effect"])),
        (r"^_\w*cache\w*$", Text::Name, Some(&["_*cache*"])),
        (r"^Py[A-Z][A-Za-z0-9_]*$", Text::Name, Some(&["Py[A-Z]*"])),
        (r"^_*(?:Async|Lazy)", Text::Name, None),
        (r"\basyncio\s*\.\s*to_thread$", Text::Name, Some(&["*.asyncio.to_thread", "asyncio.to_thread"])),
        (r"^(?:materialize|assert_result|.*_post_op)$", Text::Name, Some(&["*_post_op", "assert_result", "materialize"])),
        (r"(?:^|/)tests(?:/|$)", Text::Path, Some(&["**/tests", "**/tests/**"])),
        (r"^(?:api/)?impl/common/l2070_server_plan(?:/|$)", Text::Path, None),
        (r"(?:^|/)l2070_batching/(?:.*/)?executor\.py$", Text::Path, None),
        (r"(?:^|/)l9750_graphql/", Text::Path, None),
        (r"(?i)cursor|^(?:db|conn)$", Text::Name, None),
        (r"^(?:ab|c)+$", Text::Name, Some(&[])),
    ];
    let samples = [
        "a_b", "_ab", "ab", "apply_x_effect", "apply__effect", "_my_cache_x", "cache", "PyResult", "Pyresult", "Py", "_Async", "__Lazy_x", "Lazy",
        "materialize", "x_post_op", "_post_op", "tests", "a/tests", "a/tests/b.py", "tests/b", "attests/b", "a/b/tests/c/d.py", "impl/common/l2070_server_plan",
        "api/impl/common/l2070_server_plan/x.py", "impl/common/l2070_server_planx", "l2070_batching/executor.py", "a/l2070_batching/x/y/executor.py",
        "xl2070_batching/executor.py", "l9750_graphql/a.py", "a/l9750_graphql/b/c.py", "l9750_graphqlx/a.py", "raw_CURSOR", "db", "dbx", "conn", "asyncio.to_thread", "x.asyncio.to_thread", "xasyncio.to_thread",
    ];
    for (re, text, expect) in cases {
        let got = globs(&parse(re), text);
        if expect == Some(&[]) {
            assert!(got.is_none(), "{re}: a repeated alternation has no glob");
            continue;
        }
        if let Some(expect) = expect {
            assert_eq!(got.as_ref().map(|(g, _, _)| g.iter().map(String::as_str).collect::<Vec<_>>()), Some(expect.to_vec()), "{re}");
        }
        let (gs, fold, _) = got.unwrap_or_else(|| panic!("{re} gave no globs"));
        let m = crate::code_rule_dsl::logic::strings::Matcher::new(if fold { "iglob" } else { "glob" }, &gs).unwrap();
        let r = regex::Regex::new(re).unwrap();
        for s in samples {
            let in_text = match text {
                Text::Name => !s.contains('/') && !s.contains(' '),
                _ => true,
            };
            if in_text {
                assert_eq!(m.is_match(s), r.is_match(s), "{re} on {s:?}: globs {gs:?}");
            }
        }
    }
}

/// The recorded events (the upstream linters' tests, `fixtures/host/data-layer-port`), as `replay.ts` reads them.
fn events() -> Vec<serde_json::Value> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../gate/data-layer/fixtures/host/data-layer-port");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    paths.sort();
    let mut out = Vec::new();
    for p in paths {
        let recording: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        for mut event in recording["events"].as_array().cloned().unwrap_or_default() {
            if event.get("retained_reason").is_some() || event["upstream_passed"] != serde_json::Value::Bool(true) {
                continue;
            }
            if event.get("files").is_none_or(serde_json::Value::is_null) {
                event["files"] = recording["files"].clone();
            }
            out.push(event);
        }
    }
    out
}

/// The replay's clause filters (`replay.ts`), by check and entry.
fn keep(check: &str, entry: &str, production: bool) -> Option<Box<dyn Fn(&str) -> bool>> {
    let entry = entry.to_owned();
    match check {
        "no_blanket_grammar_bypass" if !production => {
            let public = entry == "check_no_public_grammar_methods";
            Some(Box::new(move |c: &str| (c == "public-methods") == public))
        }
        "type_safety_patterns" if !production && ["check_function_signatures", "check_file_for_patterns"].contains(&entry.as_str()) => {
            let sig = entry == "check_function_signatures";
            Some(Box::new(move |c: &str| if sig { c.starts_with("signature-") } else { c.starts_with("text-") }))
        }
        "transaction_closures" if entry.ends_with(".visit") => {
            let prefix = if entry == "TransactionClosureLinter.visit" { "inline-" } else { "named-" };
            Some(Box::new(move |c: &str| c.starts_with(prefix)))
        }
        "sql_grammar_escape_hatches" if entry.starts_with("scan_usages") => Some(Box::new(|c: &str| c.starts_with("usage-"))),
        _ => None,
    }
}

/// The converted rules on the recorded events: per check, the events whose clauses all converted and how many
/// give the recorded findings exactly (path, line, code). `RERAIL_REPLAY_CHECK` selects checks (comma-separated).
#[test]
#[ignore = "runs every recorded event through the corpus runner"]
fn converted_rules_give_the_recorded_findings() {
    let only: Option<Vec<String>> = std::env::var("RERAIL_REPLAY_CHECK").ok().map(|s| s.split(',').map(str::to_owned).collect());
    let mut per_check = BTreeMap::<String, [usize; 4]>::new();
    let mut misses = Vec::new();
    let root = crate::code_rule_dsl::prefilter::tests::temp().with_extension("replay");
    for (k, event) in events().into_iter().enumerate() {
        let check = event["check"].as_str().unwrap().to_owned();
        if only.as_ref().is_some_and(|o| !o.contains(&check)) {
            continue;
        }
        let row = per_check.entry(check.clone()).or_default();
        row[0] += 1;
        if event.get("oracle_config").is_some_and(|c| !c.is_null()) || event.get("path_skip_override").is_some_and(|v| v == true) {
            continue;
        }
        let production = event.get("production_scope").is_some_and(|v| v == true);
        let globs = |key: &str| event["scope"][key].as_array().into_iter().flatten().filter_map(|g| g.as_str().map(str::to_owned)).collect::<Vec<_>>();
        let scope = (!production).then(|| if globs("include").is_empty() { (vec![], vec![]) } else { (globs("include"), globs("exclude")) });
        let adjust = Adjust { scope, keep: keep(&check, event["entry"].as_str().unwrap_or(""), production) };
        let rule = pack().join(RULES).join(format!("data-layer-{}.json", check.replace('_', "-")));
        let c = convert(&rule, &adjust);
        if c.clauses.iter().any(|(_, r)| r.is_err()) {
            continue;
        }
        row[1] += 1;
        let dir = root.join(format!("e{k}"));
        for f in event["files"].as_array().unwrap() {
            let p = dir.join(f["path"].as_str().unwrap());
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, f["source"].as_str().unwrap()).unwrap();
        }
        let out = dir.join(".out");
        let mut got: Vec<String> = match super::super::plan::corpus::run(&c.program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18) {
            Ok(_) => std::fs::read_to_string(out.join("violation.csv"))
                .unwrap_or_default()
                .lines()
                .map(|l| {
                    let f: Vec<&str> = l.split('\t').collect();
                    format!("{}:{}:{}", f[3], f[4], f[2])
                })
                .collect(),
            Err(e) => {
                row[3] += 1;
                misses.push(format!("{check} {}: error {e}", event["test"]));
                continue;
            }
        };
        let mut want: Vec<String> = event["findings"].as_array().unwrap().iter().map(|f| format!("{}:{}:{}", f["path"].as_str().unwrap(), f["line"], f["code"].as_str().unwrap())).collect();
        got.sort();
        want.sort();
        if got == want {
            row[2] += 1;
        } else {
            misses.push(format!("{check} {}: want {want:?} got {got:?}", event["test"]));
        }
    }
    let _ = std::fs::remove_dir_all(&root);
    let mut report = String::from("check: events, all clauses converted, exact, errors\n");
    let mut sum = [0; 4];
    for (check, row) in &per_check {
        report.push_str(&format!("  {check}: {} {} {} {}\n", row[0], row[1], row[2], row[3]));
        for i in 0..4 {
            sum[i] += row[i];
        }
    }
    report.push_str(&format!("total: {} {} {} {}\n", sum[0], sum[1], sum[2], sum[3]));
    println!("{report}");
    for m in misses.iter().take(40) {
        println!("{m}");
    }
}
