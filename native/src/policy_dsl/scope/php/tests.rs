use super::*;
use crate::policy_dsl::language::GrammarDialect;
use crate::policy_dsl::scope::{Reading, ScopeTable};
use ast_grep_core::tree_sitter::LanguageExt;
use serde_json::Value as Json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn oracle_root() -> PathBuf {
    PathBuf::from(crate::policy_dsl::fixtures_dir!()).join("php/scope/php")
}

fn line_of(source: &str, offset: usize) -> u64 {
    source[..offset]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count() as u64
        + 1
}

fn try_table(source: &str) -> Result<ScopeTable, crate::policy_dsl::scope::BuildError> {
    let root = GrammarDialect::Php.ast_grep(source);
    ScopeTable::build_with(
        root.root(),
        GrammarDialect::Php,
        usize::MAX,
        &|| false,
        Reading::default(),
    )
}

fn table(source: &str) -> ScopeTable {
    try_table(source).expect("table builds")
}

/// Every statement a loop's fixpoint revisits is charged to the table's cap, so nested loops,
/// whose revisits multiply with depth, end within it rather than growing without bound.
#[test]
fn nested_loop_fixpoints_spend_the_cap() {
    let depth = 20;
    let source = format!(
        "<?php\nfunction f(bool $c) {{\n    $x = '';\n{}    $g = function () use ($x) {{ return $x; }};\n{}}}\n",
        "    while ($c) {\n        $x = $x . 'a';\n".repeat(depth),
        "    }\n".repeat(depth),
    );
    let root = GrammarDialect::Php.ast_grep(&source);
    let cap = 200_000;
    let result = ScopeTable::build_with(
        root.root(),
        GrammarDialect::Php,
        cap,
        &|| false,
        Reading::default(),
    );
    assert_eq!(
        result.err(),
        Some(crate::policy_dsl::scope::BuildError::Budget)
    );
}

/// PHPStan's `variable.undefined` reports of one file, by line and variable name (without `$`).
type Reports = BTreeMap<(u64, String), Vec<String>>;

fn reports(records: &Json) -> Reports {
    let mut reported = Reports::new();
    for record in records.as_array().expect("reports") {
        reported
            .entry((
                record["line"].as_u64().expect("line"),
                record["name"].as_str().expect("name").to_owned(),
            ))
            .or_default()
            .push(record["kind"].as_str().expect("kind").to_owned());
    }
    reported
}

#[derive(Default)]
struct Tally {
    failures: Vec<String>,
    /// Files whose table the build gave up on (its depth or budget).
    unbuilt: Vec<String>,
    /// Files the grammar reads with syntax errors.
    unparsed: usize,
    claims: usize,
    reads: usize,
}

/// Every read of a variable in one file against PHPStan's `variable.undefined`: a read Rerail finds
/// defined on every path is not reported; one defined on no path is reported undefined; a read
/// `isset`, `empty` or `??` guards is never reported; an argument a callee may take by reference
/// is reported as that callee's stub says, so Rerail claims nothing about it unless it is defined.
/// Where Rerail finds a variable defined on some paths, or cannot tell, it claims nothing.
fn compare_definedness(
    name: &str,
    source: &str,
    reported: &Reports,
    coverage: bool,
    tally: &mut Tally,
) {
    let table = match try_table(source) {
        Ok(table) => table,
        Err(error) => {
            tally.unbuilt.push(format!("{name}: {error:?}"));
            return;
        }
    };
    let flow = table.php.as_deref().expect("a PHP flow");
    let mut ours: BTreeMap<(u64, String), Vec<(Defined, Quiet)>> = BTreeMap::new();
    for (start, read) in &flow.reads {
        if read.superglobal {
            continue;
        }
        ours.entry((
            line_of(source, *start),
            read.name.trim_start_matches('$').to_owned(),
        ))
        .or_default()
        .push((read.defined, read.quiet));
    }
    for (key, states) in &ours {
        let report = reported.get(key);
        let definite = report.is_some_and(|kinds| {
            kinds
                .iter()
                .any(|kind| kind == "undefined" || kind == "compact")
        });
        let silent = states
            .iter()
            .all(|(defined, quiet)| *quiet == Quiet::Silent || *defined == Defined::Yes);
        for (defined, quiet) in states {
            tally.reads += 1;
            let claim = match (quiet, defined) {
                (Quiet::Silent, _) | (_, Defined::Yes) => Some(false),
                (Quiet::ByReference, _) => None,
                (Quiet::No, Defined::No) => Some(true),
                _ => None,
            };
            match claim {
                Some(true) if !definite => tally.failures.push(format!(
                    "{name}:{}: Rerail finds ${} undefined on every path; PHPStan reports {report:?}",
                    key.0, key.1
                )),
                Some(false) if report.is_some() && silent => tally.failures.push(format!(
                    "{name}:{}: Rerail finds ${} defined (or guarded); PHPStan reports {report:?}",
                    key.0, key.1
                )),
                _ => {}
            }
            tally.claims += usize::from(claim.is_some());
        }
    }
    for (key, kinds) in reported {
        if coverage && !ours.contains_key(key) {
            tally.failures.push(format!(
                "{name}:{}: PHPStan reports ${} {kinds:?}; Rerail records no read",
                key.0, key.1
            ));
        }
    }
}

