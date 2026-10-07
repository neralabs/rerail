//! `formatArgs` with style `printf` against where PHPStan 2.2.16's `PrintfParametersRule` and
//! `PrintfArrayParametersRule` report on fixtures (`fixtures/php/format_args`) and over whole
//! corpora (`benchmark_php_format_corpus`).

use crate::policy_dsl::CompiledPolicyDsl;
use crate::policy_dsl::language::GrammarDialect;
use crate::policy_dsl::oracle;
use crate::policy_dsl::test_support::*;
use ast_grep_core::tree_sitter::LanguageExt;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const ORACLE: &str = oracle!("php/format_args/phpstan.expected.json");
const FIXTURES: &[(&str, &str)] = &[
    ("calls.php", oracle!("php/format_args/phpstan/calls.php")),
    (
        "constants.php",
        oracle!("php/format_args/phpstan/constants.php"),
    ),
    (
        "defines.php",
        oracle!("php/format_args/phpstan/defines.php"),
    ),
    ("follow.php", oracle!("php/format_args/phpstan/follow.php")),
    (
        "heredocs.php",
        oracle!("php/format_args/phpstan/heredocs.php"),
    ),
    ("shapes.php", oracle!("php/format_args/phpstan/shapes.php")),
];

/// The calls `formatArgs` leaves unknown, by the fixture line each starts on (the only call that
/// starts there), besides a literal format PHP refuses. Those PHPStan does not read either: an
/// `...` unpacking, an interpolated literal, values that are no array literal or an array literal
/// with a key not known, a parameter, `static::X` of a class a subclass may redeclare it in, and
/// a constant or class the file does not declare. Those PHPStan reads and Rerail does not know: a
/// variable whose value differs by path (PHPStan reads the most placeholders any value has), a
/// compound assignment (`.=`), an interpolated string in a variable, `self::class`, a constant
/// PHP defines (`PHP_EOL`), a `define` that may run or not, and a format PHP refuses with a
/// `ValueError` through a variable (PHPStan's "invalid placeholder"; `parse` reads a literal).
const UNDECIDED: &[(&str, &[u64])] = &[
    ("calls.php", &[41, 42, 43, 64, 65, 79, 80]),
    ("constants.php", &[52, 53, 54, 95, 117]),
    ("defines.php", &[21]),
    ("follow.php", &[21, 22, 59, 65, 68, 70, 72, 73]),
    ("heredocs.php", &[19]),
    ("shapes.php", &[41, 43]),
];

/// The line (from 1) of a byte of `source`.
fn line_of(source: &str, offset: usize) -> u64 {
    source[..offset].matches('\n').count() as u64 + 1
}

/// A node a rule finds: the line (from 1) it starts on, its start byte, its source.
type Found<'s> = (u64, usize, &'s str);

/// The node of `source` spanning `start..end`, as `Found`.
fn found_at(source: &str, start: usize, end: usize) -> Found<'_> {
    (line_of(source, start), start, &source[start..end])
}

/// The nodes a PHP rule finds in `source`.
fn found<'s>(ast: Value, source: &'s str) -> BTreeSet<Found<'s>> {
    let mut ast = ast;
    ast["language"] = json!("php");
    let capabilities: Value = serde_json::from_str(CAPABILITIES).expect("capabilities");
    let compiled = compile_document_with(
        check_document(json!({ "id": "printf", "forbid": { "ast": ast }, "violation": finding() })),
        &capabilities,
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, "php", "f.php", source);
    assert_eq!(result["failures"], json!([]), "{result}");
    result["violations"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|finding| {
            let at = |key: &str| finding["span"][key].as_u64().expect(key) as usize;
            found_at(source, at("startByte"), at("endByte"))
        })
        .collect()
}

/// The family's calls as a call spells them: unqualified, or fully qualified (`\sprintf`, which
/// namespaced code writes to skip the namespace fallback).
fn spellings(patterns: &[&str]) -> Value {
    patterns
        .iter()
        .flat_map(|pattern| {
            [
                json!({ "pattern": pattern }),
                json!({ "pattern": format!("\\{pattern}") }),
            ]
        })
        .collect()
}

/// The lines (from 1) a PHP rule finds in `source`.
fn lines(ast: Value, source: &str) -> BTreeSet<u64> {
    found(ast, source)
        .into_iter()
        .map(|(line, _, _)| line)
        .collect()
}

