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
    let mut document: serde_json::Value = serde_json::from_str(&text).unwrap();
    let id = document["id"].as_str().unwrap().to_owned();
    // A relational clause is SQL stages: it is not converted but written by hand in `.rr` (list step 30). It is
    // taken out before the rest compiles, since the newer rules' stages use forms (`escape: sql`, `message`,
    // `line`) only their SQL engine reads.
    let mut relational = Vec::new();
    let order: Vec<String> = document["clauses"].as_array().into_iter().flatten().map(|c| c["id"].as_str().unwrap_or_default().to_owned()).collect();
    if let Some(list) = document["clauses"].as_array_mut() {
        list.retain(|c| {
            let sql = c.get("relational").is_some();
            if sql {
                relational.push(c["id"].as_str().unwrap_or_default().to_owned());
            }
            !sql
        });
    }
    let text = if relational.is_empty() { text } else { document.to_string() };
    // Only relational clauses report: nothing is left to compile.
    let reports = document["clauses"].as_array().into_iter().flatten().any(|c| c.get("define").is_none());
    let compiled = reports.then(|| CodeRuleDslRuntime::compile(text, "json".to_owned(), CAPABILITIES.to_owned()).unwrap_or_else(|e| panic!("{id}: {e:?}")));
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
    let mut clauses: Vec<(String, Result<(), Gap>)> = relational
        .into_iter()
        .filter(|c| adjust.keep.as_ref().is_none_or(|keep| keep(c)))
        .map(|c| (c, Err(Gap("relational (SQL stages)".into()))))
        .collect();
    for clause in compiled.iter().flat_map(|c| &c.inner.clauses) {
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
    clauses.sort_by_key(|(c, _)| order.iter().position(|o| o == c));
    // The emitter's text, in `.rr`: every program the converter gives is Soufflé form (decision 0043).
    let program = crate::code_rule_dsl::logic::rr::translate(&e.program(&["violation"])).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Converted { program, clauses, approx: e.approx.clone() }
}

/// The rules the converter reads, by id: the 60 of Pulling-it-all-together's rewrite (`rules-ast`), then the
/// newer rules of Data-layer-rule-expressivity (its candidates in `rules`, its certified rules in the pack's
/// `rules`), each rule once, the first source winning.
fn rule_files() -> Vec<PathBuf> {
    let mut by_id = BTreeMap::<String, PathBuf>::new();
    for dir in [pack().join(RULES), pack().join("rules"), pack().join("../rules")] {
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "json") {
                by_id.entry(p.file_stem().unwrap().to_string_lossy().into_owned()).or_insert(p);
            }
        }
    }
    by_id.into_values().collect()
}

/// A check's hand-written rule (`docs/rule-syntax/data-layer/<check>.rr`, includes resolved), as `.rr` text, where
/// it lowers. `RERAIL_REPLAY_CONVERTED` replays the converted JSON instead.
/// The codes a hand-written rule reports in log mode (`.mode log A, B`): the suite plugin, the production entry,
/// drops them, as it drops a linter's `audit` severity. A violation's code is its `code "X"`, else its name.
fn log_codes(check: &str) -> Vec<String> {
    let path = hand_dir().join(format!("{}.rr", check.replace('_', "-")));
    let Ok(text) = std::fs::read_to_string(path) else { return vec![] };
    let code_of = |name: &str| {
        text.lines()
            .filter_map(|l| l.trim().strip_prefix(".violation "))
            .find(|l| l.split('(').next() == Some(name))
            .and_then(|l| l.split(" code \"").nth(1).and_then(|c| c.split('"').next()))
            .unwrap_or(name)
            .to_owned()
    };
    text.lines()
        .filter_map(|l| l.split("//").next().unwrap().trim().strip_prefix(".mode log "))
        .flat_map(|names| names.split(',').map(str::trim).filter(|n| !n.is_empty()).map(code_of).collect::<Vec<_>>())
        .collect()
}

/// Where the hand-written data-layer rules are: `docs/rule-syntax/data-layer` (More-ergonomic's drafts), or
/// `RERAIL_HAND_DIR`. The data-layer pack's ports (`gate/data-layer/packs/data-layer/candidates/rr`) read
/// `core.rr` relations the data-layer integration engine adds (`split_line`, `lf_line`), so they wait on that merge.
fn hand_dir() -> PathBuf {
    std::env::var_os("RERAIL_HAND_DIR").map(PathBuf::from).unwrap_or_else(|| crate::code_rule_dsl::logic::rr::library().join("../data-layer"))
}