#[test]
fn php_definedness_agrees_with_phpstan() {
    let expected: Json = serde_json::from_str(
        &std::fs::read_to_string(oracle_root().join("variables.expected.json")).expect("oracle"),
    )
    .expect("oracle JSON");
    assert_eq!(expected["php"].as_str().map(|v| &v[..3]), Some("8.5"));
    let mut tally = Tally::default();
    for (name, records) in expected["fixtures"].as_object().expect("fixtures") {
        let source = std::fs::read_to_string(oracle_root().join("variables").join(name))
            .expect("fixture source");
        compare_definedness(
            name,
            &source,
            &reports(&records["definedness"]),
            true,
            &mut tally,
        );
    }
    assert!(tally.unbuilt.is_empty(), "{:?}", tally.unbuilt);
    assert!(
        tally.failures.is_empty(),
        "{} disagreements:\n{}",
        tally.failures.len(),
        tally.failures.join("\n")
    );
    assert!(
        tally.claims * 10 >= tally.reads * 8,
        "Rerail decides only {} of {} reads",
        tally.claims,
        tally.reads
    );
}

fn php_files(root: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(root).expect("corpus directory") {
        let entry = entry.expect("entry");
        let path = entry.path();
        if entry.file_type().expect("type").is_dir() {
            php_files(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "php")
            && !path.to_string_lossy().ends_with(".blade.php")
        {
            out.push(path);
        }
    }
}

/// PHPStan's `variable.undefined` over whole corpora (`RERAIL_PHP_DEFINEDNESS`, written by
/// `fixtures/php/scope/php/generate_corpus.py`), against every read Rerail decides; the summary goes to
/// `RERAIL_PHP_DEFINEDNESS_REPORT`.
#[test]
#[ignore = "PHPStan definedness over full PHP corpora"]
fn benchmark_php_definedness_corpus() {
    let input = std::env::var("RERAIL_PHP_DEFINEDNESS").expect("RERAIL_PHP_DEFINEDNESS");
    let output = std::env::var("RERAIL_PHP_DEFINEDNESS_REPORT").expect("report path");
    let corpora: Json =
        serde_json::from_str(&std::fs::read_to_string(input).expect("corpora")).expect("JSON");
    let mut summary = Vec::new();
    let mut all_failures = Vec::new();
    for corpus in corpora["corpora"].as_array().expect("corpora") {
        let excluded: Vec<&str> = corpus["excluded"]
            .as_array()
            .expect("excluded")
            .iter()
            .filter_map(Json::as_str)
            .collect();
        let mut files = Vec::new();
        for path in corpus["paths"].as_array().expect("paths") {
            php_files(Path::new(path.as_str().expect("path")), &mut files);
        }
        files.sort();
        files.retain(|file| !excluded.contains(&file.to_string_lossy().as_ref()));
        let mut tally = Tally::default();
        let empty = Reports::new();
        for file in &files {
            let key = file.to_string_lossy();
            let Ok(source) = std::fs::read_to_string(file) else {
                continue;
            };
            // A file the grammar reads with errors (a template, a construct it lacks) is outside
            // what the flow can read, as tape parity reports it.
            if GrammarDialect::Php
                .ast_grep(&source)
                .root()
                .dfs()
                .any(|node| node.is_error() || node.is_missing())
            {
                tally.unparsed += 1;
                continue;
            }
            let reported = corpus["reports"]
                .get(key.as_ref())
                .map(reports)
                .unwrap_or_else(|| empty.clone());
            // PHPStan reads variables the source does not spell (`${'a'}`), which Rerail leaves
            // unknown: a report with no read beside it is no disagreement.
            compare_definedness(&key, &source, &reported, false, &mut tally);
        }
        summary.push(serde_json::json!({
            "corpus": corpus["name"],
            "files": files.len(),
            "filesWithSyntaxErrors": tally.unparsed,
            "phpstanReports": corpus["reports"].as_object().map_or(0, |files| files.values().map(|v| v.as_array().map_or(0, Vec::len)).sum::<usize>()),
            "reads": tally.reads,
            "decided": tally.claims,
            "disagreements": tally.failures,
            "unbuilt": tally.unbuilt,
        }));
        all_failures.extend(tally.failures);
    }
    let report = serde_json::json!({
        "benchmark": "PHP variable definedness: Rerail's flow against PHPStan's variable.undefined",
        "phpstan": corpora["phpstan"],
        "php": corpora["php"],
        "corpora": summary,
    });
    std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).expect("write report");
    assert!(
        all_failures.is_empty(),
        "{} disagreements",
        all_failures.len()
    );
}

