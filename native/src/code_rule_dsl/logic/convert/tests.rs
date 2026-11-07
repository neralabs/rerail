//! The converter's run over the data-layer pack, and checks of what it writes.

use super::*;
use crate::code_rule_dsl::CodeRuleDslRuntime;
use crate::code_rule_dsl::model::{ClauseKind, Query};
use crate::code_rule_dsl::test_support::CAPABILITIES;
use std::path::{Path, PathBuf};

mod reference_tests;

/// The rules converted: the pack as rewritten by Pulling-it-all-together (`rules-ast`, decision 0037 step 1).
const RULES: &str = "rules-ast";

fn pack() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../gate/data-layer/packs/data-layer/candidates")
}

/// The authored overlay's files (`convert` module doc): `<rule-id>.rr`.
fn authored() -> PathBuf {
    pack().join("datalog").join("authored")
}

/// What one rule's conversion gave: its program, and per clause whether it converted or is authored, and why not.
struct Converted {
    program: String,
    clauses: Vec<(String, Result<Origin, Gap>)>,
    approx: BTreeSet<String>,
    /// Whether the run's options differ from the policy's own (its config, else its defaults), or its clauses were
    /// edited: what an authored file, written for the policy as it is, does not follow.
    configured: bool,
    /// The clauses whose Python matchers carry `when` (an option condition), with those conditions: what they
    /// convert to holds for this run's options only.
    when: BTreeMap<String, Vec<String>>,
}

/// Why a rule did not convert: the run's configuration does not apply to it, or its authored file is wrong.
#[derive(Debug)]
enum Failure {
    Config(String),
    Authored(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Config(e) => write!(f, "the configuration does not apply: {e}"),
            Failure::Authored(e) => f.write_str(e),
        }
    }
}

fn scope_globs(document: &serde_json::Value, key: &str) -> Vec<String> {
    document.get("scope").and_then(|s| s.get(key)).and_then(|v| v.as_array()).into_iter().flatten().filter_map(|g| g.as_str().map(str::to_owned)).collect()
}

/// The policy's reference graph configuration: the sibling links and constructor fields of its `calls { graph:
/// references }` terms and relational `references` inputs, each distinct one once.
fn reference_graphs(document: &serde_json::Value) -> Vec<serde_json::Map<String, serde_json::Value>> {
    fn walk(value: &serde_json::Value, out: &mut Vec<serde_json::Map<String, serde_json::Value>>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, inner) in map {
                    let graph = match (key.as_str(), inner.as_object()) {
                        ("calls", Some(calls)) if calls.get("graph").and_then(|g| g.as_str()) == Some("references") => Some(calls),
                        ("references", Some(references)) if references.contains_key("language") => Some(references),
                        _ => None,
                    };
                    if let Some(graph) = graph {
                        let config: serde_json::Map<_, _> = ["siblingLinks", "constructorFields"].iter().filter_map(|k| graph.get(*k).map(|v| ((*k).to_owned(), v.clone()))).collect();
                        if !out.contains(&config) {
                            out.push(config);
                        }
                    }
                    walk(inner, out);
                }
            }
            serde_json::Value::Array(items) => items.iter().for_each(|item| walk(item, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(document, &mut out);
    out
}

/// How a replay event adjusts a rule: its scope, which clauses it keeps, and an edit of the policy document before it
/// compiles (option values, clause edits) that says whether it edited clauses, or why it cannot apply.
#[derive(Default)]
struct Adjust {
    scope: Option<(Vec<String>, Vec<String>)>,
    keep: Option<Box<dyn Fn(&str) -> bool>>,
    policy: Option<Box<dyn Fn(&mut serde_json::Value) -> Result<bool, String>>>,
}

/// The leaves of a JSON value, each at its JSON pointer (RFC 6901: `/a/0`, `~` and `/` in keys as `~0`, `~1`).
fn json_leaves(value: &Json, pointer: String, out: &mut Vec<(String, Json)>) {
    match value {
        Json::Object(members) => {
            for (key, member) in members {
                json_leaves(member, format!("{pointer}/{}", key.replace('~', "~0").replace('/', "~1")), out);
            }
        }
        Json::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                json_leaves(item, format!("{pointer}/{i}"), out);
            }
        }
        leaf => out.push((pointer, leaf.clone())),
    }
}

/// A policy's option values, clause by clause: its config's, else the option's default.
fn option_values(document: &serde_json::Value) -> Vec<(usize, String, serde_json::Value)> {
    let mut out = Vec::new();
    for (i, clause) in document["clauses"].as_array().into_iter().flatten().enumerate() {
        let define = &clause["define"];
        for (name, option) in define["options"].as_object().into_iter().flatten() {
            let value = define["config"].get(name).unwrap_or(&option["default"]);
            out.push((i, name.clone(), value.clone()));
        }
    }
    out
}

/// One rule file as a program: its scope as path globs and each forbid clause's matcher as rules deriving
/// `violation(rule, clause, code, path, line)`, under the rule's authored file (`authored()`) where it has one. An
/// error is the authored file's.
fn convert(path: &Path, adjust: &Adjust) -> Result<Converted, Failure> {
    convert_over(path, adjust, Some(&authored()))
}

