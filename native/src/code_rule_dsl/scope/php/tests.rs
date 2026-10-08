use super::*;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::scope::{Reading, ScopeTable};
use ast_grep_core::tree_sitter::LanguageExt;
use serde_json::Value as Json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn oracle_root() -> PathBuf {
    PathBuf::from(env!("RERAIL_FIXTURES_DIR")).join("php/scope/php")
}

fn line_of(source: &str, offset: usize) -> u64 {
    source[..offset]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count() as u64
        + 1
}

fn try_table(source: &str) -> Result<ScopeTable, crate::code_rule_dsl::scope::BuildError> {
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
        Some(crate::code_rule_dsl::scope::BuildError::Budget)
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
    /// Per PHPStan kind (`undefined`, `maybe`, `compact`), Rerail's verdict on the reads of each
    /// reported line and variable: `undefined` (some read undefined on every path), `defined`
    /// (every read defined on every path, or guarded), `unknown`, or `noRead`.
    recall: BTreeMap<String, BTreeMap<&'static str, usize>>,
    /// The `undefined` and `compact` reports Rerail does not claim undefined, with its verdict.
    misses: Vec<String>,
}

/// What Rerail claims of one read: `Some(true)` undefined on every path, `Some(false)` defined on
/// every path (or guarded, so PHP does not warn), `None` nothing.
fn definedness_claim(defined: Defined, quiet: Quiet) -> Option<bool> {
    match (quiet, defined) {
        (Quiet::Silent, _) | (_, Defined::Yes) => Some(false),
        (Quiet::ByReference, _) => None,
        (Quiet::No, Defined::No) => Some(true),
        _ => None,
    }
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
            let claim = definedness_claim(*defined, *quiet);
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
        let verdict = match ours.get(key) {
            None => {
                if coverage {
                    tally.failures.push(format!(
                        "{name}:{}: PHPStan reports ${} {kinds:?}; Rerail records no read",
                        key.0, key.1
                    ));
                }
                "noRead"
            }
            Some(states)
                if states
                    .iter()
                    .any(|(defined, quiet)| definedness_claim(*defined, *quiet) == Some(true)) =>
            {
                "undefined"
            }
            Some(states)
                if states
                    .iter()
                    .all(|(defined, quiet)| definedness_claim(*defined, *quiet) == Some(false)) =>
            {
                "defined"
            }
            Some(_) => "unknown",
        };
        for kind in kinds {
            *tally
                .recall
                .entry(kind.clone())
                .or_default()
                .entry(verdict)
                .or_default() += 1;
            if kind != "maybe" && verdict != "undefined" {
                tally
                    .misses
                    .push(format!("{name}:{}: ${} ({kind}): {verdict}", key.0, key.1));
            }
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

/// The files of one recorded corpus a benchmark reads: the `.php` files under its `paths`, Blade
/// templates and the files PHP rejects (`excluded`) aside.
fn corpus_files(corpus: &Json) -> Vec<PathBuf> {
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
    files
}

/// A file the grammar reads with errors (a template, a construct it lacks) is outside what the
/// flow can read, as tape parity reports it.
fn has_syntax_errors(source: &str) -> bool {
    GrammarDialect::Php
        .ast_grep(source)
        .root()
        .dfs()
        .any(|node| node.is_error() || node.is_missing())
}

/// `part / whole` to six places; null where there is nothing to divide.
fn ratio(part: usize, whole: usize) -> Json {
    if whole == 0 {
        return Json::Null;
    }
    serde_json::json!((part as f64 / whole as f64 * 1e6).round() / 1e6)
}

/// `path:line: ...` records with their paths relative to the corpus checkout.
fn relative_all(corpus: &Json, records: &[String]) -> Vec<String> {
    let root = format!("/{}/", corpus["name"].as_str().expect("name"));
    records
        .iter()
        .map(|record| {
            record
                .split_once(&root)
                .map_or_else(|| record.clone(), |(_, rest)| rest.to_owned())
        })
        .collect()
}

/// PHPStan's `variable.undefined` over whole corpora (`RERAIL_PHP_DEFINEDNESS`, written by
/// `rerail-fixtures/php/scope/php/generate_corpus.py`), against every read Rerail decides; the summary goes to
/// `RERAIL_PHP_DEFINEDNESS_REPORT`. Precision: no decided read contradicts PHPStan. Recall: of
/// PHPStan's "Undefined variable" reports, the share Rerail claims undefined; of its "might not be
/// defined" reports, Rerail leaves each unknown and claims none defined.
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
        let files = corpus_files(corpus);
        let mut tally = Tally::default();
        let empty = Reports::new();
        for file in &files {
            let key = file.to_string_lossy();
            let Ok(source) = std::fs::read_to_string(file) else {
                continue;
            };
            if has_syntax_errors(&source) {
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
        let reports: usize = corpus["reports"].as_object().map_or(0, |files| {
            files
                .values()
                .map(|v| v.as_array().map_or(0, Vec::len))
                .sum()
        });
        let recall_by_kind: serde_json::Map<String, Json> = tally
            .recall
            .iter()
            .map(|(kind, verdicts)| {
                let total: usize = verdicts.values().sum();
                let maybe = kind == "maybe";
                let hit = verdicts
                    .get(if maybe { "unknown" } else { "undefined" })
                    .copied()
                    .unwrap_or(0);
                let mut row = serde_json::json!({ "reports": total, "verdicts": verdicts });
                row[if maybe { "unknownRate" } else { "recall" }] = ratio(hit, total);
                (kind.clone(), row)
            })
            .collect();
        // The reference's positives: the reads it reports undefined on every path.
        let (positives, true_positives) = tally
            .recall
            .iter()
            .filter(|(kind, _)| *kind != "maybe")
            .fold((0, 0), |(all, hit), (_, verdicts)| {
                (
                    all + verdicts.values().sum::<usize>(),
                    hit + verdicts.get("undefined").copied().unwrap_or(0),
                )
            });
        summary.push(serde_json::json!({
            "name": corpus["name"],
            "commit": corpus["commit"],
            "files": files.len(),
            "filesWithSyntaxErrors": tally.unparsed,
            "phpstanReports": reports,
            "items": tally.reads,
            "decided": tally.claims,
            "decidedRate": ratio(tally.claims, tally.reads),
            "referencePositives": positives,
            "truePositives": true_positives,
            "contradictions": tally.failures.len(),
            "precision": ratio(tally.claims - tally.failures.len(), tally.claims),
            "recall": ratio(true_positives, positives),
            "recallByKind": recall_by_kind,
            "undefinedNotClaimed": relative_all(corpus, &tally.misses),
            "disagreements": relative_all(corpus, &tally.failures),
            "unbuilt": relative_all(corpus, &tally.unbuilt),
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

/// Whether Psalm's unused-parameter check skips a parameter of this name
/// (`FunctionLikeAnalyzer::isIgnoredForUnusedParam`).
fn psalm_ignores_parameter(name: &str) -> bool {
    name.starts_with("$_") || (name.starts_with("$unused") && name != "$unused")
}

/// How Psalm would report a binding unused: `None` where it reports no binding of this form;
/// else whether Rerail claims it reported where unused, the name starts of the later parameters
/// that must all be unused too, and the issue type Psalm reports it under (a line may hold
/// several bindings of one variable: `$job = new Job(function ($job) {})`).
///
/// The forms: assignments, compound assignments and increments (`UnusedVariable`), a closure's
/// `use` copies (`UnusedVariable`), a function's parameters (`UnusedParam`; a method's may
/// implement an interface, so Psalm reports none but a private one's: Rerail claims none), a
/// closure's or arrow function's parameters (`UnusedClosureParam`) and `foreach` targets (the
/// value, by reference or not, `UnusedForeachValue`; the key and destructured elements,
/// `UnusedVariable`). Psalm reports no variable named `$_...`, and no parameter
/// `psalm_ignores_parameter` skips. Of a function's unused parameters it reports only the
/// trailing run (`FunctionLikeAnalyzer::checkParamReferences`): its position count includes the
/// parameters it skips (it tests their names without the `$`), so one before a skipped
/// parameter is never reported.
fn psalm_form(
    root: &ast_grep_core::Node<'_, impl ast_grep_core::Doc>,
    variable: &str,
    binding: &crate::code_rule_dsl::scope::Binding,
) -> Option<(bool, Vec<usize>, &'static str)> {
    if variable.starts_with("$_") {
        return None;
    }
    let site = binding.site.node?.find(root.clone())?;
    let claims = match (binding.site.form, site.kind().as_ref()) {
        // Psalm reports an assignment that stands as a statement, not one whose value an
        // enclosing expression uses (`$x = $a ?? ($b = 1)`).
        (Form::Assignment, "assignment_expression") => site
            .parent()
            .is_some_and(|parent| parent.kind() == "expression_statement"),
        (Form::Augmented, "augmented_assignment_expression" | "update_expression") => true,
        (Form::Parameter, "anonymous_function_use_clause") => true,
        (Form::Parameter, "simple_parameter") => {
            let parameters = site.parent()?;
            let function = parameters.parent()?;
            let kind = match function.kind().as_ref() {
                "function_definition" => "UnusedParam",
                "anonymous_function" | "arrow_function" => "UnusedClosureParam",
                _ => return None,
            };
            if psalm_ignores_parameter(variable) {
                return None;
            }
            let later: Vec<_> = parameters
                .children()
                .skip_while(|parameter| parameter.range() != site.range())
                .skip(1)
                .filter_map(|parameter| parameter.field("name"))
                .collect();
            if later
                .iter()
                .any(|name| psalm_ignores_parameter(&name.text()))
            {
                return Some((false, Vec::new(), kind));
            }
            let later = later.iter().map(|name| name.range().start).collect();
            return Some((true, later, kind));
        }
        (Form::For, _) => {
            // The value: the target itself, a `pair`'s last part, or under `&`.
            let name = binding.name?.find(root.clone())?;
            let parent = name.parent()?;
            let value = match parent.kind().as_ref() {
                "foreach_statement" | "by_ref" => true,
                "pair" => named(&parent)
                    .last()
                    .is_some_and(|last| last.range() == name.range()),
                _ => false,
            };
            let kind = if value {
                "UnusedForeachValue"
            } else {
                "UnusedVariable"
            };
            return Some((true, Vec::new(), kind));
        }
        _ => return None,
    };
    Some((claims, Vec::new(), "UnusedVariable"))
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
        "closures.php",
        28,
        "ignored",
        "Psalm stops analysing `closures()` at the undefined `use ($missing)` of line 9 and \
         reports nothing after it: with lines 9-14 removed it reports UnusedClosureParam here",
    ),
    (
        "more.php",
        19,
        "one",
        "`continue 2` skips this iteration's `echo $one`, but the next iteration's reads it: \
         `switch_and_continue([1, 2])` prints `21`",
    ),
];

/// The bindings Psalm reports unused in the fixtures that Rerail leaves unknown, by fixture, line,
/// variable and issue type, with why.
const PSALM_UNRECALLED: &[(&str, u64, &str, &str, &str)] = &[
    (
        "special.php",
        110,
        "kept",
        "UnusedVariable",
        "`get_defined_vars()` reads every variable: Psalm is wrong, the value is returned",
    ),
    (
        "unused.php",
        18,
        "reference",
        "UnusedForeachValue",
        "a by-reference value aliases the array's element: Rerail does not follow what an alias \
         writes",
    ),
    (
        "unused.php",
        22,
        "slot",
        "UnusedForeachValue",
        "a by-reference value aliases the array's element",
    ),
    (
        "references.php",
        26,
        "only",
        "UnusedParam",
        "`func_get_arg(0)` reads the parameter: Psalm is wrong (it counts `func_get_args()` as a \
         use, not `func_get_arg`); Rerail reads every variable there as possibly used",
    ),
];

/// Psalm's unused-variable reports of one file, by line, variable name (without `$`) and issue
/// type.
type UsageReports = std::collections::BTreeSet<UsageKey>;

/// A line, a variable name without `$`, and a Psalm issue type.
type UsageKey = (u64, String, String);

fn usage_reports(records: &Json) -> UsageReports {
    records
        .as_array()
        .expect("usage reports")
        .iter()
        .map(|record| {
            (
                record["line"].as_u64().expect("line"),
                record["name"].as_str().expect("name").to_owned(),
                record["type"].as_str().expect("type").to_owned(),
            )
        })
        .collect()
}

#[derive(Default)]
struct UsageTally {
    failures: Vec<String>,
    /// Files whose table or usage the build gave up on (its depth or budget).
    unbuilt: Vec<String>,
    /// Files the grammar reads with syntax errors.
    unparsed: usize,
    /// Bindings of a form Psalm reports.
    bindings: usize,
    /// Of those, the ones Rerail decides used or unused.
    claims: usize,
    /// Of those, the ones Rerail claims unused in a form Psalm reports unused.
    unused: usize,
    /// Bindings a listed divergence covers.
    divergent: usize,
    /// Bindings in code Psalm does not analyse (`unanalysed`).
    unanalysed: usize,
    /// The ranges of code Psalm does not analyse.
    unanalysed_ranges: usize,
    /// Per Psalm issue type, Rerail's verdict on each reported binding: `unused` (claimed),
    /// `unusedUnclaimed` (unused, in a form Rerail makes no unused claim for), `unknown`, `used`,
    /// or `noBinding`.
    recall: BTreeMap<String, BTreeMap<&'static str, usize>>,
    /// The reports Rerail does not claim unused, with its verdict.
    misses: Vec<String>,
}

/// Every binding of one file Psalm's unused-variable analysis reads, against Rerail's usedness: a
/// binding no read reaches is reported; one some read reaches is not; bindings `divergent`
/// covers, and those (and the reports) in the byte ranges of `unanalysed`, aside. `coverage`: a
/// report Rerail has no binding for is a disagreement.
fn compare_usage(
    name: &str,
    source: &str,
    reported: &UsageReports,
    divergent: &dyn Fn(u64, &str) -> bool,
    unanalysed: &[std::ops::Range<usize>],
    coverage: bool,
    tally: &mut UsageTally,
) {
    use crate::code_rule_dsl::scope::usage::{BindingId, Usage};
    let root = GrammarDialect::Php.ast_grep(source);
    let (table, usage) = match try_table(source).and_then(|table| {
        let usage = Usage::build(&table, &root.root(), usize::MAX, &|| false)?;
        Ok((table, usage))
    }) {
        Ok(built) => built,
        Err(error) => {
            tally.unbuilt.push(format!("{name}: {error:?}"));
            return;
        }
    };
    // Every binding's usedness, and each parameter's by its name's start.
    let mut bindings = Vec::new();
    let mut parameters: BTreeMap<usize, Option<bool>> = BTreeMap::new();
    for (scope_index, scope) in table.scopes.iter().enumerate() {
        for (variable, list) in &scope.bindings {
            for (index, binding) in list.iter().enumerate() {
                let Some(node) = binding.name else { continue };
                let used = usage
                    .facts(BindingId {
                        scope: scope_index as u32,
                        name: variable,
                        index,
                    })
                    .used;
                if binding.site.form == Form::Parameter {
                    parameters.insert(node.start as usize, used);
                }
                bindings.push((variable, binding, node, used));
            }
        }
    }
    // Per line, variable and issue type: whether Rerail claims each binding there unused where
    // unused, and its usedness.
    let mut ours: BTreeMap<UsageKey, Vec<(bool, Option<bool>)>> = BTreeMap::new();
    for (variable, binding, node, used) in bindings {
        let Some((claims_unused, later, kind)) = psalm_form(&root.root(), variable, binding) else {
            continue;
        };
        // A parameter is reported only while every later one is unused too: where Rerail finds a
        // later one used or cannot tell, it predicts no report for an unused one.
        if used == Some(false)
            && !later
                .iter()
                .all(|start| parameters.get(start) == Some(&Some(false)))
        {
            continue;
        }
        if unanalysed
            .iter()
            .any(|range| range.contains(&(node.start as usize)))
        {
            tally.unanalysed += 1;
            continue;
        }
        let key = (
            line_of(source, node.start as usize),
            variable.trim_start_matches('$').to_owned(),
            kind.to_owned(),
        );
        if divergent(key.0, &key.1) {
            tally.divergent += 1;
            continue;
        }
        tally.bindings += 1;
        tally.claims += usize::from(used.is_some());
        tally.unused += usize::from(claims_unused && used == Some(false));
        ours.entry(key).or_default().push((claims_unused, used));
    }
    // Psalm reports a key once however many of its bindings are unused: a report contradicts a
    // key whose every binding Rerail finds used, and its absence one binding Rerail claims unused.
    for (key, states) in &ours {
        let (line, variable, kind) = key;
        if reported.contains(key) {
            if states.iter().all(|(_, used)| *used == Some(true)) {
                tally.failures.push(format!(
                    "{name}:{line}: Rerail finds ${variable} used; Psalm reports {kind}"
                ));
            }
        } else if states.contains(&(true, Some(false))) {
            tally.failures.push(format!(
                "{name}:{line}: Rerail finds ${variable} unused; Psalm reports nothing ({kind})"
            ));
        }
    }
    let unanalysed_lines: Vec<_> = unanalysed
        .iter()
        .map(|range| line_of(source, range.start)..=line_of(source, range.end))
        .collect();
    tally.unanalysed_ranges += unanalysed.len();
    for key in reported {
        let (line, variable, kind) = key;
        if divergent(*line, variable) || unanalysed_lines.iter().any(|lines| lines.contains(line)) {
            continue;
        }
        let verdict = match ours.get(key) {
            None => {
                if coverage {
                    tally.failures.push(format!(
                        "{name}:{line}: Psalm reports ${variable} ({kind}); Rerail has no such binding"
                    ));
                }
                "noBinding"
            }
            Some(states) if states.contains(&(true, Some(false))) => "unused",
            Some(states) if states.iter().all(|(_, used)| *used == Some(true)) => "used",
            Some(states) if states.contains(&(false, Some(false))) => "unusedUnclaimed",
            Some(_) => "unknown",
        };
        *tally
            .recall
            .entry(kind.clone())
            .or_default()
            .entry(verdict)
            .or_default() += 1;
        if verdict != "unused" {
            tally
                .misses
                .push(format!("{name}:{line}: ${variable} ({kind}): {verdict}"));
        }
    }
}

/// Every binding Psalm's unused-variable analysis reads in the fixtures, against Rerail's
/// usedness; `PSALM_DIVERGENCES` aside.
#[test]
fn php_usage_agrees_with_psalm() {
    let expected: Json = serde_json::from_str(
        &std::fs::read_to_string(oracle_root().join("variables.expected.json")).expect("oracle"),
    )
    .expect("oracle JSON");
    let mut tally = UsageTally::default();
    for (name, records) in expected["fixtures"].as_object().expect("fixtures") {
        let source = std::fs::read_to_string(oracle_root().join("variables").join(name))
            .expect("fixture source");
        let divergent = |line: u64, variable: &str| {
            PSALM_DIVERGENCES
                .iter()
                .any(|(fixture, at, name_, _)| fixture == name && *at == line && *name_ == variable)
        };
        compare_usage(
            name,
            &source,
            &usage_reports(&records["usage"]),
            &divergent,
            &[],
            true,
            &mut tally,
        );
    }
    assert!(tally.unbuilt.is_empty(), "{:?}", tally.unbuilt);
    // Each binding Psalm reports in the fixtures, Rerail claims unused, but where it cannot tell.
    let expected_misses: std::collections::BTreeSet<String> = PSALM_UNRECALLED
        .iter()
        .map(|(fixture, line, variable, kind, _)| {
            format!("{fixture}:{line}: ${variable} ({kind}): unknown")
        })
        .collect();
    assert_eq!(
        tally
            .misses
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        expected_misses
    );
    assert!(
        tally.failures.is_empty(),
        "{} disagreements:\n{}",
        tally.failures.len(),
        tally.failures.join("\n")
    );
    assert!(
        tally.claims * 10 >= tally.bindings * 8,
        "Rerail decides only {} of {} bindings",
        tally.claims,
        tally.bindings
    );
}

/// Psalm's unused-variable reports over whole corpora (`RERAIL_PHP_USAGE`, written by
/// `rerail-fixtures/php/scope/php/generate_usage_corpus.py`), against every binding of a form Psalm
/// reports; the summary goes to `RERAIL_PHP_USAGE_REPORT`. Precision: no decided binding
/// contradicts Psalm. Recall: of Psalm's reports, the share Rerail claims unused.
/// The bindings `usage-corpus.divergences.json` lists (corpus, path, line, variable and a class
/// whose evidence says why Psalm and Rerail answer differently there) are counted apart.
#[test]
#[ignore = "Psalm usage over full PHP corpora"]
fn benchmark_php_usage_corpus() {
    let input = std::env::var("RERAIL_PHP_USAGE").expect("RERAIL_PHP_USAGE");
    let output = std::env::var("RERAIL_PHP_USAGE_REPORT").expect("report path");
    let corpora: Json =
        serde_json::from_str(&std::fs::read_to_string(input).expect("corpora")).expect("JSON");
    let divergences: Json = serde_json::from_str(
        &std::fs::read_to_string(oracle_root().join("usage-corpus.divergences.json"))
            .expect("divergences"),
    )
    .expect("divergences JSON");
    let items = divergences["items"].as_array().expect("items");
    for item in items {
        let class = item["class"].as_str().expect("class");
        assert!(
            divergences["classes"][class]["evidence"].is_string(),
            "divergence class {class} has no evidence"
        );
    }
    let mut summary = Vec::new();
    let mut all_failures = Vec::new();
    for corpus in corpora["corpora"].as_array().expect("corpora") {
        let corpus_name = corpus["name"].as_str().expect("name");
        let mut tally = UsageTally::default();
        let empty = UsageReports::new();
        let files = corpus_files(corpus);
        for file in &files {
            let key = file.to_string_lossy();
            let Ok(source) = std::fs::read_to_string(file) else {
                continue;
            };
            if has_syntax_errors(&source) {
                tally.unparsed += 1;
                continue;
            }
            let reported = corpus["reports"]
                .get(key.as_ref())
                .map(usage_reports)
                .unwrap_or_else(|| empty.clone());
            let divergent = |line: u64, variable: &str| {
                items.iter().any(|item| {
                    item["corpus"] == corpus_name
                        && key.ends_with(item["path"].as_str().expect("path"))
                        && item["line"] == line
                        && item["name"] == variable
                })
            };
            // What Psalm does not analyse (`generate_usage_corpus.py`'s `unanalysed`): the
            // functions it stops analysing, and the classes and files it marks as having visitor
            // issues.
            let unanalysed: Vec<std::ops::Range<usize>> = corpus["unanalysed"]
                .get(key.as_ref())
                .and_then(Json::as_array)
                .map(|ranges| {
                    ranges
                        .iter()
                        .map(|range| {
                            range[0].as_u64().expect("start") as usize
                                ..range[1].as_u64().expect("end") as usize
                        })
                        .collect()
                })
                .unwrap_or_default();
            compare_usage(
                &key,
                &source,
                &reported,
                &divergent,
                &unanalysed,
                false,
                &mut tally,
            );
        }
        let reports: usize = tally.recall.values().flat_map(BTreeMap::values).sum();
        let recalled: usize = tally
            .recall
            .values()
            .filter_map(|verdicts| verdicts.get("unused"))
            .sum();
        let recall: serde_json::Map<String, Json> = tally
            .recall
            .iter()
            .map(|(kind, verdicts)| {
                let total: usize = verdicts.values().sum();
                let hit = verdicts.get("unused").copied().unwrap_or(0);
                (
                    kind.clone(),
                    serde_json::json!({ "reports": total, "verdicts": verdicts, "recall": ratio(hit, total) }),
                )
            })
            .collect();
        summary.push(serde_json::json!({
            "name": corpus_name,
            "commit": corpus["commit"],
            "files": files.len(),
            "filesWithSyntaxErrors": tally.unparsed,
            "items": tally.bindings,
            "decided": tally.claims,
            "decidedRate": ratio(tally.claims, tally.bindings),
            "rerailUnused": tally.unused,
            "referencePositives": reports,
            "truePositives": recalled,
            "contradictions": tally.failures.len(),
            "precision": ratio(tally.claims - tally.failures.len(), tally.claims),
            "recall": ratio(recalled, reports),
            "recallByType": recall,
            "divergent": tally.divergent,
            "psalmUnanalysedRanges": tally.unanalysed_ranges,
            "psalmUnanalysedBindings": tally.unanalysed,
            "notClaimed": relative_all(corpus, &tally.misses),
            "disagreements": relative_all(corpus, &tally.failures),
            "unbuilt": relative_all(corpus, &tally.unbuilt),
        }));
        all_failures.extend(tally.failures);
    }
    let report = serde_json::json!({
        "benchmark": "PHP variable usage: Rerail's usedness against Psalm's unused-variable analysis",
        "psalm": corpora["psalm"],
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

/// PHP compares a switch's cases in order and enters `default`, wherever it is written, only
/// after comparing every case: `php -r 'function d(int $n) { switch ($n) { default:
/// var_dump($late ?? "UNDEF"); break; case 1: break; case $late = 2: break; } } d(5);'` prints
/// `int(2)`. PHPStan 2.2.16 reports the read in that `default` undefined, so the PHPStan oracle's
/// `variables/switches.php` leaves this case out.
#[test]
fn a_switch_enters_default_after_every_case_value() {
    let source = "<?php\nfunction d(int $n) {\n    switch ($n) {\n        default:\n            echo $late;\n            break;\n        case 1:\n            echo $late;\n            break;\n        case $late = 2:\n            echo $late;\n    }\n}\n";
    let table = table(source);
    let flow = table.php.as_deref().expect("a PHP flow");
    let defined: BTreeMap<usize, Defined> = flow
        .reads
        .iter()
        .filter(|(_, read)| &*read.name == "$late")
        .map(|(start, read)| (*start, read.defined))
        .collect();
    assert_eq!(
        defined.into_values().collect::<Vec<_>>(),
        [Defined::Yes, Defined::No, Defined::Yes]
    );
}

/// A callee throws after PHP has evaluated its arguments, so a `catch` sees their effects even
/// when the call ends the path (`return f(...)`): `php -r 'function f($x){ throw new Exception; }
/// function g(){ try { return f($b = 2); } catch (Exception $e) { var_dump($b); } } g();'` prints
/// `int(2)`, and with `return run(function () use (&$x) { $x = 1; })` (where `run` calls the
/// closure, then throws) the catch's `var_dump($x)` prints `int(1)`.
#[test]
fn a_catch_sees_what_a_throwing_call_evaluated() {
    let source = "<?php\nfunction g($db) {\n    $x = null;\n    try {\n        return f($b = 2, function () use (&$x) {\n            $x = 1;\n        });\n    } catch (Exception $e) {\n        echo $b;\n        echo $x;\n    }\n}\n";
    let table = table(source);
    let flow = table.php.as_deref().expect("a PHP flow");
    // The catch's reads: the last occurrence of each name in the source.
    let read = |name: &str| {
        let start = source
            .rfind(&format!("{name};"))
            .expect("a read in the catch");
        flow.read(start).expect("a read in the catch")
    };
    // Thrown before the call (`$b` unassigned) or by the callee (`$b` assigned): some paths.
    assert_eq!(read("$b").defined, Defined::Maybe);
    // The closure may have written `$x` through its by-reference `use` before the callee threw.
    assert!(
        read("$x").aliased,
        "the catch's `$x` is shared with the closure"
    );
}

/// A throw in a catch runs the `finally` before the exception leaves: `php -r 'function bar() {
/// try { throw new Exception("x"); } catch (Exception $e) { $b = 4; throw new Exception("bad"); }
/// finally { echo $b; } } try { bar(); } catch (Exception $e) {}'` prints `4` (Psalm 6.19.1's
/// UnusedVariableTest `useTryAndCatchAssignedVariableInsideFinally`).
#[test]
fn a_throw_in_a_catch_runs_the_finally() {
    let source = "<?php\nfunction bar() {\n    try {\n        throw new Exception('x');\n    } catch (Exception $e) {\n        $b = 4;\n        throw new Exception('bad');\n    } finally {\n        echo $b;\n    }\n}\n";
    let table = table(source);
    let flow = table.php.as_deref().expect("a PHP flow");
    let read = flow
        .reads()
        .find(|read| &*read.name == "$b")
        .expect("the finally's read");
    assert_eq!(
        read.bindings.len(),
        1,
        "the catch's `$b = 4` reaches the finally"
    );
    assert_ne!(read.defined, Defined::No);
}

/// PHP's variable predicates through a compiled code rule: what each finds in one function.
#[test]
fn php_variable_predicates_evaluate_through_code_rules() {
    use crate::code_rule_dsl::test_support::*;
    use serde_json::json;
    let source = "<?php\nfunction f(bool $c) {\n    $limit = 10;\n    echo $undefined;\n    if ($c) {\n        $maybe = 1;\n    }\n    echo $maybe;\n    $unused = strlen('x');\n    $x = $limit;\n    echo $x;\n    echo $_GET['q'];\n}\n";
    let capabilities: Json = serde_json::from_str(CAPABILITIES).expect("capabilities");
    let run = |rule: Json| {
        let compiled = compile_document_with(
            clause_document(json!({
                "id": "php-variables",
                "forbid": { "ast": { "language": "php", "rule": rule } },
                "violation": violation(),
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

/// A statement-level `$a++`, `$a += v` or `$a = $a + v` reads `$a` only to rebind it, so the
/// binding it reads is used only if the new one is, as Psalm 6.19.1 reports `UnusedVariable`
/// (UnusedVariableTest's `unusedVarWithIncrement`, `unusedVarWithAdditionOp`,
/// `unusedConditionalCode`): `$a = 5; $a++;` reports both. A value something else takes
/// (`echo $d++`, an argument) is used.
#[test]
fn php_self_updates_use_what_they_read_only_if_their_binding_is_used() {
    use crate::code_rule_dsl::test_support::*;
    use serde_json::json;
    let source = "<?php\nfunction f(int $n) {\n    $a = 5;\n    $a++;\n    $b = 1;\n    $b = $b + 1;\n    $c = 1;\n    $c++;\n    echo $c;\n    $d = 1;\n    echo $d++;\n    $e = 0;\n    while ($n--) { $e += $n; }\n    $f = 1;\n    $f = strlen($f) + $f;\n    $g = 1;\n    $g .= 'x';\n    return $g;\n}\n";
    let capabilities: Json = serde_json::from_str(CAPABILITIES).expect("capabilities");
    let compiled = compile_document_with(
        clause_document(json!({
            "id": "php-self-updates",
            "forbid": { "ast": { "language": "php", "rule": { "kind": "variable_name", "binds": { "used": false } } } },
            "violation": violation(),
        })),
        &capabilities,
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, "php", "f.php", source);
    let found: Vec<String> = result["violations"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|violation| {
            let start = violation["span"]["startByte"].as_u64().expect("start") as usize;
            let end = violation["span"]["endByte"].as_u64().expect("end") as usize;
            format!("{}:{}", line_of(source, start), &source[start..end])
        })
        .collect();
    assert_eq!(
        found,
        [
            "3:$a", "4:$a", "5:$b", "6:$b", "11:$d", "12:$e", "13:$e", "15:$f"
        ]
    );
}

/// A function's `global $a` reads the global `$a` whenever the function is called: the global
/// scope's bindings of `$a` are not unused (Psalm 6.19.1 UnusedVariableTest's valid
/// `globalVariableUsage`; `php -r '$a = "hello"; function e() { global $a; echo $a; } e();'`
/// prints `hello`). A global no `global` statement names stays unused.
#[test]
fn php_global_statements_may_use_the_global_scopes_bindings() {
    use crate::code_rule_dsl::scope::usage::{BindingId, Usage};
    let source = "<?php\n$a = 'hello';\n$b = 'unused';\nfunction example(): void {\n    global $a;\n    echo $a;\n}\nexample();\n";
    let root = GrammarDialect::Php.ast_grep(source);
    let table = table(source);
    let usage = Usage::build(&table, &root.root(), usize::MAX, &|| false).expect("usage");
    let used = |name: &str| {
        usage
            .facts(BindingId {
                scope: 0,
                name,
                index: 0,
            })
            .used
    };
    assert_eq!(used("$a"), None);
    assert_eq!(used("$b"), Some(false));
}

/// A closure's `use (&$x)` reads what `$x` holds when the closure runs (Psalm 6.19.1 reports
/// neither; laravel's `Collection::unique` fills `$exists = []` this way), and `$a[k] ??= v`
/// keeps the chain of changes to `$a` (symfony's `DoctrineDataCollector::sanitizeQuery`).
#[test]
fn php_reference_uses_and_subscript_coalesces_keep_what_they_read_used() {
    use crate::code_rule_dsl::scope::usage::{BindingId, Usage};
    let source = "<?php\nfunction f(array $items, array $q) {\n    $exists = [];\n    $g = function ($item) use (&$exists) { return in_array($item, $exists); };\n    $q['a'] = 1;\n    $q['b'] ??= [];\n    return [$g, $q];\n}\n";
    let root = GrammarDialect::Php.ast_grep(source);
    let table = table(source);
    let usage = Usage::build(&table, &root.root(), usize::MAX, &|| false).expect("usage");
    let function = table
        .scopes
        .iter()
        .position(|scope| scope.bindings.contains_key("$exists"))
        .expect("f's scope") as u32;
    let used = |name: &str, index: usize| {
        usage
            .facts(BindingId {
                scope: function,
                name,
                index,
            })
            .used
    };
    assert_eq!(used("$exists", 0), Some(true));
    // `$q`: the parameter, `$q['a'] = 1`, `$q['b'] ??= []`.
    assert_eq!(
        [used("$q", 0), used("$q", 1), used("$q", 2)],
        [Some(true); 3]
    );
}

/// A fluent chain (`$builder->a()->b()->...`, as Symfony's bundle configurators write hundreds of
/// calls long) runs from the inside out: an assignment in an inner call's argument defines the
/// variable for every outer call. The flow walks the chain without recursing per call, so its
/// length does not exhaust the build's depth or stack.
#[test]
fn a_long_method_chain_runs_from_the_inside_out() {
    let mut source = String::from("<?php\nfunction f($d) {\n    $d->a($x = 1)");
    for _ in 0..400 {
        source.push_str("\n        ->b(static fn ($v) => $v)");
    }
    source.push_str("\n        ->c($x, $u);\n}\n");
    let table = try_table(&source).expect("the chain builds within the budget");
    let flow = table.php.as_deref().expect("a PHP flow");
    let read = |name: &str| {
        let start = source.rfind(name).expect("the outermost call's argument");
        flow.read(start).expect("a read").defined
    };
    assert_eq!(read("$x"), Defined::Yes);
    assert_eq!(read("$u"), Defined::No);
}

/// `eval` runs its code in the calling scope, which may read any variable: `php -r 'function f()
/// { $x = 1; eval("echo \$x;"); } f();'` prints `1` (laravel's BladeBoolTest evaluates compiled
/// Blade reading `$someViewVarTruthy` so; Psalm 6.19.1 reports nothing there).
#[test]
fn php_eval_may_read_every_variable_of_its_scope() {
    use crate::code_rule_dsl::scope::usage::{BindingId, Usage};
    let source = "<?php\nfunction f(string $code) {\n    $x = 1;\n    eval($code);\n}\nfunction g() {\n    $y = 1;\n}\n";
    let root = GrammarDialect::Php.ast_grep(source);
    let table = table(source);
    let usage = Usage::build(&table, &root.root(), usize::MAX, &|| false).expect("usage");
    let used = |name: &str| {
        let scope = table
            .scopes
            .iter()
            .position(|scope| scope.bindings.contains_key(name))
            .expect("a scope binding it") as u32;
        usage
            .facts(BindingId {
                scope,
                name,
                index: 0,
            })
            .used
    };
    assert_eq!(used("$x"), None);
    assert_eq!(used("$y"), Some(false));
}

/// Bindings a read the flow cannot see may use are not unused: what reaches a `goto` (`php -r
/// 'function j() { $t = 1; goto done; done: echo $t; } j();'` prints `1`), what an `unset` destroys
/// (`$o = new D(); unset($o);` runs `D::__destruct` there; Psalm 6.19.1 counts it a use), and a
/// variable a heredoc interpolates after an `<?php` in its text, which tree-sitter-php 0.24.2
/// reads as a comment (symfony's `WcswidthDataGenerator::getHeader`). A plain dead store stays
/// unused.
#[test]
fn php_unseen_reads_leave_bindings_unknown() {
    use crate::code_rule_dsl::scope::usage::{BindingId, Usage};
    let source = "<?php\nfunction jumps(int $n) {\n    $t = 1;\n    goto done;\n    done:\n    return $t + $n;\n}\nfunction destroys() {\n    $o = new ArrayObject();\n    unset($o);\n    $plain = 1;\n}\nfunction header(string $v) {\n    $date = 'x';\n\n    return <<<EOT\n        <?php\n\n        /*\n         * Unicode version: $v\n         * Date: $date\n         */\n\n        EOT;\n}\n";
    let root = GrammarDialect::Php.ast_grep(source);
    let table = table(source);
    let usage = Usage::build(&table, &root.root(), usize::MAX, &|| false).expect("usage");
    let used = |name: &str| {
        let scope = table
            .scopes
            .iter()
            .position(|scope| scope.bindings.contains_key(name))
            .expect("a scope binding it") as u32;
        usage
            .facts(BindingId {
                scope,
                name,
                index: 0,
            })
            .used
    };
    assert_eq!(
        [used("$t"), used("$o"), used("$date"), used("$plain")],
        [None, None, None, Some(false)]
    );
}

/// A closure's by-value `use` copy holds what reaches its clause: appending to a copy of an array
/// calls no `offsetSet`, so the copy is unused (Psalm's UnusedVariableTest
/// `useArrayAssignmentNeverUsed`); appending to a copy of an object may be its use.
#[test]
fn php_use_copy_of_an_array_is_unused_when_only_appended_to() {
    use crate::code_rule_dsl::scope::usage::{BindingId, Usage};
    let used = |outer: &str| {
        let source = format!(
            "<?php\nfunction f() {{\n    $data = {outer};\n    return function () use ($data) {{\n        $data[] = 1;\n    }};\n}}\n"
        );
        let root = GrammarDialect::Php.ast_grep(&source);
        let table = table(&source);
        let usage = Usage::build(&table, &root.root(), usize::MAX, &|| false).expect("usage");
        let closure = table
            .scopes
            .iter()
            .rposition(|scope| scope.bindings.contains_key("$data"))
            .expect("the closure binds $data") as u32;
        usage
            .facts(BindingId {
                scope: closure,
                name: "$data",
                index: 0,
            })
            .used
    };
    assert_eq!([used("[]"), used("new ArrayObject()")], [Some(false), None]);
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
    use crate::code_rule_dsl::test_support::*;
    use serde_json::json;
    let source = "<?php\nfunction f() {\n    $a = [];\n    $a[] = 1;\n}\n";
    let capabilities: Json = serde_json::from_str(CAPABILITIES).expect("capabilities");
    let lines = |filter: Json| -> Vec<u64> {
        let compiled = compile_document_with(
            clause_document(json!({
                "id": "php-references",
                "forbid": { "ast": { "language": "php", "rule": { "kind": "variable_name", "binds": filter } } },
                "violation": violation(),
            })),
            &capabilities,
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "php", "f.php", source);
        result["violations"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|violation| {
                let start = violation["span"]["startByte"].as_u64().expect("start") as usize;
                line_of(source, start)
            })
            .collect()
    };
    assert_eq!(lines(json!({ "used": false })), [3, 4]);
    assert_eq!(lines(json!({ "referenced": false })), [4]);
    assert_eq!(lines(json!({ "referenced": true })), [3]);
}