/// The bindings whose usedness Psalm reports, by the form it reports them as: assignments,
/// compound assignments and increments (`UnusedVariable`), a closure's `use` copies
/// (`UnusedVariable`), a function's parameters (`UnusedParam`; a method's may implement an
/// interface, so Psalm reports none of them) and `foreach` variables (`UnusedForeachValue`, which
/// covers the value but not the key: Rerail claims nothing unused there).
fn psalm_form(
    table: &ScopeTable,
    root: &ast_grep_core::Node<'_, impl ast_grep_core::Doc>,
    binding: &crate::policy_dsl::scope::Binding,
) -> Option<bool> {
    let site = binding.site.node?.find(root.clone())?;
    Some(match (binding.site.form, site.kind().as_ref()) {
        // Psalm reports an assignment that stands as a statement, not one whose value an
        // enclosing expression uses (`$x = $a ?? ($b = 1)`).
        (Form::Assignment, "assignment_expression") => site
            .parent()
            .is_some_and(|parent| parent.kind() == "expression_statement"),
        (Form::Augmented, "augmented_assignment_expression" | "update_expression") => true,
        (Form::Parameter, "anonymous_function_use_clause") => true,
        (Form::Parameter, "simple_parameter") => {
            let function = site.parent()?.parent()?;
            if function.kind() != "function_definition" {
                return None;
            }
            true
        }
        (Form::For, _) => false,
        _ => return None,
    })
    .filter(|_| !table.scopes.is_empty())
}

/// Where Psalm is wrong and Rerail is not, by fixture, line and variable, with the evidence.
const PSALM_DIVERGENCES: &[(&str, u64, &str, &str)] = &[
    (
        "closures.php",
        3,
        "a",
        "Psalm reports the parameter unused although the arrow function on line 16 reads it; it \
         attributes the unread `use ($a)` copy of line 25 to the parameter",
    ),
    (
        "closures.php",
        25,
        "a",
        "the unread `use ($a)` copy, which Psalm reports at the parameter (line 3) instead",
    ),
    (
        "more.php",
        19,
        "one",
        "`continue 2` skips this iteration's `echo $one`, but the next iteration's reads it: \
         `switch_and_continue([1, 2])` prints `21`",
    ),
];