fn hand_written(check: &str) -> Option<String> {
    if std::env::var_os("RERAIL_REPLAY_CONVERTED").is_some() {
        return None;
    }
    let path = hand_dir().join(format!("{}.rr", check.replace('_', "-")));
    if !path.exists() {
        return None;
    }
    // A rule that does not lower falls back to the converted JSON; say why, so a broken rule is not silent.
    let program = match crate::code_rule_dsl::logic::rr::parse_file(&path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{check}: the hand-written rule does not lower: {e}");
            return None;
        }
    };
    let text = crate::code_rule_dsl::logic::rr::print(&program).unwrap_or_else(|e| panic!("{check}: {e}"));
    // `RERAIL_REPLAY_DUMP=<dir>`: the program as replayed, `<dir>/<check>.rr`.
    if let Some(dir) = std::env::var_os("RERAIL_REPLAY_DUMP") {
        std::fs::write(PathBuf::from(dir).join(format!("{check}.rr")), &text).unwrap();
    }
    Some(text)
}

/// The rule file of a recorded check (`async_sync_boundary` → `data-layer-async-sync-boundary`).
fn rule_file(check: &str) -> Option<PathBuf> {
    let name = format!("data-layer-{}", check.replace('_', "-"));
    rule_files().into_iter().find(|p| p.file_stem().is_some_and(|s| s.to_string_lossy() == name))
}

/// The converter's run: every rule of the pack as a program in `candidates/datalog/<rule>.rr`, and the
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
        crate::code_rule_dsl::logic::rr::parse(&c.program).unwrap_or_else(|e| panic!("{id}: {e:?}\n{}", c.program));
        std::fs::write(out.join(format!("{id}.rr")), &c.program).unwrap();
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