/// `convert` with the authored files in `authored`, or none.
fn convert_over(path: &Path, adjust: &Adjust, authored: Option<&Path>) -> Result<Converted, Failure> {
    let text = std::fs::read_to_string(path).unwrap();
    let mut document: serde_json::Value = serde_json::from_str(&text).unwrap();
    let original = document.clone();
    let edited = match &adjust.policy {
        Some(edit) => edit(&mut document).map_err(Failure::Config)?,
        None => false,
    };
    let configured = edited || option_values(&original) != option_values(&document);
    // The reference graph the corpus runner binds (`logic::plan::corpus::references`): the policy's, one per policy.
    let graphs = reference_graphs(&document);
    if graphs.len() > 1 {
        return Err(Failure::Config(format!("{} configures more than one reference graph", document["id"])));
    }
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
    let text = if relational.is_empty() && document == original { text } else { document.to_string() };
    // Only relational clauses report: nothing is left to compile.
    let reports = document["clauses"].as_array().into_iter().flatten().any(|c| c.get("define").is_none());
    let compiled = match reports.then(|| CodeRuleDslRuntime::compile(text, "json".to_owned(), CAPABILITIES.to_owned())) {
        Some(Ok(c)) => Some(c),
        Some(Err(e)) if configured => return Err(Failure::Config(format!("{id} does not compile so configured: {}", e.reason))),
        Some(Err(e)) => panic!("{id}: {e:?}"),
        None => None,
    };
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
    // The report gate (`report_when: {count: {gt: k}}`, once, on a define clause, as the run configures it): a fact
    // the authored file reads; the converter's own clauses cannot be gated by it, which counts every clause's
    // findings, so in a gated policy they are gaps.
    let report_when_gt = document["clauses"].as_array().into_iter().flatten().find_map(|c| c["define"]["report_when"]["count"]["gt"].as_u64());
    e.decls.insert("report_when_gt".into(), "k: int".into());
    if let Some(k) = report_when_gt {
        e.facts.push(format!("report_when_gt({k})."));
    }
    // The options as this run resolves them, for the authored file (`GIVEN`): booleans that hold, array items,
    // other values as text, and the leaves of JSON values (an object or array, or a string holding one, as
    // `parameters` does) by JSON pointer, as W's `json_extract` reads them.
    e.decls.insert("option_on".into(), "name: sym".into());
    e.decls.insert("option_item".into(), "name: sym, i: int, v: sym".into());
    e.decls.insert("option_value".into(), "name: sym, v: sym".into());
    e.decls.insert("option_json".into(), "name: sym, pointer: sym, v: sym".into());
    let as_text = |v: &Json| v.as_str().map_or_else(|| v.to_string(), str::to_owned);
    let mut option_facts = BTreeSet::new();
    for (_, name, value) in option_values(&document) {
        match &value {
            Json::Bool(true) => {
                option_facts.insert(format!("option_on({}).", quote(&name)));
            }
            Json::Bool(false) | Json::Null => {}
            Json::Array(items) => option_facts.extend(items.iter().enumerate().map(|(i, item)| format!("option_item({}, {i}, {}).", quote(&name), quote(&as_text(item))))),
            other => {
                option_facts.insert(format!("option_value({}, {}).", quote(&name), quote(&as_text(other))));
            }
        }
        let json = match &value {
            Json::String(s) => serde_json::from_str::<Json>(s).ok().filter(|j| j.is_object() || j.is_array()),
            Json::Object(_) | Json::Array(_) => Some(value.clone()),
            _ => None,
        };
        let mut leaves = Vec::new();
        if let Some(json) = &json {
            json_leaves(json, String::new(), &mut leaves);
        }
        option_facts.extend(leaves.into_iter().map(|(pointer, leaf)| format!("option_json({}, {}, {}).", quote(&name), quote(&pointer), quote(&as_text(&leaf)))));
    }
    e.facts.extend(option_facts);
    // The clauses whose Python matchers carry `when`: their conversion follows this run's options only.
    let when: BTreeMap<String, Vec<String>> = document["clauses"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let ast = &c["forbid"]["ast"];
            let matchers: Vec<&Json> = ast.as_array().map_or_else(|| vec![ast], |a| a.iter().collect());
            let conditions: Vec<String> = matchers.iter().filter(|m| m["language"] == "python").filter_map(|m| m.get("when")).map(Json::to_string).collect();
            (!conditions.is_empty()).then(|| (c["id"].as_str().unwrap_or_default().to_owned(), conditions))
        })
        .collect();
    let mut clauses: Vec<(String, Result<Origin, Gap>)> = relational
        .into_iter()
        .filter(|c| adjust.keep.as_ref().is_none_or(|keep| keep(c)))
        .map(|c| (c, Err(Gap("relational (SQL stages)".into()))))
        .collect();
    for clause in compiled.iter().flat_map(|c| &c.inner.clauses) {
        if adjust.keep.as_ref().is_some_and(|keep| !keep(&clause.id)) {
            continue;
        }
        // A clause that fails leaves nothing it emitted.
        let result = e.clause(|e| -> Res<()> {
            let matchers = match &clause.kind {
                ClauseKind::Forbid(Query::Ast { matchers, .. }) => matchers,
                ClauseKind::Forbid(Query::Regex { .. }) => return gap("a regex over file content or paths"),
                ClauseKind::Forbid(Query::SelectWhen { .. }) => return gap("select/when"),
                ClauseKind::RequireEach { .. } => return gap("require each (define)"),
                ClauseKind::Budget { .. } => return gap("budget"),
                _ => return gap("another clause kind"),
            };
            let mut pythons = matchers.iter().filter(|m| m.language == GrammarDialect::Python);
            let Some(grammar) = pythons.next() else { return gap("no Python matcher") };
            // W reads a file with the first matcher whose `when` and file condition hold (`eval::query::grammar_matcher`).
            if pythons.next().is_some() {
                return gap("several Python matchers (the first that applies to a file)");
            }
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
            if report_when_gt.is_some() {
                return gap("report_when (a gate on the count of all the rule's findings)");
            }
            e.rules.push(format!("violation({}, {}, {}, $p, {line}) :- {}.", quote(&id), quote(&clause.id), quote(&clause.violation.code), body.join(", ")));
            Ok(())
        });
        clauses.push((clause.id.clone(), result.map(|()| Origin::Converted)));
    }
    clauses.sort_by_key(|(c, _)| order.iter().position(|o| o == c));
    // The emitter's text, in `.rr`: every program the converter gives is Soufflé form (decision 0043).
    let text = crate::code_rule_dsl::logic::rr::translate(&e.program(&["violation"])).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut prog = crate::code_rule_dsl::logic::rr::parse(&text).unwrap_or_else(|e| panic!("{}: {e:?}", path.display()));
    // The `.references` setting: the graph's configuration over the run's scope.
    if let Some(mut graph) = graphs.into_iter().next() {
        graph.insert("scope".into(), serde_json::json!({"include": include, "exclude": exclude}));
        prog.settings.push(("references".into(), serde_json::Value::Object(graph).to_string()));
    }
    // The authored file's clauses over the converter's, with only what `violation` reaches.
    let file = authored.map(|dir| dir.join(format!("{id}.rr"))).filter(|f| f.exists());
    let prog = if let Some(file) = file {
        let authored = crate::code_rule_dsl::logic::rr::parse_file(&file).map_err(|e| Failure::Authored(format!("{}: {e:?}", file.display())))?;
        let (prog, given) = overlay(prog, authored, &id, &order, &|c| adjust.keep.as_ref().is_none_or(|keep| keep(c))).map_err(|e| Failure::Authored(format!("{}: {e}", file.display())))?;
        for (clause, result) in &mut clauses {
            if given.contains(clause) {
                *result = Ok(Origin::Authored);
            }
        }
        prog
    } else {
        reachable(prog)
    };
    let program = crate::code_rule_dsl::logic::rr::print(&prog).map_err(|e| Failure::Authored(format!("{id}: the merged program does not print: {e:?}")))?;
    Ok(Converted { program, clauses, approx: e.approx.clone(), configured, when })
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
/// it lowers and the check has no authored file (`authored()`, which follows the oracle's configurations: on
/// layer-violations, missing-async-pairs, post-op-no-internal-dispatch, public-api-triples and swallowed-exceptions
/// it gives 107 of 107 events, the hand-written rules 97 of 98 and none of the 9 configured).
/// `RERAIL_REPLAY_HAND_WRITTEN` runs the hand-written rule over an authored file; `RERAIL_REPLAY_CONVERTED` replays
/// the converted JSON (and its authored file) instead.
fn hand_written(check: &str) -> Option<String> {
    let overlay = authored().join(format!("data-layer-{}.rr", check.replace('_', "-")));
    if std::env::var_os("RERAIL_REPLAY_CONVERTED").is_some() || (overlay.exists() && std::env::var_os("RERAIL_REPLAY_HAND_WRITTEN").is_none()) {
        return None;
    }
    let path = crate::code_rule_dsl::logic::rr::library().join(format!("../data-layer/{}.rr", check.replace('_', "-")));
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

/// The converter's run: every rule of the pack as a program in `candidates/datalog/<rule>.rr` (its authored file
/// merged), and the coverage (`coverage.txt`): clauses converted or authored, the gaps by kind, the approximations.
#[test]
#[ignore = "writes the converted rules into the pack"]
fn convert_the_data_layer_pack() {
    let out = pack().join("datalog");
    std::fs::create_dir_all(&out).unwrap();
    let (mut total, mut converted, mut authored, mut by_options) = (0, 0, 0, 0);
    let mut gaps = BTreeMap::<String, usize>::new();
    let mut approx = BTreeMap::<String, usize>::new();
    let mut per_rule = Vec::new();
    for path in rule_files() {
        let id = path.file_stem().unwrap().to_string_lossy().into_owned();
        let c = convert(&path, &Adjust::default()).unwrap_or_else(|e| panic!("{e}"));
        crate::code_rule_dsl::logic::rr::parse(&c.program).unwrap_or_else(|e| panic!("{id}: {e:?}\n{}", c.program));
        std::fs::write(out.join(format!("{id}.rr")), &c.program).unwrap();
        let ok = c.clauses.iter().filter(|(_, r)| r.is_ok()).count();
        let by_hand = c.clauses.iter().filter(|(_, r)| *r == Ok(Origin::Authored)).count();
        total += c.clauses.len();
        converted += ok - by_hand;
        authored += by_hand;
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
        per_rule.push(format!("{id}: {ok}/{} (authored {by_hand})", c.clauses.len()));
        for (clause, r) in &c.clauses {
            match r {
                Err(Gap(g)) => {
                    let g: String = g.chars().take(160).collect();
                    per_rule.push(format!("    {clause}: {}", g.replace('\n', " ")));
                }
                // Converted for the default options: under others the clause says something else.
                Ok(Origin::Converted) if c.when.contains_key(clause) => {
                    by_options += 1;
                    per_rule.push(format!("    {clause}: converted for the default options only (when {}): author it on option_on/option_item", c.when[clause].join(", ")));
                }
                _ => {}
            }
        }
    }
    let mut report = format!(
        "{}/{total} clauses converted or authored ({converted} converted, {by_options} of them for the default options only; {authored} authored)\n\ngaps (clauses):\n",
        converted + authored
    );
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

/// One policy's conversion (`RERAIL_CONVERT_POLICY`: its file, or its rule id), under its authored file: each
/// clause converted, authored or a gap, printed, and the program, printed or written to `RERAIL_CONVERT_OUT`.
#[test]
#[ignore = "converts one policy; RERAIL_CONVERT_POLICY, --ignored --nocapture"]
fn convert_one_policy() {
    let policy = std::env::var("RERAIL_CONVERT_POLICY").expect("RERAIL_CONVERT_POLICY");
    let path = Some(PathBuf::from(&policy))
        .filter(|p| p.exists())
        .or_else(|| rule_files().into_iter().find(|p| p.file_stem().is_some_and(|s| s.to_string_lossy() == policy)))
        .unwrap_or_else(|| panic!("{policy}: no policy file or rule id"));
    let c = convert(&path, &Adjust::default()).unwrap_or_else(|e| panic!("{e}"));
    let ok = c.clauses.iter().filter(|(_, r)| r.is_ok()).count();
    let authored = c.clauses.iter().filter(|(_, r)| *r == Ok(Origin::Authored)).count();
    println!("{ok}/{} clauses converted or authored (authored {authored})", c.clauses.len());
    for (clause, r) in &c.clauses {
        match r {
            Ok(Origin::Converted) if c.when.contains_key(clause) => println!("  {clause}: converted for the default options only (when {})", c.when[clause].join(", ")),
            Ok(Origin::Converted) => println!("  {clause}: converted"),
            Ok(Origin::Authored) => println!("  {clause}: authored"),
            Err(Gap(g)) => println!("  {clause}: gap: {}", g.chars().take(160).collect::<String>()),
        }
    }
    for a in &c.approx {
        println!("  approx  {a}");
    }
    match std::env::var("RERAIL_CONVERT_OUT") {
        Ok(out) => std::fs::write(out, &c.program).unwrap(),
        Err(_) => println!("{}", c.program),
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

type Json = serde_json::Value;

/// A finding as the replay compares it: (path, line, code), the code `None` where the oracle records none.
type Key = (String, u64, Option<String>);

/// One event the replay runs: an upstream linter call on files, and what it found.
struct Event {
    /// `recordings 1-6` (the baseline the 60 rules were converted against), `batches 7-8 and probes`, or
    /// `batches 9-10 API records`.
    group: &'static str,
    check: String,
    test: String,
    files: Vec<(String, String)>,
    /// The oracle's findings, or why they cannot be compared.
    want: Result<Vec<Key>, String>,
    /// Whether the oracle compares more than the keys (messages, metadata, categories, columns).
    keys_only: bool,
    /// The rule as the oracle ran it, or why it cannot be configured so.
    adjust: Result<Adjust, String>,
    /// Whether the oracle ran the linter unconfigured (a `source_gate` aside: a prefilter of the JSON rules, which
    /// changes no finding), its own path skips on and only the keys compared: what a hand-written rule, which has
    /// no options, replays.
    plain: bool,
}

fn port() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../gate/data-layer/fixtures/host/data-layer-port")
}

/// Every event of the oracle (`fixtures/host/data-layer-port`): the recordings and probes, then the API records of
/// batches 9 and 10.
fn events() -> Vec<Event> {
    let mut out = recorded_events();
    out.extend(api_events());
    out
}

/// The recorded events (the upstream linters' tests), as `replay.ts` reads them: every batch's recording and probes.
fn recorded_events() -> Vec<Event> {
    let first = |name: &str| (1..=6).any(|b| name == format!("batch-{b:02}-recording.json"));
    let events = |name: &str| name.starts_with("batch-") && (name.ends_with("-recording.json") || name.ends_with("probes.json"));
    let mut paths: Vec<PathBuf> = std::fs::read_dir(port())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(events))
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for p in paths {
        let recording: Json = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let group = if first(&p.file_name().unwrap().to_string_lossy()) { "recordings 1-6" } else { "batches 7-8 and probes" };
        for event in recording["events"].as_array().into_iter().flatten() {
            if event.get("retained_reason").is_some() || event["upstream_passed"] != Json::Bool(true) {
                continue;
            }
            let files = if event.get("files").is_none_or(Json::is_null) { &recording["files"] } else { &event["files"] };
            let files = files.as_array().into_iter().flatten().map(|f| (f["path"].as_str().unwrap().to_owned(), f["source"].as_str().unwrap().to_owned())).collect();
            let check = event["check"].as_str().unwrap().to_owned();
            let entry = event["entry"].as_str().unwrap_or_default();
            let truthy = |k: &str| event.get(k).is_some_and(|v| !v.is_null() && v != false && v.as_array().is_none_or(|a| !a.is_empty()));
            // `replay.ts` compares messages, metadata fields and categories where asked, and the backend checks' columns.
            let keys_only = truthy("compare_message") || truthy("compare_fields") || truthy("compare_clause") || check == "backend_type_strings";
            let want = Ok(event["findings"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|f| (f["path"].as_str().unwrap().to_owned(), f["line"].as_u64().unwrap_or_default(), f["code"].as_str().map(str::to_owned)))
                .collect());
            let adjust = recorded_adjust(event, &check, entry, event["production_scope"] == true);
            let config = event.get("oracle_config").filter(|c| !c.is_null());
            let gate_only = config.is_none_or(|c| c.as_object().is_some_and(|c| c.keys().all(|k| k == "source_gate")));
            let plain = gate_only && !truthy("compare_message") && !truthy("compare_fields") && event["path_skip_override"] != true;
            out.push(Event { group, test: event["test"].as_str().unwrap_or_default().to_owned(), check, files, want, keys_only, adjust, plain });
        }
    }
    out
}

/// A recorded event's rule as `replay.ts` runs it: outside production its scope is the event's and `oracleAnalyzer`
/// is on; the oracle's `source_gate`, `categories` (`category-<name>` options, each name in its first category) and
/// `options` set the options the policy declares, and, as in `replay.ts`, an oracle setting the policy declares no
/// option for is left out (the rule has no such switch: `lazy_architecture`'s `oracleAnalyzer`, say); the
/// escape-hatch analyzer calls drop `report_when`; and a `path_skip_override` (the test switched off the linter's
/// own path skips, which are the regex clauses' `paths`, not the scope) drops those `paths`. A signed allowlist is
/// checked by the host, not the rule: that is not replayed.
fn recorded_adjust(event: &Json, check: &str, entry: &str, production: bool) -> Result<Adjust, String> {
    let oracle = &event["oracle_config"];
    if oracle.get("allowlist_signature").is_some_and(|v| !v.is_null()) {
        return Err("a signed allowlist (the host's signature guard)".into());
    }
    let globs = |key: &str| event["scope"][key].as_array().into_iter().flatten().filter_map(|g| g.as_str().map(str::to_owned)).collect::<Vec<_>>();
    let scope = (!production).then(|| if globs("include").is_empty() { (vec![], vec![]) } else { (globs("include"), globs("exclude")) });
    let source_gate = oracle.get("source_gate").filter(|v| !v.is_null()).cloned();
    // No name is in two categories in the recordings, so the order categories are read in does not matter.
    let categories = oracle.get("categories").and_then(Json::as_object).map(|categories| {
        let mut assigned = BTreeSet::new();
        let mut out = BTreeMap::new();
        for (category, names) in categories {
            let names: Vec<Json> = names.as_array().into_iter().flatten().filter(|n| assigned.insert(n.to_string())).cloned().collect();
            out.insert(category.clone(), Json::Array(names));
        }
        out
    });
    let options = oracle.get("options").and_then(Json::as_object).cloned().unwrap_or_default();
    let report_when = check == "sql_grammar_escape_hatches" && entry != "plugin.reporter";
    let path_skip = event["path_skip_override"] == true;
    let policy = move |document: &mut Json| -> Result<bool, String> {
        let mut edited = false;
        for clause in document.get_mut("clauses").and_then(Json::as_array_mut).into_iter().flatten() {
            if path_skip && clause.get_mut("forbid").and_then(|f| f.get_mut("regex")).and_then(Json::as_object_mut).is_some_and(|r| r.remove("paths").is_some()) {
                edited = true;
            }
            let Some(define) = clause.get_mut("define").and_then(Json::as_object_mut) else { continue };
            if report_when && define.remove("report_when").is_some() {
                edited = true;
            }
            let Some(declares) = define.get("options").and_then(Json::as_object).cloned() else { continue };
            let mut config = define.get("config").and_then(Json::as_object).cloned().unwrap_or_default();
            if declares.contains_key("oracleAnalyzer") {
                config.insert("oracleAnalyzer".into(), Json::Bool(!production));
            }
            if let Some(gate) = source_gate.as_ref().filter(|_| declares.contains_key("sourceGate")) {
                config.insert("sourceGate".into(), gate.clone());
            }
            if let Some(categories) = &categories {
                for name in declares.keys().filter(|n| n.starts_with("category-")) {
                    config.insert(name.clone(), categories.get(&name["category-".len()..]).cloned().unwrap_or_else(|| Json::Array(Vec::new())));
                }
            }
            for (name, value) in options.iter().filter(|(name, _)| declares.contains_key(*name)) {
                config.insert(name.clone(), value.clone());
            }
            if !config.is_empty() {
                define.insert("config".into(), Json::Object(config));
            }
        }
        Ok(edited)
    };
    Ok(Adjust { scope, keep: keep(check, entry, production), policy: Some(Box::new(policy)) })
}

/// The API records of batches 9 and 10 (`batch-nine.ts`, `batch-ten.ts`): each linter entry point's result on its
/// files, the test passed, each distinct record once.
fn api_events() -> Vec<Event> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for name in ["batch-09-complete-api.json", "batch-10-complete-api.json.gz", "batch-10-file-contract-regressions.json", "batch-10-repair-regressions.json"] {
        let path = port().join(name);
        let bytes = if name.ends_with(".gz") {
            let output = std::process::Command::new("gzip").arg("-dc").arg(&path).output().unwrap();
            assert!(output.status.success(), "gzip -dc {}", path.display());
            output.stdout
        } else {
            std::fs::read(&path).unwrap()
        };
        let oracle: Json = serde_json::from_slice(&bytes).unwrap();
        let records = oracle.get("records").or_else(|| oracle.get("cases").filter(|c| c.is_array()));
        let passed = |test: &str| oracle["cases"].get(test).and_then(Json::as_array).is_none_or(|phases| phases.iter().all(|p| p["outcome"] == "passed"));
        for record in records.and_then(Json::as_array).into_iter().flatten() {
            let test = record["test"].as_str().unwrap_or_default().to_owned();
            let mut content = record.clone();
            content.as_object_mut().unwrap().remove("test");
            if !passed(&test) || !seen.insert(content.to_string()) {
                continue;
            }
            let check = record["check"].as_str().unwrap().to_owned();
            let entry = record["entry"].as_str().unwrap_or_default().to_owned();
            let files: Option<Vec<(String, String)>> = record["files"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|f| {
                    let source = f["source"].as_str().or_else(|| f["sourceSha256"].as_str().and_then(|s| oracle["sources"][s].as_str()));
                    source.map(|s| (f["path"].as_str().unwrap().to_owned(), s.to_owned()))
                })
                .collect();
            let (want, files) = match files {
                Some(files) => (api_want(&check, &entry, &record["expected"], &files), files),
                None => (Err("a file whose source is not recorded".into()), Vec::new()),
            };
            let adjust = Ok(api_adjust(&check, &entry, &record["parameters"], record["production"] == true));
            out.push(Event { group: "batches 9-10 API records", check, test, files, want, keys_only: true, adjust, plain: false });
        }
    }
    out
}

/// An API record's rule as `batch-nine.ts` and `batch-ten.ts` run it: outside production, every clause's options
/// configured with the entry point and the parameters, and every file in scope.
fn api_adjust(check: &str, entry: &str, parameters: &Json, production: bool) -> Adjust {
    if production {
        return Adjust::default();
    }
    let entry = Json::String(if check == "connection_service_architecture" && entry == "lint" { "all".into() } else { entry.into() });
    let parameters = Json::String(parameters.to_string());
    let policy = move |document: &mut Json| -> Result<bool, String> {
        for clause in document.get_mut("clauses").and_then(Json::as_array_mut).into_iter().flatten() {
            let Some(define) = clause.get_mut("define").and_then(Json::as_object_mut) else { continue };
            if define.get("options").is_none_or(Json::is_null) {
                continue;
            }
            let mut config = define.get("config").and_then(Json::as_object).cloned().unwrap_or_default();
            config.insert("entryPoint".into(), entry.clone());
            config.insert("parameters".into(), parameters.clone());
            define.insert("config".into(), Json::Object(config));
        }
        Ok(false)
    };
    Adjust { scope: Some((vec!["**/*".into()], Vec::new())), keep: None, policy: Some(Box::new(policy)) }
}

/// An API record's result as keys: rows of a path (`path`, `file_path`, `file`, or the one file), a line (`line`,
/// `line_number`) and a code (`code`, `rule`, `rule_id`) where it has one; or why it is not findings.
fn api_want(check: &str, entry: &str, expected: &Json, files: &[(String, String)]) -> Result<Vec<Key>, String> {
    let Some(rows) = expected.as_array() else { return Err("the oracle's result is an exception, not findings".into()) };
    rows.iter()
        .map(|row| {
            let Some(row) = row.as_object() else { return Err("findings recorded as text, not rows".into()) };
            let text = |keys: &[&str]| keys.iter().find_map(|k| row.get(*k).and_then(Json::as_str)).map(str::to_owned);
            let path = match text(&["path", "file_path", "file"]) {
                // The scan reports its temporary copy: `tmp/embedding_vector_orchestration_scan/<digest>/<path>`.
                Some(p) if check == "embedding_vector_orchestration" && entry == "scan_source" => {
                    p.strip_prefix("tmp/embedding_vector_orchestration_scan/").and_then(|r| r.split_once('/')).map_or(p.clone(), |(_, rest)| rest.to_owned())
                }
                Some(p) => p,
                None if files.len() == 1 => files[0].0.clone(),
                None => return Err("a finding without a path".into()),
            };
            let Some(line) = ["line", "line_number"].iter().find_map(|k| row.get(*k).and_then(Json::as_u64)) else { return Err("a finding without a line".into()) };
            Ok((path, line, text(&["code", "rule", "rule_id"])))
        })
        .collect()
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

/// What the replay counts, per group and check.
#[derive(Default)]
struct Row {
    events: usize,
    /// Events it can run: a rule file, comparable findings, a configuration that applies.
    replayable: usize,
    /// Replayable events whose kept clauses are all converted or authored, run.
    run: usize,
    exact: usize,
    /// Of the exact, those whose oracle compares more than (path, line, code), which the replay does not.
    keys_only: usize,
    errors: usize,
    /// Run events configured otherwise than the policy is, with an authored clause, which follows the policy's own
    /// options: their mismatches are listed apart.
    config_authored: usize,
    config_authored_exact: usize,
    /// Run events whose oracle (or program) gave one key more than once: findings are a set of keys (J1), so they
    /// count once.
    collapsed: usize,
    /// Why the other events were not run, by reason.
    not_run: BTreeMap<String, usize>,
}

impl Row {
    fn add(&mut self, o: &Row) {
        self.events += o.events;
        self.replayable += o.replayable;
        self.run += o.run;
        self.exact += o.exact;
        self.keys_only += o.keys_only;
        self.errors += o.errors;
        self.config_authored += o.config_authored;
        self.config_authored_exact += o.config_authored_exact;
        self.collapsed += o.collapsed;
        for (reason, n) in &o.not_run {
            *self.not_run.entry(reason.clone()).or_default() += n;
        }
    }

    fn line(&self) -> String {
        let mut line = format!(
            "events {}, replayable {}, run {}, exact {} (keys only {}), errors {}, config+authored {} (exact {}), duplicates collapsed {}",
            self.events, self.replayable, self.run, self.exact, self.keys_only, self.errors, self.config_authored, self.config_authored_exact, self.collapsed
        );
        if !self.not_run.is_empty() {
            let reasons: Vec<String> = self.not_run.iter().map(|(r, n)| format!("{r} {n}")).collect();
            line.push_str(&format!("; not run: {}", reasons.join(", ")));
        }
        line
    }
}

/// The converted rules, with their authored files, on every event of the oracle (`events`), each configured as its
/// oracle ran (`recorded_adjust`, `api_adjust`); a check with a hand-written rule that lowers (`hand_written`) runs
/// that instead, on its unconfigured events (`Event::plain`), unless `RERAIL_REPLAY_CONVERTED` is set. Per group and
/// check: the events, those it can replay, those whose kept clauses are all converted or authored (run), how many
/// give the oracle's findings exactly as a set of (path, line, code) keys (J1: a key found twice is one; the events
/// where that collapsed some are counted), and why the rest were not run; then per check the clauses still gapped,
/// the first misses, and the mismatches of configured events with authored clauses. `RERAIL_REPLAY_CHECK` (check
/// names) and `RERAIL_REPLAY_RULE` (rule ids), comma-separated, select; `RERAIL_REPLAY_EVENT` keeps the events whose
/// name contains it; `RERAIL_REPLAY_MISSES` is how many misses are printed (40); `RERAIL_REPLAY_SHOW` and
/// `RERAIL_REPLAY_EXTRA` print relations' rows and add rules (below).
#[test]
#[ignore = "runs every recorded event through the corpus runner"]
fn converted_rules_give_the_recorded_findings() {
    let list = |var: &str| std::env::var(var).ok().map(|s| s.split(',').map(str::to_owned).collect::<Vec<_>>());
    let (checks, rules) = (list("RERAIL_REPLAY_CHECK"), list("RERAIL_REPLAY_RULE"));
    let selected = |check: &str| {
        (checks.is_none() && rules.is_none())
            || checks.as_ref().is_some_and(|c| c.iter().any(|c| c == check))
            || rules.as_ref().is_some_and(|r| r.iter().any(|r| *r == format!("data-layer-{}", check.replace('_', "-"))))
    };
    let shown: usize = std::env::var("RERAIL_REPLAY_MISSES").ok().map_or(40, |n| n.parse().expect("RERAIL_REPLAY_MISSES: a count"));
    let mut per_check = BTreeMap::<(&'static str, String), Row>::new();
    let mut gapped = BTreeMap::<String, BTreeSet<String>>::new();
    let (mut misses, mut config_misses) = (Vec::new(), Vec::new());
    let root = crate::code_rule_dsl::prefilter::tests::temp().with_extension("replay");
    for (k, event) in events().into_iter().enumerate() {
        if !selected(&event.check) {
            continue;
        }
        // `RERAIL_REPLAY_EVENT=<text>`: only the events whose name contains it.
        if std::env::var("RERAIL_REPLAY_EVENT").is_ok_and(|e| !event.test.contains(&e)) {
            continue;
        }
        let row = per_check.entry((event.group, event.check.clone())).or_default();
        row.events += 1;
        let mut skip = |reason: &str| *row.not_run.entry(reason.chars().take(120).collect()).or_default() += 1;
        let Some(rule) = rule_file(&event.check) else {
            skip("no rule file");
            continue;
        };
        let want = match &event.want {
            Ok(want) => want,
            Err(reason) => {
                skip(reason);
                continue;
            }
        };
        // A hand-written `.rr` that lowers wins over the converted JSON. It has no options, so it replays the events
        // the oracle ran unconfigured (`Event::plain`).
        let written = hand_written(&event.check);
        let (program, config_authored) = match written {
            Some(_) if !event.plain => {
                skip("configured (the hand-written rule has no options)");
                continue;
            }
            Some(program) => {
                row.replayable += 1;
                (program, false)
            }
            None => {
                let adjust = match &event.adjust {
                    Ok(adjust) => adjust,
                    Err(reason) => {
                        skip(reason);
                        continue;
                    }
                };
                let c = match convert(&rule, adjust) {
                    Ok(c) => c,
                    Err(Failure::Config(reason)) => {
                        skip(&reason);
                        continue;
                    }
                    Err(e @ Failure::Authored(_)) => {
                        row.replayable += 1;
                        row.errors += 1;
                        misses.push(format!("{} {}: error {e}", event.check, event.test));
                        continue;
                    }
                };
                row.replayable += 1;
                let gaps: Vec<&String> = c.clauses.iter().filter(|(_, r)| r.is_err()).map(|(id, _)| id).collect();
                if !gaps.is_empty() {
                    *row.not_run.entry("clauses gapped".to_owned()).or_default() += 1;
                    gapped.entry(event.check.clone()).or_default().extend(gaps.into_iter().cloned());
                    continue;
                }
                let config_authored = c.configured && c.clauses.iter().any(|(_, r)| *r == Ok(Origin::Authored));
                (c.program, config_authored)
            }
        };
        row.run += 1;
        if config_authored {
            row.config_authored += 1;
        }
        let dir = root.join(format!("e{k}"));
        write_files(&dir, event.files.iter().map(|(p, s)| (p.as_str(), s.as_str())));
        // `RERAIL_REPLAY_SHOW=a,b`: relations a and b are outputs too, and their rows are printed.
        let show: Vec<String> = std::env::var("RERAIL_REPLAY_SHOW").map(|s| s.split(',').map(str::to_owned).collect()).unwrap_or_default();
        let program = show.iter().fold(program, |p, r| format!("{p}\n.output {r}\n"));
        // `RERAIL_REPLAY_EXTRA=<file>`: `.rr` rules added to every replayed program, to try a change to a rule.
        let program = match std::env::var("RERAIL_REPLAY_EXTRA") {
            Ok(f) => format!("{program}\n{}\n", std::fs::read_to_string(f).unwrap()),
            Err(_) => program,
        };
        let got = match finding_keys(&program, &dir) {
            Ok(got) => got,
            Err(e) => {
                row.errors += 1;
                misses.push(format!("{} {}: error {e}", event.check, event.test));
                continue;
            }
        };
        for r in &show {
            let rows = std::fs::read_to_string(dir.join(".out").join(format!("{r}.csv"))).unwrap_or_else(|_| "(no file)".to_owned());
            eprintln!("{} {r}:\n{rows}", event.test);
        }
        // Where the oracle records no code, (path, line) only.
        let codeless = want.iter().any(|(_, _, code)| code.is_none());
        let key = |path: &str, line: u64, code: Option<&str>| match code {
            Some(code) if !codeless => format!("{path}:{line}:{code}"),
            _ => format!("{path}:{line}"),
        };
        let mut want: Vec<String> = want.iter().map(|(p, l, c)| key(p, *l, c.as_deref())).collect();
        let mut got: Vec<String> = got.iter().map(|(p, l, c)| key(p, *l, Some(c))).collect();
        // Findings are a set of keys (J1): several of one clause on one line are one.
        let n = (want.len(), got.len());
        want.sort();
        want.dedup();
        got.sort();
        got.dedup();
        row.collapsed += usize::from(n != (want.len(), got.len()));
        if got == want {
            row.exact += 1;
            row.keys_only += usize::from(event.keys_only || codeless);
            row.config_authored_exact += usize::from(config_authored);
        } else {
            let miss = format!("{} {}: want {want:?} got {got:?}", event.check, event.test);
            if config_authored { config_misses.push(miss) } else { misses.push(miss) }
        }
    }
    let _ = std::fs::remove_dir_all(&root);
    let mut report = String::new();
    let mut sums = BTreeMap::<&str, Row>::new();
    for ((group, check), row) in &per_check {
        report.push_str(&format!("  {group} {check}: {}\n", row.line()));
        for key in [*group, "all"] {
            sums.entry(key).or_default().add(row);
        }
    }
    for (group, sum) in &sums {
        report.push_str(&format!("total ({group}): {}\n", sum.line()));
    }
    report.push_str("\nclauses still gapped, by check:\n");
    for (check, clauses) in &gapped {
        report.push_str(&format!("  {check}: {}\n", clauses.iter().cloned().collect::<Vec<_>>().join(", ")));
    }
    println!("{report}");
    println!("misses: {} (the first {})", misses.len(), shown.min(misses.len()));
    for m in misses.iter().take(shown) {
        println!("{m}");
    }
    println!("\nconfigured events with authored clauses that miss: {} (the first {})", config_misses.len(), shown.min(config_misses.len()));
    for m in config_misses.iter().take(shown) {
        println!("{m}");
    }
}

/// Writes `files` (path, source) under `dir`, which is made though there are none (an event of no files is an empty
/// corpus).
fn write_files<'a>(dir: &Path, files: impl IntoIterator<Item = (&'a str, &'a str)>) {
    std::fs::create_dir_all(dir).unwrap();
    for (path, source) in files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, source).unwrap();
    }
}