/// Every binding Psalm's unused-variable analysis reads, against Rerail's usedness: a binding no
/// read reaches is reported; one some read reaches is not; `PSALM_DIVERGENCES` aside.
#[test]
fn php_usage_agrees_with_psalm() {
    use crate::policy_dsl::scope::usage::{BindingId, Usage};
    let expected: Json = serde_json::from_str(
        &std::fs::read_to_string(oracle_root().join("variables.expected.json")).expect("oracle"),
    )
    .expect("oracle JSON");
    let mut failures = Vec::new();
    let (mut claims, mut bindings) = (0, 0);
    for (name, records) in expected["fixtures"].as_object().expect("fixtures") {
        let source = std::fs::read_to_string(oracle_root().join("variables").join(name))
            .expect("fixture source");
        let mut reported: BTreeMap<(u64, String), String> = BTreeMap::new();
        for record in records["usage"].as_array().expect("usage") {
            reported.insert(
                (
                    record["line"].as_u64().expect("line"),
                    record["name"].as_str().expect("name").to_owned(),
                ),
                record["type"].as_str().expect("type").to_owned(),
            );
        }
        let root = GrammarDialect::Php.ast_grep(&source);
        let table = table(&source);
        let usage = Usage::build(&table, &root.root(), usize::MAX, &|| false).expect("usage");
        let mut seen = BTreeMap::new();
        for (scope_index, scope) in table.scopes.iter().enumerate() {
            for (variable, list) in &scope.bindings {
                for (index, binding) in list.iter().enumerate() {
                    let Some(claims_unused) = psalm_form(&table, &root.root(), binding) else {
                        continue;
                    };
                    let Some(node) = binding.name else { continue };
                    bindings += 1;
                    let key = (
                        line_of(&source, node.start as usize),
                        variable.trim_start_matches('$').to_owned(),
                    );
                    let used = usage
                        .facts(BindingId {
                            scope: scope_index as u32,
                            name: variable,
                            index,
                        })
                        .used;
                    seen.insert(key.clone(), used);
                    if PSALM_DIVERGENCES
                        .iter()
                        .any(|(fixture, line, variable, _)| {
                            *fixture == name && *line == key.0 && *variable == key.1
                        })
                    {
                        continue;
                    }
                    let report = reported.get(&key);
                    match used {
                        Some(false) if claims_unused && report.is_none() => failures.push(format!(
                            "{name}:{}: Rerail finds ${} unused; Psalm reports nothing",
                            key.0, key.1
                        )),
                        Some(true) if report.is_some() => failures.push(format!(
                            "{name}:{}: Rerail finds ${} used; Psalm reports {report:?}",
                            key.0, key.1
                        )),
                        _ => {}
                    }
                    claims += usize::from(used.is_some());
                }
            }
        }
        for (key, kind) in &reported {
            if !seen.contains_key(key) {
                failures.push(format!(
                    "{name}:{}: Psalm reports ${} ({kind}); Rerail has no such binding",
                    key.0, key.1
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} disagreements:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        claims * 10 >= bindings * 8,
        "Rerail decides only {claims} of {bindings} bindings"
    );
}

/// PHP's variable predicates through a compiled policy: what each finds in one function.
#[test]
fn php_variable_predicates_evaluate_through_policies() {
    use crate::policy_dsl::test_support::*;
    use serde_json::json;
    let source = "<?php\nfunction f(bool $c) {\n    $limit = 10;\n    echo $undefined;\n    if ($c) {\n        $maybe = 1;\n    }\n    echo $maybe;\n    $unused = strlen('x');\n    $x = $limit;\n    echo $x;\n    echo $_GET['q'];\n}\n";
    let capabilities: Json = serde_json::from_str(CAPABILITIES).expect("capabilities");
    let run = |rule: Json| {
        let compiled = compile_document_with(
            check_document(json!({
                "id": "php-variables",
                "forbid": { "ast": { "language": "php", "rule": rule } },
                "finding": finding(),
            })),
            &capabilities,
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        matched_texts(&evaluate_file(&compiled, "php", "f.php", source), source)
    };
    // `$maybe` is defined on one path only: neither undefined nor defined.
    assert_eq!(
        run(json!({ "kind": "variable_name", "resolves": { "unresolved": true } })),
        ["$undefined"]
    );
    assert_eq!(
        run(json!({ "kind": "variable_name", "binds": { "form": "assignment", "used": false } })),
        ["$unused"]
    );
    // `$x` follows to `$limit`, which follows to 10.
    assert_eq!(
        run(json!({ "kind": "variable_name", "value": { "follow": true, "eq": 10 } })),
        ["$limit", "$x"]
    );
    assert_eq!(
        run(json!({ "kind": "variable_name", "resolves": { "global": ["_GET"] } })),
        ["$_GET"]
    );
    assert_eq!(
        run(
            json!({ "kind": "variable_name", "resolves": { "binding": { "form": "parameter", "scope": "function" } } })
        ),
        ["$c"]
    );
}

/// What may unset a variable (`eval`, `include`, the code a `goto` comes back from) leaves no
/// variable known defined after it; `extract` only defines or overwrites.
#[test]
fn code_that_may_unset_leaves_definedness_unknown() {
    let defined = |source: &str| {
        let table = table(source);
        let flow = table.php.as_deref().expect("a PHP flow");
        let start = source.find("echo $x").expect("a read") + "echo ".len();
        flow.read(start).expect("the read").defined
    };
    for source in [
        "<?php\nfunction f() { $x = 1; eval('unset($x);'); echo $x; }\n",
        "<?php\nfunction f() { $x = 1; include 'unsets.php'; echo $x; }\n",
        "<?php\nfunction f(bool $c) { $x = 1; back: echo $x; unset($x); if ($c) { goto back; } }\n",
        // A forward `goto` reaches the label with `$x` unset; fallthrough defines it.
        "<?php\nfunction f($c) { if ($c) { goto done; } $x = 1; done: echo $x; }\n",
    ] {
        assert_eq!(defined(source), Defined::Unknown, "{source}");
    }
    assert_eq!(
        defined("<?php\nfunction f(array $a) { $x = 1; extract($a); echo $x; }\n"),
        Defined::Yes
    );
}

/// A named argument to a builtin's by-reference parameter binds the variable as the positional
/// one does (`preg_match`'s `matches`).
#[test]
fn named_by_reference_builtin_arguments_bind() {
    for source in [
        "<?php\nfunction f(string $s) { preg_match('/x/', $s, matches: $m); echo $m; }\n",
        "<?php\nfunction f(string $s) { preg_match(subject: $s, matches: $m, pattern: '/x/'); echo $m; }\n",
    ] {
        let table = table(source);
        let flow = table.php.as_deref().expect("a PHP flow");
        let start = source.find("echo $m").expect("a read") + "echo ".len();
        assert_eq!(
            flow.read(start).expect("the read").defined,
            Defined::Yes,
            "{source}"
        );
    }
}

/// A subscript assignment (`$a[] = 1`) references the binding it changes without using it.
#[test]
fn subscript_assignments_reference_without_using() {
    use crate::policy_dsl::test_support::*;
    use serde_json::json;
    let source = "<?php\nfunction f() {\n    $a = [];\n    $a[] = 1;\n}\n";
    let capabilities: Json = serde_json::from_str(CAPABILITIES).expect("capabilities");
    let lines = |filter: Json| -> Vec<u64> {
        let compiled = compile_document_with(
            check_document(json!({
                "id": "php-references",
                "forbid": { "ast": { "language": "php", "rule": { "kind": "variable_name", "binds": filter } } },
                "finding": finding(),
            })),
            &capabilities,
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "php", "f.php", source);
        result["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|finding| {
                let start = finding["span"]["startByte"].as_u64().expect("start") as usize;
                line_of(source, start)
            })
            .collect()
    };
    assert_eq!(lines(json!({ "used": false })), [3, 4]);
    assert_eq!(lines(json!({ "referenced": false })), [4]);
    assert_eq!(lines(json!({ "referenced": true })), [3]);
}