/// One policy file's conversion (`RERAIL_CONVERT_POLICY`): the clauses converted and the gaps, printed; the
/// program is written to `RERAIL_CONVERT_OUT` where given.
#[test]
#[ignore = "converts one policy file; RERAIL_CONVERT_POLICY, --ignored --nocapture"]
fn convert_one_policy() {
    let path = PathBuf::from(std::env::var("RERAIL_CONVERT_POLICY").expect("RERAIL_CONVERT_POLICY"));
    let c = convert(&path, &Adjust::default());
    let ok = c.clauses.iter().filter(|(_, r)| r.is_ok()).count();
    println!("{ok}/{} clauses converted", c.clauses.len());
    let mut gaps = BTreeMap::<String, usize>::new();
    for (_, r) in &c.clauses {
        if let Err(Gap(g)) = r {
            *gaps.entry(g.chars().take(100).collect()).or_default() += 1;
        }
    }
    let mut by_count: Vec<_> = gaps.into_iter().collect();
    by_count.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (g, n) in by_count {
        println!("  gap {n:4}  {g}");
    }
    for a in &c.approx {
        println!("  approx  {a}");
    }
    if let Ok(out) = std::env::var("RERAIL_CONVERT_OUT") {
        std::fs::write(out, &c.program).unwrap();
    }
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

/// The recorded events (the upstream linters' tests, `fixtures/host/data-layer-port`), as `replay.ts` reads them:
/// every batch's recording and probes. Each event carries its group in `_group`: `recordings 1-6` (the baseline
/// the 60 rules were converted against) or `batches 7-8 and probes`.
fn events() -> Vec<serde_json::Value> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../gate/data-layer/fixtures/host/data-layer-port");
    let first = |name: &str| (1..=6).any(|b| name == format!("batch-{b:02}-recording.json"));
    let events = |name: &str| name.starts_with("batch-") && (name.ends_with("-recording.json") || name.ends_with("probes.json"));
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(events))
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for p in paths {
        let recording: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let group = if first(&p.file_name().unwrap().to_string_lossy()) { "recordings 1-6" } else { "batches 7-8 and probes" };
        for mut event in recording["events"].as_array().cloned().unwrap_or_default() {
            if event.get("retained_reason").is_some() || event["upstream_passed"] != serde_json::Value::Bool(true) {
                continue;
            }
            if event.get("files").is_none_or(serde_json::Value::is_null) {
                event["files"] = recording["files"].clone();
            }
            event["_group"] = group.into();
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
    let mut per_check = BTreeMap::<(String, String), [usize; 4]>::new();
    let mut misses = Vec::new();
    let root = crate::code_rule_dsl::prefilter::tests::temp().with_extension("replay");
    for (k, event) in events().into_iter().enumerate() {
        let check = event["check"].as_str().unwrap().to_owned();
        if only.as_ref().is_some_and(|o| !o.contains(&check)) {
            continue;
        }
        // `RERAIL_REPLAY_EVENT=<text>`: only the events whose name contains it.
        if std::env::var("RERAIL_REPLAY_EVENT").is_ok_and(|e| !event["test"].as_str().unwrap_or("").contains(&e)) {
            continue;
        }
        let row = per_check.entry((event["_group"].as_str().unwrap().to_owned(), check.clone())).or_default();
        row[0] += 1;
        // Configured oracles, path-skip overrides, and comparisons of the message or of oracle metadata (which
        // the findings here do not carry) are not replayed.
        let unread = ["compare_message", "compare_fields"].iter().any(|k| event.get(*k).is_some_and(|v| !v.is_null() && v != false && v.as_array().is_none_or(|a| !a.is_empty())));
        // `source_gate` is a prefilter of the JSON rules (a file mentioning `except`, say): it changes no finding,
        // so a hand-written rule, which has none, replays its events.
        let written = hand_written(&check);
        let hand = written.is_some();
        let config = event.get("oracle_config").filter(|c| !c.is_null());
        let gate_only = config.and_then(|c| c.as_object()).is_some_and(|c| c.keys().all(|k| k == "source_gate"));
        if unread || (config.is_some() && !(written.is_some() && gate_only)) || event.get("path_skip_override").is_some_and(|v| v == true) {
            continue;
        }
        let production = event.get("production_scope").is_some_and(|v| v == true);
        let globs = |key: &str| event["scope"][key].as_array().into_iter().flatten().filter_map(|g| g.as_str().map(str::to_owned)).collect::<Vec<_>>();
        let scope = (!production).then(|| if globs("include").is_empty() { (vec![], vec![]) } else { (globs("include"), globs("exclude")) });
        let adjust = Adjust { scope, keep: keep(&check, event["entry"].as_str().unwrap_or(""), production) };
        let Some(rule) = rule_file(&check) else {
            misses.push(format!("{check}: no rule file"));
            continue;
        };
        // A hand-written `.rr` that lowers wins over the converted JSON.
        let program = match written {
            Some(program) => program,
            None => {
                let c = convert(&rule, &adjust);
                if c.clauses.iter().any(|(_, r)| r.is_err()) {
                    continue;
                }
                c.program
            }
        };
        row[1] += 1;
        let dir = root.join(format!("e{k}"));
        for f in event["files"].as_array().unwrap() {
            let p = dir.join(f["path"].as_str().unwrap());
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, f["source"].as_str().unwrap()).unwrap();
        }
        let out = dir.join(".out");
        // `RERAIL_REPLAY_SHOW=a,b`: relations a and b are outputs too, and their rows are printed.
        let show: Vec<String> = std::env::var("RERAIL_REPLAY_SHOW").map(|s| s.split(',').map(str::to_owned).collect()).unwrap_or_default();
        let program = show.iter().fold(program, |p, r| format!("{p}\n.output {r}\n"));
        // `RERAIL_REPLAY_EXTRA=<file>`: `.rr` rules added to every replayed program, to try a change to a rule.
        let program = match std::env::var("RERAIL_REPLAY_EXTRA") {
            Ok(f) => format!("{program}\n{}\n", std::fs::read_to_string(f).unwrap()),
            Err(_) => program,
        };
        // `RERAIL_REPLAY_PROFILE=<dir>`: each event's run profiled into <dir>/e<k> (`logic::profile`), to be summed.
        if let Ok(profiles) = std::env::var("RERAIL_REPLAY_PROFILE") {
            crate::code_rule_dsl::logic::profile::set(true);
            crate::code_rule_dsl::logic::profile::set_directory(&std::path::Path::new(&profiles).join(format!("e{k}")));
        }
        let ran = super::super::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18)
            // Every output has its file, empty or not: no `violation.csv` is a program without violations.
            .and_then(|_| std::fs::read_to_string(out.join("violation.csv")).map_err(|e| super::super::plan::Error::Plan(format!("violation.csv: {e}"))));
        let mut got: Vec<String> = match ran {
            Ok(csv) => csv
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
        for r in &show {
            let rows = std::fs::read_to_string(out.join(format!("{r}.csv"))).unwrap_or_else(|_| "(no file)".to_owned());
            eprintln!("{} {r}:\n{rows}", event["test"]);
        }
        let mut want: Vec<String> = event["findings"].as_array().unwrap().iter().map(|f| format!("{}:{}:{}", f["path"].as_str().unwrap(), f["line"], f["code"].as_str().unwrap())).collect();
        // The suite plugin (a production event) drops a hand-written rule's log-mode codes.
        if production && hand {
            let log = log_codes(&check);
            got.retain(|g| !log.iter().any(|c| g.ends_with(&format!(":{c}"))));
        }
        // Findings are a set of keys (path, line, code): a key found twice is one finding.
        got.sort();
        got.dedup();
        want.sort();
        want.dedup();
        if got == want {
            row[2] += 1;
        } else {
            misses.push(format!("{check} {}: want {want:?} got {got:?}", event["test"]));
        }
    }
    let _ = std::fs::remove_dir_all(&root);
    let mut report = String::from("check: events, all clauses converted, exact, errors\n");
    let mut sums = BTreeMap::<&str, [usize; 4]>::new();
    for ((group, check), row) in &per_check {
        report.push_str(&format!("  {group} {check}: {} {} {} {}\n", row[0], row[1], row[2], row[3]));
        for key in [group.as_str(), "all"] {
            let sum = sums.entry(key).or_default();
            for i in 0..4 {
                sum[i] += row[i];
            }
        }
    }
    for (group, sum) in &sums {
        let name = if *group == "recordings 1-6" { "total".to_owned() } else { format!("total ({group})") };
        report.push_str(&format!("{name}: {} {} {} {}\n", sum[0], sum[1], sum[2], sum[3]));
    }
    println!("{report}");
    for m in misses.iter().take(40) {
        println!("{m}");
    }
    check_baseline(&per_check, only.as_deref());
}