/// `formatArgs` over the variadic calls (`sprintf`, `printf`; `$$$A` matches no node after a
/// comma, so a call with the format alone is a pattern of its own), the array ones (`vsprintf`,
/// `vprintf`) and the scanf ones (`sscanf`, `fscanf`), holding where `side` (`unused` or
/// `missing`) has any value or field.
fn format_args(side: &str) -> Value {
    json!({ "rule": { "any": format_args_terms(side) } })
}

/// The calls `format_args` reads, each with `formatArgs` negated: those whose format and values
/// are known and have no value or field `side`.
fn format_args_negated(side: &str) -> Value {
    let mut terms = format_args_terms(side);
    for term in &mut terms {
        let test = term["formatArgs"].take();
        let call = term["any"].take();
        *term = json!({ "all": [{ "any": call }, { "not": { "formatArgs": test } }] });
    }
    json!({ "rule": { "any": terms } })
}

fn format_args_terms(side: &str) -> [Value; 3] {
    let test = |style: &str, args: &str, unused: &str| {
        let mut test = json!({ "style": style, "string": "F", "args": args });
        test[side] = match (side, style) {
            ("unused", _) => json!({ "kind": unused }),
            (_, "printf") => json!({ "any": [{ "kind": "placeholder" }, { "kind": "star" }] }),
            _ => json!({ "kind": "placeholder" }),
        };
        test
    };
    [
        json!({
            "any": spellings(&["sprintf($F)", "printf($F)", "sprintf($F, $$$A)", "printf($F, $$$A)"]),
            "formatArgs": test("printf", "A", "argument"),
        }),
        json!({
            "any": spellings(&["vsprintf($F, $V)", "vprintf($F, $V)"]),
            "formatArgs": test("printf", "V", "array_element_initializer"),
        }),
        json!({
            "any": spellings(&[
                "sscanf($S, $F)", "fscanf($S, $F)", "sscanf($S, $F, $$$A)", "fscanf($S, $F, $$$A)",
            ]),
            "formatArgs": test("scanf", "A", "argument"),
        }),
    ]
}

/// Every call of the family, whatever its arguments.
fn family_calls() -> Value {
    json!({ "rule": { "any": spellings(&[
        "sprintf($$$A)", "printf($$$A)", "vsprintf($$$A)", "vprintf($$$A)", "sscanf($$$A)",
        "fscanf($$$A)",
    ]) } })
}

/// The calls whose literal format PHP refuses (PHPStan's "contains an invalid placeholder"): the
/// printf ones, then the scanf ones.
fn invalid_formats() -> [Value; 2] {
    let terms = [
        (
            [
                "sprintf($F)",
                "printf($F)",
                "sprintf($F, $$$A)",
                "printf($F, $$$A)",
                "vsprintf($F, $$$A)",
                "vprintf($F, $$$A)",
            ]
            .as_slice(),
            "printfFormat",
        ),
        (
            [
                "sscanf($S, $F)",
                "fscanf($S, $F)",
                "sscanf($S, $F, $$$A)",
                "fscanf($S, $F, $$$A)",
            ]
            .as_slice(),
            "scanfFormat",
        ),
    ];
    terms.map(|(patterns, grammar)| {
        json!({
            "rule": { "any": spellings(patterns) },
            // `$F` captures the argument; `parse` holds on the literal it passes.
            "constraints": { "F": { "has": {
                "parse": { "grammar": grammar, "has": { "kind": "error" } },
            } } },
        })
    })
}

/// What Rerail claims of one printf-family call.
#[derive(Clone, Copy, Default)]
struct Verdict {
    /// A value no placeholder reads.
    unused: bool,
    /// A placeholder no value fills.
    missing: bool,
    /// A literal format PHP refuses.
    invalid: bool,
    /// The format and values are known and agree (or the format is no string).
    agrees: bool,
}

impl Verdict {
    fn mismatch(self) -> bool {
        self.unused || self.missing || self.invalid
    }

    fn decided(self) -> bool {
        self.mismatch() || self.agrees
    }
}

/// One policy whose checks give Rerail's verdict on every printf-family call of a file: its
/// unused values and missing placeholders (`formatArgs`), a literal format PHP refuses, and the
/// calls whose format and values are known with nothing unused.
struct Verdicts(CompiledPolicyDsl);