/// A program's findings over the files under `dir` (the corpus runner), each `path:line:code`.
fn findings(program: &str, dir: &Path) -> Result<Vec<String>, String> {
    Ok(finding_keys(program, dir)?.into_iter().map(|(path, line, code)| format!("{path}:{line}:{code}")).collect())
}

/// A program's findings over the files under `dir`: (path, line, code).
fn finding_keys(program: &str, dir: &Path) -> Result<Vec<(String, u64, String)>, String> {
    let out = dir.join(".out");
    super::super::plan::corpus::run(program, dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).map_err(|e| e.to_string())?;
    Ok(std::fs::read_to_string(out.join("violation.csv"))
        .unwrap_or_default()
        .lines()
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f[3].to_owned(), f[4].parse().unwrap_or_default(), f[2].to_owned())
        })
        .collect())
}

/// `has` filters its ancestor, the node it is at, by that node's kinds, not the descendant's: the executor's typed
/// parameters (a `has` within the `parameters` field) and the tag conditions (a `has` from the condition) are
/// found as W finds them.
#[test]
fn has_filters_the_ancestor_by_its_own_kinds() {
    let path = "l2070_batching/executor.py";
    let source = "def run(step: ServerPlanStep, other: int):\n    if step_tags(step):\n        pass\n    if \"x\" in step_tags(step):\n        pass\n";
    let rule = rule_file("serverplan_target_architecture").unwrap();
    let c = convert_over(&rule, &Adjust::default(), None).unwrap();
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("has-ancestor");
    let _ = std::fs::remove_dir_all(&dir);
    write_files(&dir, [(path, source)]);
    let mut got = findings(&c.program, &dir).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    got.sort();
    // W, on the same source: its findings of the clauses converted (its lines 0-based).
    let compiled = CodeRuleDslRuntime::compile(std::fs::read_to_string(&rule).unwrap(), "json".to_owned(), CAPABILITIES.to_owned()).unwrap();
    let result = crate::code_rule_dsl::test_support::evaluate_file(&compiled, "python", path, source);
    let converted: BTreeSet<&str> = c.clauses.iter().filter(|(_, r)| r.is_ok()).map(|(id, _)| id.as_str()).collect();
    let mut w: Vec<String> = result["violations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["clauseId"].as_str().is_some_and(|id| converted.contains(id)))
        .map(|v| format!("{path}:{}:{}", v["span"]["startLine"].as_u64().unwrap() + 1, v["code"].as_str().unwrap()))
        .collect();
    w.sort();
    let want = [format!("{path}:1:SPTA006"), format!("{path}:2:SPTA007"), format!("{path}:4:SPTA007")];
    assert_eq!(w, want, "W");
    assert_eq!(got, want);
}

/// A converted rule's findings on one file, and W's of the clauses converted (its lines 0-based), each sorted.
fn converted_and_w(rule: &Path, path: &str, source: &str, name: &str) -> (Vec<String>, Vec<String>) {
    let c = convert_over(rule, &Adjust::default(), None).unwrap();
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension(name);
    let _ = std::fs::remove_dir_all(&dir);
    write_files(&dir, [(path, source)]);
    let mut got = findings(&c.program, &dir).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let compiled = CodeRuleDslRuntime::compile(std::fs::read_to_string(rule).unwrap(), "json".to_owned(), CAPABILITIES.to_owned()).unwrap();
    let result = crate::code_rule_dsl::test_support::evaluate_file(&compiled, "python", path, source);
    let converted: BTreeSet<&str> = c.clauses.iter().filter(|(_, r)| r.is_ok()).map(|(id, _)| id.as_str()).collect();
    let mut w: Vec<String> = result["violations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["clauseId"].as_str().is_some_and(|id| converted.contains(id)))
        .map(|v| format!("{path}:{}:{}", v["span"]["startLine"].as_u64().unwrap() + 1, v["code"].as_str().unwrap()))
        .collect();
    for keys in [&mut got, &mut w] {
        keys.sort();
        keys.dedup();
    }
    (got, w)
}

/// A metavariable bound around a helper relation (an `any` branch, a `not`) is a column of it, passed in: here the
/// call's argument must be the name the assignment binds (`$X`), as W finds, not any name: `x = f(x)` is SAME, and
/// `y = f(z)`, whose argument is not `$X`, is OTHER (the negation). The constructor rule's "parameter named
/// `$PARAM`" was such a helper.
#[test]
fn helpers_take_the_metavariables_bound_around_them() {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("metavariables");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let rule = dir.join("probe.json");
    let same = serde_json::json!({"any": [{"pattern": "$X"}, {"kind": "call", "has": {"field": "arguments", "stopBy": "neighbor", "has": {"pattern": "$X", "stopBy": "neighbor"}}}]});
    let clause = |id: &str, code: &str, right: Json| {
        serde_json::json!({"id": id, "forbid": {"ast": {"language": "python", "rule": {"all": [
            {"kind": "assignment"},
            {"has": {"field": "left", "stopBy": "neighbor", "pattern": "$X"}},
            {"has": {"field": "right", "stopBy": "neighbor", "all": [{"kind": "call"}, right]}},
        ]}}}, "violation": {"anchor": "witness.span", "code": code, "message": id}})
    };
    let policy = serde_json::json!({"format": "rerail.policy/v1", "id": "probe", "revision": 1, "mode": "block", "scope": {"include": ["**/*.py"], "exclude": []},
        "clauses": [clause("same", "SAME", same.clone()), clause("other", "OTHER", serde_json::json!({"not": same}))]});
    std::fs::write(&rule, policy.to_string()).unwrap();
    let (got, w) = converted_and_w(&rule, "a.py", "x = f(x)\ny = f(z)\n", "metavariables-run");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(w, ["a.py:1:SAME", "a.py:2:OTHER"], "W");
    assert_eq!(got, w);
}

/// A clause that fails after emitting relations takes them back: the emitter is as it was before the clause.
#[test]
fn a_failed_clause_leaves_nothing() {
    let mut e = Emitter::new(GrammarDialect::Python);
    e.clause(|e| {
        e.rule("in_scope($d)", "d: int", vec!["doc($d, _, _)".into()]);
        Ok(())
    })
    .unwrap();
    let before = e.program(&["violation"]);
    let r = e.clause(|e| {
        let m = e.relation(vec![vec!["node(_, $n, \"call\", _, _)".to_owned()], vec!["node(_, $n, \"attribute\", _, _)".to_owned()]], 0, "$n");
        e.kindset(&[e.kinds.kind_id("call")].into())?;
        e.approx.insert("an approximation".into());
        gap(format!("after {m}"))
    });
    assert!(r.is_err());
    assert_eq!(e.program(&["violation"]), before);
    assert!(e.approx.is_empty());
}

/// Every converted program holds only what `violation` reaches: no rule, declaration, input or kind set of a
/// clause that failed, or of a branch it did not use. Over every rule of the pack, many with failed clauses.
#[test]
fn converted_programs_hold_only_what_violation_reaches() {
    let mut failed = 0;
    for path in rule_files() {
        let c = convert_over(&path, &Adjust::default(), None).unwrap();
        failed += c.clauses.iter().filter(|(_, r)| r.is_err()).count();
        let prog = crate::code_rule_dsl::logic::rr::parse(&c.program).unwrap();
        let (mut reached, mut todo) = (BTreeSet::new(), vec!["violation".to_owned()]);
        while let Some(rel) = todo.pop() {
            if reached.insert(rel.clone()) {
                todo.extend(prog.rules.iter().filter(|r| r.head.rel == rel).flat_map(|r| r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.clone())));
            }
        }
        let id = path.display();
        for r in &prog.rules {
            assert!(reached.contains(&r.head.rel), "{id}: {} is not reached", r.head.rel);
        }
        for rel in prog.decls.keys().chain(&prog.inputs) {
            assert!(reached.contains(rel), "{id}: {rel} is declared and not reached");
        }
        let sets: BTreeSet<String> = reached.iter().filter_map(|r| r.strip_prefix(crate::code_rule_dsl::logic::front::AXIS)).map(str::to_owned).chain(prog.rules.iter().flat_map(|r| &r.body).filter_map(|l| match l {
            crate::code_rule_dsl::logic::front::Literal::Nearest { kindset, .. } => Some(kindset.clone()),
            _ => None,
        })).collect();
        for set in prog.kindsets.keys() {
            assert!(sets.contains(set), "{id}: kind set {set} is not read");
        }
    }
    assert!(failed > 0, "some clause failed");
}

/// The target architecture rule, its executor fixture, and an authored file for it: `executor-parameters` (which
/// converts) written again as an `if` in a function named `run`, and `required-contracts` (SQL stages) as the
/// module's first line.
const TARGET: &str = "serverplan_target_architecture";
const TARGET_ID: &str = "data-layer-serverplan-target-architecture";
const EXECUTOR: (&str, &str) = ("l2070_batching/executor.py", "def run(step: ServerPlanStep, other: int):\n    if step_tags(step):\n        pass\n    if \"x\" in step_tags(step):\n        pass\n");
const TARGET_AUTHORED: &str = r#"
.decl doc(d: number, path: symbol, module: symbol)
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl field(p: node, f: symbol, c: node)
.decl text(n: node, s: symbol)
.decl line(n: node, l: number)
.decl in_scope(d: number)
.decl violation(rule: symbol, clause: symbol, code: symbol, path: symbol, line: number)
.output violation
.decl executor_parameters_def(kind: symbol)
executor_parameters_def("function_definition").
.decl executor_parameters_run(f: node)
executor_parameters_run(f) :- node(_, f, "function_definition", _, _), field(f, "name", x), text(x, "run").
violation("data-layer-serverplan-target-architecture", "executor-parameters", "SPTA006", p, l) :-
    node(d, s, "if_statement", _, _), inside(s, f, executor_parameters_def), executor_parameters_run(f), in_scope(d), doc(d, p, _), line(s, l).
.decl required_contracts_module(d: number)
required_contracts_module(d) :- node(d, _, "module", _, _).
violation("data-layer-serverplan-target-architecture", "required-contracts", "SPTA001", p, 1) :- required_contracts_module(d), in_scope(d), doc(d, p, _).
"#;

/// An authored clause replaces the converter's, a relational one is given, and a clause the run leaves out is
/// dropped with its helpers.
#[test]
fn authored_clauses_replace_the_converters() {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("authored");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{TARGET_ID}.rr")), TARGET_AUTHORED).unwrap();
    let rule = rule_file(TARGET).unwrap();
    let c = convert_over(&rule, &Adjust::default(), Some(&dir)).unwrap();
    let origin = |c: &Converted, id: &str| c.clauses.iter().find(|(c, _)| c == id).map(|(_, r)| r.clone());
    assert_eq!(origin(&c, "executor-parameters"), Some(Ok(Origin::Authored)));
    assert_eq!(origin(&c, "required-contracts"), Some(Ok(Origin::Authored)));
    assert_eq!(origin(&c, "tag-conditions"), Some(Ok(Origin::Converted)));
    assert_eq!(c.program.lines().filter(|l| l.contains("\"executor-parameters\"")).count(), 1, "{}", c.program);
    let run = dir.join("run");
    write_files(&run, [EXECUTOR]);
    let mut got = findings(&c.program, &run).unwrap();
    got.sort();
    let at = |l: usize, code: &str| format!("{}:{l}:{code}", EXECUTOR.0);
    assert_eq!(got, [at(1, "SPTA001"), at(2, "SPTA006"), at(2, "SPTA007"), at(4, "SPTA006"), at(4, "SPTA007")]);
    let without = Adjust { keep: Some(Box::new(|c: &str| c != "executor-parameters")), ..Adjust::default() };
    let c = convert_over(&rule, &without, Some(&dir)).unwrap();
    assert_eq!(origin(&c, "executor-parameters"), None);
    assert!(!c.program.contains("executor-parameters") && !c.program.contains("executor_parameters_"), "{}", c.program);
    let _ = std::fs::remove_dir_all(&dir);
}

/// What an authored file may not do is a named error.
#[test]
fn overlay_errors_are_named() {
    let rule = rule_file(TARGET).unwrap();
    let c = convert_over(&rule, &Adjust::default(), None).unwrap();
    let converted = crate::code_rule_dsl::logic::rr::parse(&c.program).unwrap();
    let clauses: Vec<String> = c.clauses.iter().map(|(c, _)| c.clone()).collect();
    let head = ".decl node(d: number, n: node, kind: symbol, start: number, end: number)\n.decl doc(d: number, path: symbol, module: symbol)\n.decl in_scope(d: number)\n.decl violation(rule: symbol, clause: symbol, code: symbol, path: symbol, line: number)\n";
    let violation = |rule: &str, clause: &str, body: &str| format!("violation({}, {}, \"X\", p, 1) :- {body}, node(d, n, _, _, _), doc(d, p, _).\n", quote(rule), quote(clause));
    let ok = |body: &str| violation(TARGET_ID, "executor-parameters", body);
    let cases = [
        (format!("{head}.decl m5(n: node)\nm5(n) :- node(_, n, \"call\", _, _).\n{}", ok("m5(n)")), "a name the converter makes"),
        (format!("{head}.decl k1(kind: symbol)\nk1(\"call\").\n{}", ok("node(_, n, \"call\", _, _), inside(n, a, k1)")), "kind set k1 has a name the converter makes"),
        (format!("{head}{}", violation("data-layer-other", "executor-parameters", "in_scope(d)")), "a violation of rule"),
        (format!("{head}{}", violation(TARGET_ID, "no-such-clause", "in_scope(d)")), "which the policy has not"),
        (format!("{}.decl text(n: node)\n{}", head, ok("text(n)")), "other columns than the converter's"),
        (format!("{head}in_scope(d) :- doc(d, _, _).\n{}", ok("in_scope(d)")), "read it only"),
        (format!("{head}{}", ok("executor_parameters_undeclared(n)")), "is read and not declared"),
        (format!("{head}.decl executor_parameters_empty(n: node)\n{}", ok("executor_parameters_empty(n)")), "nothing derives or inputs it"),
        (format!("{head}.decl report_when_gt(k: number)\nreport_when_gt(3).\n{}", ok("report_when_gt(_)")), "read it only"),
    ];
    for (text, error) in cases {
        let authored = crate::code_rule_dsl::logic::rr::parse(&text).unwrap_or_else(|e| panic!("{e:?}\n{text}"));
        match overlay(converted.clone(), authored, TARGET_ID, &clauses, &|_| true) {
            Ok(_) => panic!("no error, wanted {error:?}:\n{text}"),
            Err(e) => assert!(e.contains(error), "{e}, wanted {error:?}:\n{text}"),
        }
    }
}

/// The report gate: `report_when_gt(k)` holds `k` where the policy, as the run configures it, has
/// `report_when: {count: {gt: k}}`, and nothing where the run drops it; an authored file gates on it, and the
/// converter leaves a gated policy's clauses to the file.
#[test]
fn the_report_gate_is_given() {
    const ID: &str = "data-layer-sql-grammar-escape-hatches";
    let authored = format!(
        r#"
.decl doc(d: number, path: symbol, module: symbol)
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl line(n: node, l: number)
.decl in_scope(d: number)
.decl report_when_gt(k: number)
.decl violation(rule: symbol, clause: symbol, code: symbol, path: symbol, line: number)
.decl usage_0_witness(p: symbol, l: number)
usage_0_witness(p, l) :- node(d, n, "call", _, _), in_scope(d), doc(d, p, _), line(n, l).
.decl usage_0_quiet(k: number)
usage_0_quiet(k) :- report_when_gt(k), c = count : {{ usage_0_witness(_, _) }}, c <= k.
violation("{ID}", "usage-0", "escape_hatch", p, l) :- usage_0_witness(p, l), !usage_0_quiet(_).
"#
    );
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("report-gate");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{ID}.rr")), authored).unwrap();
    let run = dir.join("run");
    write_files(&run, [("impl/a.py", "f()\ng()\n")]);
    let rule = rule_file("sql_grammar_escape_hatches").unwrap();
    let everywhere = || Some((vec!["**/*".to_owned()], Vec::new()));
    // As the policy is: the gate is 16, two findings are under it, and the converter gives no clause of its own.
    let gated = convert_over(&rule, &Adjust { scope: everywhere(), ..Adjust::default() }, Some(&dir)).unwrap();
    assert!(gated.program.contains("report_when_gt(16)."), "{}", gated.program);
    assert!(gated.clauses.iter().all(|(c, r)| c == "usage-0" || r.is_err()), "{:?}", gated.clauses);
    assert_eq!(findings(&gated.program, &run).unwrap(), Vec::<String>::new());
    // As the escape-hatch analyzer calls run it, without the gate: the declaration and no row.
    let drop: Box<dyn Fn(&mut Json) -> Result<bool, String>> = Box::new(|document| {
        for clause in document["clauses"].as_array_mut().unwrap() {
            if let Some(define) = clause.get_mut("define").and_then(Json::as_object_mut) {
                define.remove("report_when");
            }
        }
        Ok(true)
    });
    let open = convert_over(&rule, &Adjust { scope: everywhere(), policy: Some(drop), ..Adjust::default() }, Some(&dir)).unwrap();
    assert!(open.program.contains(".decl report_when_gt(k: number)") && !open.program.contains("report_when_gt(16)"), "{}", open.program);
    let mut got = findings(&open.program, &run).unwrap();
    got.sort();
    assert_eq!(got, ["impl/a.py:1:escape_hatch", "impl/a.py:2:escape_hatch"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The options, as the run resolves them, are given to the authored file (`option_on`, `option_item`), and a clause
/// whose matcher is under `when: {option: …}` is marked: `data-layer-transaction-patterns`' `category-1` is only
/// for `oracleAnalyzer` on (off at the defaults), and lists its names in `category-LEGITIMATE`.
#[test]
fn the_options_are_given() {
    const ID: &str = "data-layer-transaction-patterns";
    let rule = rule_file("transaction_patterns").unwrap();
    let plain = convert_over(&rule, &Adjust::default(), None).unwrap();
    assert_eq!(plain.clauses.iter().find(|(c, _)| c == "category-1").map(|(_, r)| r.clone()), Some(Ok(Origin::Converted)));
    assert_eq!(plain.when.get("category-1").map(Vec::as_slice), Some(&[r#"{"option":{"oracleAnalyzer":true}}"#.to_owned()][..]));
    let authored = format!(
        r#"
.decl doc(d: number, path: symbol, module: symbol)
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl field(p: node, f: symbol, c: node)
.decl text(n: node, s: symbol)
.decl line(n: node, l: number)
.decl in_scope(d: number)
.decl option_on(name: symbol)
.decl option_item(name: symbol, i: number, v: symbol)
.decl violation(rule: symbol, clause: symbol, code: symbol, path: symbol, line: number)
.decl category_1_def(n: node, s: symbol)
category_1_def(n, s) :- node(_, n, "function_definition", _, _), field(n, "name", x), text(x, s).
violation("{ID}", "category-1", "TEST", p, l) :- option_on("oracleAnalyzer"), category_1_def(n, s), option_item("category-LEGITIMATE", _, s), node(d, n, _, _, _), in_scope(d), doc(d, p, _), line(n, l).
"#
    );
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("options");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{ID}.rr")), authored).unwrap();
    let everywhere = || Some((vec!["**/*".to_owned()], Vec::new()));
    // Each run in a directory of its own: a second corpus run over one output directory in a process decodes
    // its symbols with the first run's atoms.
    let tests = |program: &str, name: &str| {
        let run = dir.join(name);
        write_files(&run, [("impl/a.py", "def __enter__(self):\n    pass\ndef __exit__(self):\n    pass\n")]);
        findings(program, &run).unwrap().into_iter().filter(|f| f.ends_with(":TEST")).collect::<Vec<_>>()
    };
    // The defaults: `oracleAnalyzer` off, `category-LEGITIMATE` its default list.
    let defaults = convert_over(&rule, &Adjust { scope: everywhere(), ..Adjust::default() }, Some(&dir)).unwrap();
    let enter_item = |program: &str| program.lines().any(|l| l.starts_with(r#"option_item("category-LEGITIMATE", "#) && l.ends_with(r#", "__enter__")."#));
    assert!(enter_item(&defaults.program), "{}", defaults.program);
    assert!(!defaults.program.contains(r#"option_on("oracleAnalyzer")."#), "{}", defaults.program);
    assert_eq!(tests(&defaults.program, "defaults"), Vec::<String>::new());
    // As an analyzer event configures it: `oracleAnalyzer` on, the category one name.
    let configure: Box<dyn Fn(&mut Json) -> Result<bool, String>> = Box::new(|document| {
        for clause in document["clauses"].as_array_mut().unwrap() {
            let Some(define) = clause.get_mut("define").and_then(Json::as_object_mut) else { continue };
            if define.get("options").is_some_and(|o| o.get("oracleAnalyzer").is_some()) {
                define.insert("config".into(), serde_json::json!({"oracleAnalyzer": true, "category-LEGITIMATE": ["__exit__"]}));
            }
        }
        Ok(false)
    });
    let analyzer = convert_over(&rule, &Adjust { scope: everywhere(), policy: Some(configure), ..Adjust::default() }, Some(&dir)).unwrap();
    assert!(analyzer.configured);
    assert!(analyzer.program.contains(r#"option_on("oracleAnalyzer")."#) && !enter_item(&analyzer.program), "{}", analyzer.program);
    assert_eq!(tests(&analyzer.program, "analyzer"), ["impl/a.py:3:TEST"]);
    // JSON options, by pointer, as W's `json_extract` reads `parameters`.
    let mut leaves = Vec::new();
    json_leaves(&serde_json::json!({"connectionPath": "a", "lineCounts": {"x/y.py": 3}, "probe": [true]}), String::new(), &mut leaves);
    let leaves: Vec<(String, String)> = leaves.into_iter().map(|(p, v)| (p, v.as_str().map_or_else(|| v.to_string(), str::to_owned))).collect();
    let want = [("/connectionPath", "a"), ("/lineCounts/x~1y.py", "3"), ("/probe/0", "true")].map(|(p, v)| (p.to_owned(), v.to_owned()));
    assert_eq!(leaves, want);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A measurement, not a check: converted rules (`RERAIL_MEASURE_RULES`, comma-separated rule ids) run over a corpus
/// on disk (`RERAIL_MEASURE_CORPUS`), one run each. Prints each run's CPU (user + system) by phase, peak RSS, the
/// violations, and the text tests' work (decision 0037): relations, distinct strings tested, their CPU.
#[test]
#[ignore = "a measurement over a corpus on disk"]
fn measure_converted_rules() {
    let corpus = PathBuf::from(std::env::var("RERAIL_MEASURE_CORPUS").expect("RERAIL_MEASURE_CORPUS"));
    let rules = std::env::var("RERAIL_MEASURE_RULES").expect("RERAIL_MEASURE_RULES");
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

/// A Python module that exercises the python library (`docs/rule-syntax/lib/python/*.rr`): definitions of every
/// parameter kind, decorators, a class, imports, chained comparisons and assignments, `if`/`elif`/`else`.
const LIBRARY_FIXTURE: (&str, &str) = (
    "pkg/sub/mod.py",
    "\"\"\"Module doc.\"\"\"\nimport os\nfrom . import sibling\nfrom ..pkg.mod import thing as alias\n\nX = 1\n\n\n@decorator(arg)\ndef f(a, /, b: int = 0, *c: str, d, **e) -> None:\n    \"\"\"Doc.\"\"\"\n    global X\n    x = a\n    x = x + 1\n    if a < b <= c:\n        return x\n    elif a or b or d:\n        pass\n    else:\n        g()\n    return os.path.join(x)\n\n\nclass C(Base, metaclass=M):\n    def m(self, y):\n        def inner():\n            return self.m(y)\n        return inner()\n\n\ndef g():\n    f(1, d=2)\n\n\nlam = lambda q, r=1: q\ns = f\"x{X}\"\nt = u = 3\ndel X\n",
);

/// The python library's relations through an overlay that includes them (`#include "python/ast.rr"`,
/// `"python/calls.rr"`): for each relation the closure declares, an overlay whose one clause reads it, converted,
/// merged and run by the corpus runner over `LIBRARY_FIXTURE`. Prints, per relation, its rows' count or the error.
#[test]
#[ignore = "a report: which library relations lower and run through an overlay"]
fn library_relations_through_an_overlay_report() {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("library-report");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // One corpus directory per run: a run's store is its own.
    let run = |i: usize| {
        let run = dir.join(format!("run-{i}"));
        write_files(&run, [LIBRARY_FIXTURE]);
        run
    };
    let includes = "#include \"python/ast.rr\"\n#include \"python/calls.rr\"\n";
    std::fs::write(dir.join("closure.rr"), includes).unwrap();
    let closure = crate::code_rule_dsl::logic::rr::parse_file(&dir.join("closure.rr"));
    let closure = match closure {
        Ok(p) => p,
        Err(e) => panic!("the closure does not lower: {e}"),
    };
    let rule = rule_file(TARGET).unwrap();
    for (i, (rel, cols)) in closure.decls.iter().filter(|(rel, _)| !rel.starts_with(crate::code_rule_dsl::logic::front::AXIS) && !closure.inputs.contains(rel)).enumerate() {
        let args = vec!["_"; cols.len()].join(", ");
        let authored = format!(
            "{includes}.decl doc(d: number, path: symbol, module: symbol)\n.decl in_scope(d: number)\n\
             .decl violation(rule: symbol, clause: symbol, code: symbol, path: symbol, line: number)\n\
             violation(\"{TARGET_ID}\", \"executor-parameters\", \"{rel}\", p, 1) :- {rel}({args}), in_scope(d), doc(d, p, _).\n"
        );
        std::fs::write(dir.join(format!("{TARGET_ID}.rr")), authored).unwrap();
        let only = Adjust { keep: Some(Box::new(|c: &str| c == "executor-parameters")), scope: Some((vec!["**/*".to_owned()], Vec::new())), ..Adjust::default() };
        let result = convert_over(&rule, &only, Some(&dir)).map_err(|e| e.to_string()).and_then(|c| findings(&c.program, &run(i)));
        let _ = std::fs::remove_dir_all(dir.join(format!("run-{i}")));
        match result {
            Ok(rows) => println!("REL {rel}: {}", if rows.iter().any(|r| r.ends_with(&format!(":{rel}"))) { "rows" } else { "no rows" }),
            Err(e) => println!("REL {rel}: ERROR {}", e.lines().next().unwrap_or_default()),
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// An overlay that includes the python library (`#include "python/ast.rr"`: names, imports, flow, and calls
/// beside it) lowers, merges and runs through the corpus runner: its relations read core.rr's (`kind`, `within`,
/// `docof`, `unset`, `next_sibling`, `op_at`, `earlier`, `visit_index`, …) with no declaration of them, and give
/// on `LIBRARY_FIXTURE` what the library says: a chained comparison's links, each parameter's kind, the last
/// assignment before a read in CPython's order, owners' dotted paths, calls' names.
#[test]
fn an_overlay_includes_the_python_library() {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("library-overlay");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let authored = format!(
        r#"
#include "python/ast.rr"
#include "python/calls.rr"
.decl doc(d: number, path: symbol, module: symbol)
.decl line(n: node, l: number)
.decl in_scope(d: number)
.decl violation(rule: symbol, clause: symbol, code: symbol, path: symbol, line: number)
.decl library_at(n: node, p: symbol, l: number)
library_at(n, p, l) :- docof(n, d), in_scope(d), doc(d, p, _), line(n, l).
violation("{TARGET_ID}", "executor-parameters", cat("cmp ", o), p, l) :- cmp_pair(c, _, o, _, _), library_at(c, p, l).
violation("{TARGET_ID}", "executor-parameters", cat("param ", cat(x, cat(" ", k))), p, l) :- sig_param(_, _, nm, _, k, _, _), text(nm, x),
    library_at(nm, p, l).
violation("{TARGET_ID}", "executor-parameters", cat("latest ", cat(x, cat(" ", to_string(la)))), p, l) :- latest_assign(n, x, a),
    line(a, la), library_at(n, p, l).
violation("{TARGET_ID}", "executor-parameters", cat("owner ", s), p, l) :- owner_path(f, s), library_at(f, p, l).
violation("{TARGET_ID}", "executor-parameters", cat("call ", x), p, l) :- call_name(c, x), library_at(c, p, l).
"#
    );
    std::fs::write(dir.join(format!("{TARGET_ID}.rr")), authored).unwrap();
    let run = dir.join("run");
    write_files(&run, [LIBRARY_FIXTURE]);
    let only = Adjust { keep: Some(Box::new(|c: &str| c == "executor-parameters")), scope: Some((vec!["**/*".to_owned()], Vec::new())), ..Adjust::default() };
    let c = convert_over(&rule_file(TARGET).unwrap(), &only, Some(&dir)).unwrap_or_else(|e| panic!("{e}"));
    let mut got = findings(&c.program, &run).unwrap_or_else(|e| panic!("{e}\n{}", c.program));
    got.sort();
    let path = LIBRARY_FIXTURE.0;
    let mut want: Vec<String> = [
        (15, "cmp <"),
        (15, "cmp <="),
        (10, "param a posonly"),
        (10, "param b pos"),
        (10, "param c vararg"),
        (10, "param d kwonly"),
        (10, "param e kwarg"),
        (25, "param self pos"),
        (25, "param y pos"),
        (35, "param q pos"),
        (35, "param r pos"),
        (14, "latest x 13"),
        (16, "latest x 14"),
        (21, "latest x 14"),
        (36, "latest X 6"),
        (10, "owner f"),
        (25, "owner C.m"),
        (26, "owner C.m.inner"),
        (31, "owner g"),
        (9, "call decorator"),
        (20, "call g"),
        (21, "call join"),
        (27, "call m"),
        (28, "call inner"),
        (32, "call f"),
    ]
    .iter()
    .map(|(l, code)| format!("{path}:{l}:{code}"))
    .collect();
    want.sort();
    assert_eq!(got, want);
    let _ = std::fs::remove_dir_all(&dir);
}

/// An authored clause with `match_all` and `match_group` (on one match, its column joined to a node's, and under a
/// disjunction) survives the merge and its printing: the merged program prints them as the built-ins (not as
/// relations `match_all_`), reads back to the same program, and the runner gives the clause's findings from it.
#[test]
fn authored_match_built_ins_print_and_run() {
    let re = r"step_tags\\((?P<arg>\\w+)\\)";
    let authored = format!(
        r#"
.decl doc(d: number, path: symbol, module: symbol)
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl source(d: number, sp: span)
.decl tape_line(d: number, l: number, sp: span)
.decl text(n: loc, s: symbol)
.decl line(n: loc, l: number)
.decl col(n: loc, c: number)
.decl in_scope(d: number)
.decl violation(rule: symbol, clause: symbol, code: symbol, path: symbol, line: number)
violation("{TARGET_ID}", "executor-parameters", "SPTA006", p, l) :- in_scope(d), doc(d, p, _), source(d, sp),
    match_all("{re}", sp, m), match_group("{re}", sp, m, "arg", g), text(g, "step"), line(m, l), (col(m, c), c = 7 ; col(m, c), c > 10).
violation("{TARGET_ID}", "executor-parameters", "SPTA006", p, l) :- in_scope(d), doc(d, p, _), node(d, call, "call", _, _),
    line(call, l), col(call, c), tape_line(d, l, sp), match_all("{re}", sp, m), col(m, c), match_group("{re}", sp, m, "arg", g), text(g, "step").
violation("{TARGET_ID}", "required-contracts", "SPTA001", p, 1) :- in_scope(d), doc(d, p, _), node(d, _, "module", _, _).
"#
    );
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("authored-match");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{TARGET_ID}.rr")), authored).unwrap();
    let c = convert_over(&rule_file(TARGET).unwrap(), &Adjust::default(), Some(&dir)).unwrap();
    assert!(c.program.contains("match_all(\"") && c.program.contains("match_group(\"") && !c.program.contains("match_all_("), "{}", c.program);
    let again = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse(&c.program).unwrap()).unwrap();
    assert_eq!(again, c.program);
    let run = dir.join("run");
    write_files(&run, [EXECUTOR]);
    let mut got = findings(&c.program, &run).unwrap();
    got.sort();
    got.dedup();
    let at = |l: usize, code: &str| format!("{}:{l}:{code}", EXECUTOR.0);
    assert_eq!(got, [at(1, "SPTA001"), at(2, "SPTA006"), at(2, "SPTA007"), at(4, "SPTA006"), at(4, "SPTA007")]);
    let _ = std::fs::remove_dir_all(&dir);
}