/// The replay's recorded result per check (`replay_baseline.txt`): `<group> | <check> | events converted exact
/// errors`. A check may not convert fewer events, give fewer exact, or more errors than recorded; a better result
/// is printed, and `RERAIL_REPLAY_BLESS=1` records it. With `RERAIL_REPLAY_CHECK`, only those checks compare.
fn check_baseline(per_check: &BTreeMap<(String, String), [usize; 4]>, only: Option<&[String]>) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl/logic/convert/replay_baseline.txt");
    let row = |line: &str| -> Option<((String, String), [usize; 4])> {
        let mut parts = line.split(" | ");
        let (group, check, numbers) = (parts.next()?, parts.next()?, parts.next()?);
        let n: Vec<usize> = numbers.split_whitespace().map(|n| n.parse().ok()).collect::<Option<_>>()?;
        Some(((group.to_owned(), check.to_owned()), n.try_into().ok()?))
    };
    let recorded: BTreeMap<_, _> = std::fs::read_to_string(&path).unwrap_or_default().lines().filter(|l| !l.starts_with('#')).filter_map(row).collect();
    if std::env::var_os("RERAIL_REPLAY_BLESS").is_some() {
        assert!(only.is_none() && std::env::var_os("RERAIL_REPLAY_EVENT").is_none(), "bless the whole replay only");
        let mut text = String::from("# converted_rules_give_the_recorded_findings, per check: events converted exact errors\n");
        for ((group, check), r) in per_check {
            text.push_str(&format!("{group} | {check} | {} {} {} {}\n", r[0], r[1], r[2], r[3]));
        }
        std::fs::write(&path, text).unwrap();
        return;
    }
    let mut worse = Vec::new();
    for (key, want) in &recorded {
        if only.is_some_and(|o| !o.contains(&key.1)) {
            continue;
        }
        let got = per_check.get(key).copied().unwrap_or_default();
        if got[1] < want[1] || got[2] < want[2] || got[3] > want[3] {
            worse.push(format!("{} {}: {:?} (recorded {want:?})", key.0, key.1, got));
        } else if got != *want {
            println!("better than recorded: {} {}: {got:?} (recorded {want:?}); RERAIL_REPLAY_BLESS=1 records it", key.0, key.1);
        }
    }
    for (key, got) in per_check {
        if !recorded.contains_key(key) {
            println!("not recorded: {} {}: {got:?}", key.0, key.1);
        }
    }
    assert!(!recorded.is_empty(), "no replay baseline at {}", path.display());
    assert!(worse.is_empty(), "worse than the replay baseline:\n{}", worse.join("\n"));
}