impl Verdicts {
    fn new() -> Self {
        // A call `agrees` with nothing unused may still miss a value: `missing` decides it.
        let [invalid_printf, invalid_scanf] = invalid_formats();
        let checks = [
            ("calls", "calls", family_calls()),
            ("unused", "unused", format_args("unused")),
            ("missing", "missing", format_args("missing")),
            ("invalid", "invalid", invalid_printf),
            ("invalidScanf", "invalid", invalid_scanf),
            ("agrees", "agrees", format_args_negated("unused")),
        ];
        let checks: Vec<Value> = checks
            .into_iter()
            .map(|(id, code, mut ast)| {
                ast["language"] = json!("php");
                json!({
                    "id": id,
                    "forbid": { "ast": ast },
                    "violation": { "anchor": "witness.span", "code": code, "message": "x" },
                })
            })
            .collect();
        let capabilities: Value = serde_json::from_str(CAPABILITIES).expect("capabilities");
        Self(
            compile_document_with(
                json!({ "format": "rerail.policy/v1", "id": "printf", "revision": 1, "mode": "steer", "clauses": checks }),
                &capabilities,
            )
            .unwrap_or_else(|error| panic!("{}", error.reason)),
        )
    }

    /// Each call of `source` by its span (start and end byte), with Rerail's verdict; `Err` with
    /// the evaluation's failures. A claim is the verdict of the call it spans exactly.
    fn of(&self, source: &str) -> Result<BTreeMap<(usize, usize), Verdict>, Value> {
        let result = evaluate_file(&self.0, "php", "f.php", source);
        if result["failures"] != json!([]) {
            return Err(result["failures"].clone());
        }
        let mut calls = BTreeMap::new();
        let mut claims = Vec::new();
        for finding in result["violations"].as_array().expect("findings") {
            let at = |key: &str| finding["span"][key].as_u64().expect(key) as usize;
            let span = (at("startByte"), at("endByte"));
            match finding["code"].as_str().expect("code") {
                "calls" => {
                    calls.insert(span, Verdict::default());
                }
                code => claims.push((span, code.to_owned())),
            }
        }
        for (span, code) in claims {
            let verdict = calls.get_mut(&span).expect("a claim is on a call");
            match code.as_str() {
                "unused" => verdict.unused = true,
                "missing" => verdict.missing = true,
                "invalid" => verdict.invalid = true,
                _ => verdict.agrees = true,
            }
        }
        Ok(calls)
    }
}

#[test]
fn printf_arguments_match_phpstan() {
    let oracle: Value = serde_json::from_str(ORACLE).expect("oracle");
    let verdicts = Verdicts::new();
    let mut reports = 0;
    for (name, source) in FIXTURES {
        let calls = verdicts.of(source).expect("evaluates");
        let found_where = |holds: fn(&Verdict) -> bool| -> BTreeSet<Found<'_>> {
            calls
                .iter()
                .filter(|(_, verdict)| holds(verdict))
                .map(|(&(start, end), _)| found_at(source, start, end))
                .collect()
        };
        let all = found_where(|_| true);
        // PHPStan reports a call on the line it starts on. A line reported or undecided starts
        // one call, the one each finding must be: a finding on a call around it or inside it, or
        // an undecided call beside a decided one, is no match.
        let call_on = |line: u64| {
            let on: Vec<_> = all.iter().filter(|call| call.0 == line).collect();
            match on.as_slice() {
                [call] => **call,
                _ => panic!("{name}:{line}: one call starts on the line, not {on:?}"),
            }
        };
        let unknown: BTreeSet<u64> = UNDECIDED
            .iter()
            .filter(|(fixture, _)| fixture == name)
            .flat_map(|(_, lines)| lines.iter().copied())
            .collect();
        let mut expected = [BTreeSet::new(), BTreeSet::new(), BTreeSet::new()];
        for report in oracle[name].as_array().expect("reports") {
            reports += 1;
            let line = report[0].as_u64().expect("line");
            if unknown.contains(&line) {
                continue;
            }
            let side = match (&report[1], &report[2]) {
                (Value::String(_), _) => 2,
                (placeholders, values) if placeholders.as_u64() > values.as_u64() => 1,
                _ => 0,
            };
            expected[side].insert(call_on(line));
        }
        assert_eq!(found_where(|v| v.unused), expected[0], "{name}: unused");
        assert_eq!(found_where(|v| v.missing), expected[1], "{name}: missing");
        assert_eq!(found_where(|v| v.invalid), expected[2], "{name}: invalid");
        // Every other call is decided: its format and values are known, or its format is no
        // string.
        let undecided = &(&all - &found_where(|v| v.decided())) - &expected[2];
        let unknown: BTreeSet<_> = unknown.into_iter().map(call_on).collect();
        assert_eq!(undecided, unknown, "{name}: undecided calls");
    }
    assert!(reports >= 60, "{reports} reports");
}