/// The rules `rules` (comma-separated ids, or `*`) in one corpus run: each file parsed and batched once, every rule
/// over each chunk. Prints the run as `measure_converted_rules` does, and the violations per rule.
fn measure_all(corpus: &Path, rules: &str) {
    let ids: Vec<String> = if rules == "*" {
        let mut ids: Vec<String> = std::fs::read_dir(pack().join("datalog")).unwrap().filter_map(|e| {
            let p = e.unwrap().path();
            (p.extension().is_some_and(|x| x == "rr")).then(|| p.file_stem().unwrap().to_string_lossy().into_owned())
        }).collect();
        ids.sort();
        ids
    } else {
        rules.split(',').map(str::to_owned).collect()
    };
    // A check's hand-written rule (`docs/rule-syntax/data-layer`), where it lowers, replaces its converted one, as in
    // the replay (`RERAIL_MEASURE_HAND=off`: the converted rules only).
    // `RERAIL_MEASURE_HAND=only`: the hand-written rules alone, including the checks the pack has no rule for; `skip`: the
    // converted rules of the other checks. A run binds one shape of base relations (`core.rr`'s, or T's), so the two
    // sets are two runs.
    let mode = std::env::var("RERAIL_MEASURE_HAND").unwrap_or_default();
    let hand = mode != "off";
    let texts: Vec<String> = ids
        .iter()
        .map(|id| {
            // `RERAIL_HAND_SKIP`: checks (comma-separated, as `layer-violations`) whose hand-written rule is left out.
            let skip = std::env::var("RERAIL_HAND_SKIP").unwrap_or_default();
            let skipped = |c: &str| skip.split(',').any(|s| s.trim() == c);
            let written = id.strip_prefix("data-layer-").filter(|c| hand && !skipped(c)).and_then(|c| hand_written(&c.replace('-', "_")));
            if written.is_some() {
                println!("HAND {id}");
            }
            match (mode.as_str(), written) {
                ("only", w) => w,
                ("skip", Some(_)) => None,
                (_, Some(w)) => Some(w),
                _ => Some(std::fs::read_to_string(pack().join("datalog").join(format!("{id}.rr"))).unwrap()),
            }
        })
        .flatten()
        .collect();
    let mut texts = texts;
    // The hand-written rules of checks the pack has no rule for.
    if hand && mode != "skip" && rules == "*" {
        let dir = hand_dir();
        let mut extra: Vec<String> = std::fs::read_dir(&dir).unwrap().filter_map(|e| {
            let p = e.unwrap().path();
            (p.extension().is_some_and(|x| x == "rr")).then(|| p.file_stem().unwrap().to_string_lossy().into_owned())
        // `x.tables.rr` and `x.symbols.rr` are tables a policy includes, not checks.
        }).filter(|c| !c.contains('.') && !ids.contains(&format!("data-layer-{c}"))).filter(|c| !std::env::var("RERAIL_HAND_SKIP").unwrap_or_default().split(',').any(|s| s.trim() == c)).collect();
        extra.sort();
        for c in extra {
            // Repository inputs are read (`corpus::inputs`), `input_error` rows included.
            match hand_written(&c.replace('-', "_")) {
                Some(t) => {
                    println!("HAND {c} (no converted rule)");
                    texts.push(t);
                }
                None => {}
            }
        }
    }
    let programs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let out = crate::code_rule_dsl::prefilter::tests::temp().with_extension("measure-all");
    if let Some(dir) = std::env::var_os("RERAIL_PROFILE") {
        crate::code_rule_dsl::logic::profile::set_directory(&PathBuf::from(dir).join("all"));
    }
    let r = super::super::plan::corpus::run_all(&programs, corpus, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let cpu: f64 = r.phases.iter().map(|p| p.1 + p.2).sum();
    let rss = r.phases.iter().map(|p| p.3).fold(0.0, f64::max);
    println!(
        "MEASURE all {} rules: {} files, {} nodes, {} chunks, violations {}, CPU {cpu:.2} s, peak RSS {rss:.0} MB; text tests {} relations, {} strings, {:.3} s CPU",
        programs.len(), r.files, r.nodes, r.chunks, r.rows.get("violation").copied().unwrap_or(0), r.text.0, r.text.1, r.text.2
    );
    for (name, user, sys, rss, statements) in &r.phases {
        println!("  {name:<16} {:.2} s CPU  rss {rss:.0} MB  {statements} statements", user + sys);
    }
    // Violations per rule: the first column of `violation.csv` is the rule's id.
    let mut per: BTreeMap<String, usize> = BTreeMap::new();
    for line in std::fs::read_to_string(out.join("violation.csv")).unwrap_or_default().lines() {
        let id = line.split('\t').next().unwrap_or("").trim_matches('"').to_owned();
        *per.entry(id).or_default() += 1;
    }
    for (id, n) in &per {
        println!("VIOLATIONS {id} {n}");
    }
    let _ = std::fs::remove_dir_all(&out);
}

/// A measurement, not a check: converted rules (`RERAIL_MEASURE_RULES`, comma-separated rule ids) run over a corpus
/// on disk (`RERAIL_MEASURE_CORPUS`), one run each. Prints each run's CPU (user + system) by phase, peak RSS, the
/// violations, and the text tests' work (decision 0037): relations, distinct strings tested, their CPU.
#[test]
#[ignore = "a measurement over a corpus on disk"]
fn measure_converted_rules() {
    let corpus = PathBuf::from(std::env::var("RERAIL_MEASURE_CORPUS").expect("RERAIL_MEASURE_CORPUS"));
    let rules = std::env::var("RERAIL_MEASURE_RULES").expect("RERAIL_MEASURE_RULES");
    // `RERAIL_MEASURE_ALL=1`: the rules in one run (`corpus::run_all`), `*` for every rule of the pack.
    if std::env::var_os("RERAIL_MEASURE_ALL").is_some() {
        return measure_all(&corpus, &rules);
    }
    for id in rules.split(',') {
        let program = std::fs::read_to_string(pack().join("datalog").join(format!("{id}.rr"))).unwrap();
        // The relations the cross-file part derives, and the base relations it reads.
        let mut parsed = crate::code_rule_dsl::logic::rr::parse(&program).unwrap();
        crate::code_rule_dsl::logic::strings::as_relations(&mut parsed).unwrap();
        let split = crate::code_rule_dsl::typing::file_local::programs(&parsed);
        let cross: BTreeSet<&str> = split.cross.rules.iter().map(|r| r.head.rel.as_str()).collect();
        let local = crate::code_rule_dsl::typing::file_local::split(&parsed);
        let not_local: BTreeSet<&str> = parsed.rules.iter().map(|r| r.head.rel.as_str()).filter(|r| !local.contains(*r)).collect();
        println!("CROSS {id}: derives {cross:?}, not file-local {not_local:?}, reads base {:?}", split.base);
        let out = crate::code_rule_dsl::prefilter::tests::temp().with_extension(format!("measure-{id}"));
        // `RERAIL_PROFILE=<dir>`: each rule's profile in `<dir>/<id>` (`logic::profile`).
        if let Some(dir) = std::env::var_os("RERAIL_PROFILE") {
            crate::code_rule_dsl::logic::profile::set_directory(&PathBuf::from(dir).join(id));
        }
        // `RERAIL_MEASURE_BINDING=metal`: the file-local FIX on the device work queue.
        let binding = match std::env::var("RERAIL_MEASURE_BINDING").as_deref() {
            Ok("metal") => crate::code_rule_dsl::typing::file_local::Binding::Metal,
            _ => crate::code_rule_dsl::typing::file_local::Binding::Cpu,
        };
        let r = super::super::plan::corpus::run(&program, &corpus, None, &out, "program", binding, 1 << 18).unwrap();
        let cpu: f64 = r.phases.iter().map(|p| p.1 + p.2).sum();
        let rss = r.phases.iter().map(|p| p.3).fold(0.0, f64::max);
        println!(
            "MEASURE {id}: {} files, {} nodes, {} chunks, violations {}, CPU {cpu:.2} s, peak RSS {rss:.0} MB; text tests {} relations, {} strings, {:.3} s CPU",
            r.files, r.nodes, r.chunks, r.rows.get("violation").copied().unwrap_or(0), r.text.0, r.text.1, r.text.2
        );
        for (name, user, sys, rss, statements) in &r.phases {
            println!("  {name:<16} {:.2} s CPU  rss {rss:.0} MB  {statements} statements", user + sys);
        }
        let _ = std::fs::remove_dir_all(&out);
    }
}