/// Where PHPStan's `argument.*printf`/`argument.*scanf` and Rerail's verdict differ on the corpora
/// and Rerail is not wrong: corpus, path under the corpus root, line, and the evidence.
const PHPSTAN_DIVERGENCES: &[(&str, &str, u64, &str)] = &[];

/// The PHP files under `root`, Blade templates aside.
fn php_files(root: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(root).expect("corpus directory") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            php_files(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "php")
            && !path.to_string_lossy().ends_with(".blade.php")
        {
            out.push(path);
        }
    }
}

/// The function a call at `start` names, lowercased, without a leading backslash.
fn callee(source: &str, start: usize) -> String {
    source[start..]
        .trim_start_matches('\\')
        .chars()
        .take_while(|char| char.is_ascii_alphanumeric() || *char == '_')
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Whether Rerail's mismatch is the one PHPStan reports: an invalid format, more placeholders
/// than values (`missing`), fewer (`unused`), or a count PHPStan gives as a range (any).
fn same_kind(verdict: Verdict, kind: &str) -> bool {
    match kind {
        "invalid" => verdict.invalid,
        "missing" => verdict.missing,
        "unused" => verdict.unused,
        _ => verdict.mismatch(),
    }
}

fn describe(verdict: Verdict) -> String {
    let claims: Vec<&str> = [
        (verdict.unused, "unused value"),
        (verdict.missing, "missing value"),
        (verdict.invalid, "invalid format"),
    ]
    .into_iter()
    .filter_map(|(holds, claim)| holds.then_some(claim))
    .collect();
    match (claims.is_empty(), verdict.agrees) {
        (false, _) => claims.join(" and "),
        (true, true) => "format and values agree".to_owned(),
        (true, false) => "undecided".to_owned(),
    }
}

#[derive(Default)]
struct Tally {
    unparsed: usize,
    not_utf8: usize,
    unbuilt: Vec<String>,
    items: usize,
    decided: usize,
    rerail_positives: usize,
    reference_positives: usize,
    reports_in_skipped: usize,
    true_positives: usize,
    recall_undecided: usize,
    recall_no_call: usize,
    flagged_silent: usize,
    reported_agrees: usize,
    other_kind: usize,
}

/// PHPStan's printf-family reports over whole corpora (`RERAIL_PHP_FORMAT`, written by
/// `fixtures/php/format_args/generate_corpus.py`) against Rerail's verdict on every call of
/// the family; the report section goes to `RERAIL_PHP_FORMAT_REPORT`. A call Rerail flags where
/// PHPStan is silent, or PHPStan reports where Rerail says the format and values agree (or
/// claims the other side), is a contradiction; one PHPStan reports where Rerail is undecided is a
/// recall miss.
#[test]
#[ignore = "PHPStan printf-family reports over full PHP corpora"]
fn benchmark_php_format_corpus() {
    let input = std::env::var("RERAIL_PHP_FORMAT").expect("RERAIL_PHP_FORMAT");
    let output = std::env::var("RERAIL_PHP_FORMAT_REPORT").expect("report path");
    let recorded: Value =
        serde_json::from_str(&std::fs::read_to_string(input).expect("input")).expect("JSON");
    let verdicts = Verdicts::new();
    let mut sections = Vec::new();
    let mut divergences = Vec::new();
    let mut unexplained = Vec::new();
    let mut documented = BTreeSet::new();
    for corpus in recorded["corpora"].as_array().expect("corpora") {
        let name = corpus["name"].as_str().expect("name");
        let root = format!("{}/", corpus["root"].as_str().expect("root"));
        let excluded: BTreeSet<&str> = corpus["excluded"]
            .as_array()
            .expect("excluded")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let mut files = Vec::new();
        for path in corpus["paths"].as_array().expect("paths") {
            php_files(Path::new(path.as_str().expect("path")), &mut files);
        }
        files.sort();
        files.retain(|file| !excluded.contains(file.to_string_lossy().as_ref()));
        let reports = corpus["reports"].as_object().expect("reports");
        let mut tally = Tally::default();
        for file in &files {
            let key = file.to_string_lossy();
            let relative = key.strip_prefix(&root).unwrap_or(&key).to_owned();
            let reported: Vec<&Value> = reports
                .get(key.as_ref())
                .and_then(Value::as_array)
                .map(|reports| reports.iter().collect())
                .unwrap_or_default();
            // A snapshot holds UTF-8 sources only (a Latin-1 fixture is none).
            let Ok(source) = std::fs::read_to_string(file) else {
                tally.not_utf8 += 1;
                tally.reports_in_skipped += reported.len();
                continue;
            };
            // A file the grammar reads with errors is outside what Rerail decides, as tape
            // parity reports it.
            if GrammarDialect::Php
                .ast_grep(&source)
                .root()
                .dfs()
                .any(|node| node.is_error() || node.is_missing())
            {
                tally.unparsed += 1;
                tally.reports_in_skipped += reported.len();
                continue;
            }
            let calls = match verdicts.of(&source) {
                Ok(calls) => calls,
                Err(failures) => {
                    tally.unbuilt.push(format!("{relative}: {failures}"));
                    continue;
                }
            };
            // Calls and reports by line and function: PHPStan reports a call on the line its
            // name starts.
            type Group<'a> = (Vec<Verdict>, Vec<&'a Value>);
            let mut groups: BTreeMap<(u64, String), Group<'_>> = BTreeMap::new();
            for (&(start, _), verdict) in &calls {
                tally.items += 1;
                tally.decided += usize::from(verdict.decided());
                tally.rerail_positives += usize::from(verdict.mismatch());
                groups
                    .entry((line_of(&source, start), callee(&source, start)))
                    .or_default()
                    .0
                    .push(*verdict);
            }
            for report in reported {
                tally.reference_positives += 1;
                let line = report["line"].as_u64().expect("line");
                let function = report["function"].as_str().expect("function").to_owned();
                groups.entry((line, function)).or_default().1.push(report);
            }
            for ((line, function), (verdicts, reports)) in groups {
                let mut open: Vec<Option<Verdict>> = verdicts.into_iter().map(Some).collect();
                let mut take = |holds: &dyn Fn(Verdict) -> bool| {
                    open.iter_mut()
                        .find(|verdict| verdict.is_some_and(holds))
                        .and_then(Option::take)
                };
                let mut found = Vec::new();
                for report in reports {
                    let kind = report["kind"].as_str().expect("kind");
                    let message = report["message"].as_str().expect("message");
                    if take(&|verdict| same_kind(verdict, kind)).is_some() {
                        tally.true_positives += 1;
                    } else if let Some(verdict) = take(&Verdict::mismatch) {
                        tally.other_kind += 1;
                        found.push((describe(verdict), message.to_owned()));
                    } else if take(&|verdict| !verdict.decided()).is_some() {
                        tally.recall_undecided += 1;
                        eprintln!("recall miss (undecided) {name}:{relative}:{line}: {message}");
                    } else if let Some(verdict) = take(&|verdict| verdict.agrees) {
                        tally.reported_agrees += 1;
                        found.push((describe(verdict), message.to_owned()));
                    } else {
                        tally.recall_no_call += 1;
                        eprintln!(
                            "recall miss (no {function} call) {name}:{relative}:{line}: {message}"
                        );
                    }
                }
                for verdict in open.into_iter().flatten() {
                    if verdict.mismatch() {
                        tally.flagged_silent += 1;
                        found.push((describe(verdict), "silent".to_owned()));
                    }
                }
                for (rerail, reference) in found {
                    let evidence = PHPSTAN_DIVERGENCES.iter().find(|(corpus, path, at, _)| {
                        *corpus == name && *path == relative && *at == line
                    });
                    match evidence {
                        Some(entry) => {
                            documented.insert((entry.0, entry.1, entry.2));
                            divergences.push(json!({
                                "corpus": name, "path": relative, "line": line,
                                "rerail": rerail, "reference": reference, "evidence": entry.3,
                            }));
                        }
                        None => unexplained.push(format!(
                            "{name}:{relative}:{line}: {function}: Rerail {rerail}, PHPStan {reference}"
                        )),
                    }
                }
            }
        }
        let contradictions = tally.flagged_silent + tally.reported_agrees + tally.other_kind;
        let ratio = |part: usize, whole: usize| {
            if whole == 0 {
                1.0
            } else {
                part as f64 / whole as f64
            }
        };
        sections.push(json!({
            "name": name,
            "files": files.len(),
            "filesWithSyntaxErrors": tally.unparsed,
            "unbuilt": tally.unbuilt,
            "items": tally.items,
            "decided": tally.decided,
            "decidedRate": ratio(tally.decided, tally.items),
            "referencePositives": tally.reference_positives,
            "rerailPositives": tally.rerail_positives,
            "truePositives": tally.true_positives,
            "contradictions": contradictions,
            // Every contradiction is a call Rerail decides.
            "precision": 1.0 - ratio(contradictions, tally.decided.max(1)),
            // No reference positive leaves recall undefined.
            "recall": (tally.reference_positives > 0)
                .then(|| ratio(tally.true_positives, tally.reference_positives)),
            "rerailFlagsPhpstanSilent": tally.flagged_silent,
            "phpstanReportsRerailAgrees": tally.reported_agrees,
            "phpstanReportsOtherKind": tally.other_kind,
            "recallMissesUndecided": tally.recall_undecided,
            "recallMissesNoCall": tally.recall_no_call,
            "filesNotUtf8": tally.not_utf8,
            "phpstanReportsInUncomparedFiles": tally.reports_in_skipped,
            "excludedFiles": excluded.len(),
        }));
    }
    let report = json!({
        "capability": "format",
        "reference": {
            "tool": "PHPStan",
            "version": recorded["phpstan"],
            "php": recorded["php"],
            "signal": "argument.sprintf, argument.printf, argument.vsprintf, argument.vprintf, argument.sscanf and argument.fscanf (PrintfParametersRule, PrintfArrayParametersRule, level 0): a call whose placeholder count differs from the values passed, or whose format contains an invalid placeholder",
        },
        "method": "Items are every call of sprintf, printf, vsprintf, vprintf, sscanf and fscanf (unqualified or fully qualified) in the corpus files tree-sitter-php parses without errors (Blade templates, files `php -l` rejects and non-UTF-8 files excluded), as one Rerail policy enumerates them. Rerail claims a mismatch where `formatArgs` (style printf or scanf) finds a value no placeholder reads (`unused`) or a placeholder no value fills (`missing`), or `parse` (printfFormat/scanfFormat) finds an error in the literal format; it claims agreement where `not formatArgs` holds (format and values known, nothing unused, nothing missing); other calls are undecided (a format or values Rerail cannot know: a parameter, a property, a call's result, a ternary, an interpolated or partly unknown concatenation, a class constant of another file, an `...` unpacking). PHPStan's reports are matched to calls by line and function. A contradiction is a call Rerail flags where PHPStan is silent, PHPStan reports where Rerail claims agreement, or both report but on different sides (missing vs unused vs invalid). precision = 1 - contradictions/decided; recall = PHPStan reports Rerail also flags (same side) / PHPStan reports in compared files (null where PHPStan reports none); decidedRate = decided calls / calls. PHPStan reports Rerail leaves undecided, or on lines with no call of that function Rerail enumerates, are recall misses.",
        "corpora": sections,
        "divergences": divergences,
    });
    std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).expect("write report");
    let stale: Vec<_> = PHPSTAN_DIVERGENCES
        .iter()
        .filter(|(corpus, path, line, _)| !documented.contains(&(*corpus, *path, *line)))
        .collect();
    assert!(
        stale.is_empty(),
        "divergences no longer observed: {stale:?}"
    );
    assert!(
        unexplained.is_empty(),
        "{} unexplained contradictions:\n{}",
        unexplained.len(),
        unexplained.join("\n")
    );
}

/// Compiles one ast query in `language`.
fn compile(language: &str, rule: Value) -> napi::Result<crate::policy_dsl::CompiledPolicyDsl> {
    let capabilities: Value = serde_json::from_str(CAPABILITIES).expect("capabilities");
    compile_document_with(
        check_document(json!({
            "id": "printf",
            "forbid": { "ast": { "language": language, "rule": rule } },
            "violation": { "anchor": "witness.span", "code": "X", "message": "x" },
        })),
        &capabilities,
    )
}

#[test]
fn a_format_tree_maps_placeholders_to_their_source() {
    let source = "<?php\necho sprintf(\"a \\x25s %d\", $x, $y);\necho sprintf(\"%s $x\", $y);\n";
    let compiled = compile(
        "php",
        json!({
            "kind": "encapsed_string",
            "parse": { "grammar": "printfFormat", "has": { "kind": "placeholder", "value": { "eq": 0 }, "capture": "P" } },
        }),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    // The interpolated string's format is unknown: no finding.
    assert_eq!(
        matched_texts(&evaluate_file(&compiled, "php", "f.php", source), source),
        ["\"a \\x25s %d\" P=\\x25s"]
    );
}

#[test]
fn styles_and_grammars_are_their_familys() {
    let format_args = |style: &str| json!({ "pattern": "f($S, $$$A)", "formatArgs": { "style": style, "string": "S", "args": "A", "unused": { "kind": "argument" } } });
    let parse =
        |grammar: &str| json!({ "kind": "string", "parse": { "grammar": grammar, "regex": "." } });
    for (language, rule, expected) in [
        (
            "php",
            parse("percentFormat"),
            "parse/grammar: `percentFormat` parses Python literals; PHP's printf formats are `printfFormat`",
        ),
        (
            "php",
            parse("braceFormat"),
            "parse/grammar: `braceFormat` parses Python literals; PHP has no str.format templates (its printf formats are `printfFormat`)",
        ),
        (
            "php",
            parse("annotation"),
            "parse/grammar: `annotation` parses Python literals; PHP holds no type in a string: its declared types are syntax, and its PHPDoc types are `phpDoc`",
        ),
        (
            "php",
            parse("eslintDirective"),
            "parse/grammar: `eslintDirective` parses TypeScript comments",
        ),
        (
            "typescript",
            parse("phpDoc"),
            "parse/grammar: `phpDoc` parses PHP doc comments",
        ),
        (
            "python",
            parse("printfFormat"),
            "parse/grammar: `printfFormat` parses PHP strings; Python's printf formats are `percentFormat`",
        ),
        (
            "typescript",
            parse("printfFormat"),
            "parse/grammar: `printfFormat` parses PHP strings",
        ),
        (
            "php",
            format_args("percent"),
            "formatArgs/style: `percent` reads Python's `%` formats; PHP's printf-family calls are style `printf`",
        ),
        (
            "php",
            format_args("brace"),
            "formatArgs/style: `brace` reads Python's `str.format` calls; PHP has no such templates (its printf-family calls are style `printf`)",
        ),
        (
            "python",
            format_args("printf"),
            "formatArgs/style: `printf` reads PHP's printf-family calls; Python's printf formats are style `percent`",
        ),
    ] {
        assert_eq!(
            rejection(compile(language, rule)),
            format!("/clauses/0/forbid/ast/rule/{expected}"),
            "{language}"
        );
    }
}

/// `scanf`: with variables passed, PHP 8.5.11 throws where a placeholder numbers a variable past
/// the last passed or a passed variable is assigned by none (`literal::scanf::tests` holds the
/// runtime oracle); with none, the call returns an array. A format through a variable, a
/// concatenation or a constant throws alike (`php -r`: "Different numbers of variable names and
/// field specifiers" on line 8, "Variable is not assigned by any conversion specifiers" on 9
/// and 11).
#[test]
fn scanf_variables_are_missing_or_unused_as_php_throws() {
    let source = "<?php\nsscanf($s, '%d %d', $a);\nsscanf($s, '%d', $a, $b);\nsscanf($s, '%2$s', $a, $b);\nsscanf($s, '%d %d', $a, $b);\nsscanf($s, '%d %d');\n$f = '%d %d';\nsscanf($s, $f, $a);\nsscanf($s, '%d' . ' %d', $a, $b, $c);\nconst F = '%d';\nsscanf($s, F, $a, $b);\n";
    let rule = |side: &str, rule: Value| {
        let mut test = json!({ "style": "scanf", "string": "F", "args": "A" });
        test[side] = rule;
        json!({ "rule": { "any": [{ "pattern": "sscanf($S, $F)" }, { "pattern": "sscanf($S, $F, $$$A)" }], "formatArgs": test } })
    };
    assert_eq!(
        lines(rule("missing", json!({ "kind": "placeholder" })), source),
        BTreeSet::from([2, 8])
    );
    assert_eq!(
        lines(rule("unused", json!({ "kind": "argument" })), source),
        BTreeSet::from([3, 4, 9, 11])
    );
}
