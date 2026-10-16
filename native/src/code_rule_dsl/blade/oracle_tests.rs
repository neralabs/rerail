//! Blade's embedded PHP against illuminate/view's `BladeCompiler` (`fixtures/blade`): the PHP
//! regions the view reads, the definedness of their variables (PHPStan on the compiled template)
//! and the values of their constant expressions (PHP).

use super::view::View;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::scope::php::{Defined, Quiet};
use crate::code_rule_dsl::scope::{Reading, ScopeTable};
use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
use crate::code_rule_dsl::value::Context;
use crate::code_rule_dsl::value::php::{decode, tests::agrees};
use ast_grep_core::{AstGrep, Doc, Node};
use serde_json::Value as Json;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

fn oracle_root() -> PathBuf {
    PathBuf::from(crate::code_rule_dsl::fixtures_dir!()).join("blade/blade")
}

fn expected() -> Json {
    let text = std::fs::read_to_string(oracle_root().join("blade.expected.json")).expect("oracle");
    let expected: Json = serde_json::from_str(&text).expect("oracle JSON");
    assert_eq!(expected["illuminate/view"], "v13.34.0");
    assert_eq!(expected["php"].as_str().map(|v| &v[..3]), Some("8.5"));
    expected
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(oracle_root().join("fixtures").join(name)).expect("fixture")
}

fn range(span: &Json) -> Range<usize> {
    span[0].as_u64().expect("start") as usize..span[1].as_u64().expect("end") as usize
}

/// The template's bytes in `ranges` that are not whitespace.
fn code_bytes(source: &str, ranges: impl IntoIterator<Item = Range<usize>>) -> BTreeSet<usize> {
    ranges
        .into_iter()
        .flatten()
        .filter(|&at| !source.as_bytes()[at].is_ascii_whitespace())
        .collect()
}

/// The template bytes Blade copies into PHP, and those the view copies.
pub(super) fn region_bytes(source: &str, regions: &Json) -> (BTreeSet<usize>, BTreeSet<usize>) {
    let blade = code_bytes(
        source,
        regions
            .as_array()
            .expect("regions")
            .iter()
            .flat_map(|region| {
                region["copied"]
                    .as_array()
                    .expect("copied")
                    .iter()
                    .map(range)
            }),
    );
    let tree = super::forte::tree::parse_tree(source.as_bytes());
    let rerail = View::read(&tree, source, &|| false)
        .expect("an unbounded budget")
        .view
        .map(|view| code_bytes(source, view.copied()))
        .unwrap_or_default();
    (blade, rerail)
}

/// Runs of consecutive bytes, as `start..end`.
pub(super) fn runs(bytes: &BTreeSet<usize>) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    for &at in bytes {
        match out.last_mut() {
            Some(last) if last.end == at => last.end = at + 1,
            _ => out.push(at..at + 1),
        }
    }
    out
}

/// Every PHP region Blade compiles from a fixture is the PHP the view reads there: the template
/// bytes Blade copies into the compiled PHP are exactly those the view copies into its program.
#[test]
fn blade_view_reads_the_regions_blade_compiles() {
    let expected = expected();
    let mut failures = Vec::new();
    let mut regions = 0;
    for (name, record) in expected["fixtures"].as_object().expect("fixtures") {
        let source = fixture(name);
        assert_eq!(record["unlocated"], Json::Array(Vec::new()), "{name}");
        regions += record["regions"].as_array().expect("regions").len();
        let (blade, rerail) = region_bytes(&source, &record["regions"]);
        for (what, missing) in [
            (
                "Blade compiles; the view does not read",
                blade.difference(&rerail),
            ),
            (
                "the view reads; Blade does not compile",
                rerail.difference(&blade),
            ),
        ] {
            for run in runs(&missing.copied().collect()) {
                failures.push(format!("{name}:{run:?}: {what} {:?}", &source[run.clone()]));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} disagreements:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(regions > 150, "only {regions} regions");
}

/// The PHP program a Blade template of `source` embeds (`blade::view`), a document of its own
/// whose nodes stand for template bytes (`origin::InFile`); `None` where it embeds none.
fn view(source: &str) -> Option<AstGrep<TapeDoc>> {
    let grammar = Grammar::new(GrammarDialect::Blade).expect("grammar");
    let image = TapeImage::encode_template(&grammar, source, &|| false).expect("encodes");
    let host = TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false)
        .expect("restores");
    let guests = host.guests().expect("owned");
    guests.first().map(|guest| AstGrep::doc(guest.doc.clone()))
}

fn descendants<'r, D: Doc>(node: &Node<'r, D>) -> Vec<Node<'r, D>> {
    node.dfs().collect()
}

/// One PHPStan report: the variable, its kind, and the template offsets of the reads of it Blade
/// copied onto the reported line (PHPStan reports lines, so one of them).
struct Report {
    line: u64,
    name: String,
    kind: String,
    at: BTreeSet<usize>,
}

fn reports(records: &Json) -> Vec<Report> {
    records
        .as_array()
        .expect("reports")
        .iter()
        .map(|record| Report {
            line: record["line"].as_u64().expect("line"),
            name: record["name"].as_str().expect("name").to_owned(),
            kind: record["kind"].as_str().expect("kind").to_owned(),
            at: record["at"]
                .as_array()
                .expect("at")
                .iter()
                .map(|offset| offset.as_u64().expect("offset") as usize)
                .collect(),
        })
        .collect()
}

/// Rerail's definedness of a template's variable reads against PHPStan's `variable.undefined` on the
/// compiled template, tallied.
#[derive(Default)]
struct Definedness {
    /// Variable reads Rerail records (superglobals aside).
    reads: usize,
    /// Reads Rerail decides: undefined on every path, or defined (or guarded).
    claims: usize,
    undefined: usize,
    /// Each read by PHPStan's verdict (`undefined`, `maybe`, `compact`, or `defined`: not
    /// reported) and Rerail's (`undefined`, `defined`, `unknown`).
    matrix: BTreeMap<(String, &'static str), usize>,
    reports: usize,
    /// PHPStan reports of `undefined` Rerail also finds undefined.
    recalled_undefined: usize,
    /// PHPStan reports of `undefined`.
    reported_undefined: usize,
    /// Reports at no variable the template spells: one Blade or the application's directive
    /// writes (`$__themeViews`, `$_instance`).
    unplaced: usize,
    /// Reports at reads in the arguments of a directive core Blade does not compile, which the
    /// view does not read.
    in_foreign_directives: usize,
    failures: Vec<String>,
}

/// Compares the reads in `source`'s view with PHPStan's `reported` on its compiled template:
/// a read Rerail finds defined on every path (or guarded by `isset`, `empty` or `??`) is not
/// reported; one no path defines is reported undefined. Where Rerail cannot tell (the view's
/// data, a variable some paths define), it claims nothing. Every report placed at the template's
/// variables is at a read Rerail records, unless it is in `foreign`, the arguments of directives
/// core Blade does not compile (where Rerail reads, and so claims, nothing); and not every read it
/// may be at is one Rerail finds defined.
fn compare_definedness(
    name: &str,
    source: &str,
    reported: &[Report],
    foreign: &[Range<usize>],
    tally: &mut Definedness,
) {
    let Some(doc) = view(source) else {
        assert!(reported.is_empty(), "{name}: PHPStan reports; no program");
        return;
    };
    let table = ScopeTable::build_with(
        doc.root(),
        GrammarDialect::Php,
        usize::MAX,
        &|| false,
        Reading::default(),
    )
    .expect("table builds");
    let program = doc.root();
    let flow = table.php.as_deref().expect("a PHP flow");
    let inside = |at: usize| foreign.iter().any(|span| span.contains(&at));
    // Each read's claim: `Some(true)` undefined on every path, `Some(false)` defined or
    // guarded, `None` nothing.
    let mut claimed: BTreeMap<(usize, String), Option<bool>> = BTreeMap::new();
    for node in descendants(&program) {
        if node.kind() != "variable_name" {
            continue;
        }
        let Some(read) = flow.read(node.range().start) else {
            continue;
        };
        // Reports place reads at template offsets: the bytes the read stands for.
        let start = crate::code_rule_dsl::origin::InFile::tape(doc.root().get_doc())
            .node(&node)
            .start;
        if read.superglobal {
            continue;
        }
        let claim = match (read.quiet, read.defined) {
            (Quiet::Silent, _) | (_, Defined::Yes) => Some(false),
            (Quiet::No, Defined::No) => Some(true),
            _ => None,
        };
        tally.reads += 1;
        tally.claims += usize::from(claim.is_some());
        tally.undefined += usize::from(claim == Some(true));
        let variable = read.name.trim_start_matches('$').to_owned();
        let verdict = reported
            .iter()
            .find(|report| report.name == variable && report.at.contains(&start))
            .map_or("defined", |report| report.kind.as_str());
        let rerail = match claim {
            Some(true) => "undefined",
            Some(false) => "defined",
            None => "unknown",
        };
        *tally
            .matrix
            .entry((verdict.to_owned(), rerail))
            .or_default() += 1;
        if claim.is_some() && inside(start) {
            tally.failures.push(format!(
                "{name}:{start}: Rerail decides ${variable} in a directive core Blade does not compile"
            ));
        }
        if claim == Some(true)
            && !reported.iter().any(|report| {
                report.kind == "undefined" && report.name == variable && report.at.contains(&start)
            })
        {
            tally.failures.push(format!(
                "{name}:{start}: Rerail finds ${variable} undefined; PHPStan does not"
            ));
        }
        claimed.insert((start, variable), claim);
    }
    for report in reported {
        tally.reports += 1;
        tally.reported_undefined += usize::from(report.kind == "undefined");
        if report.at.is_empty() {
            tally.unplaced += 1;
            continue;
        }
        let claims: Vec<_> = report
            .at
            .iter()
            .filter_map(|&at| claimed.get(&(at, report.name.clone())))
            .collect();
        if report.kind == "undefined" && claims.contains(&&Some(true)) {
            tally.recalled_undefined += 1;
        }
        if claims.is_empty() {
            if report.at.iter().all(|&at| inside(at)) {
                tally.in_foreign_directives += 1;
            } else {
                tally.failures.push(format!(
                    "{name}: PHPStan reports ${} {} on line {} (at {:?}); Rerail records no read there",
                    report.name, report.kind, report.line, report.at
                ));
            }
        } else if claims.iter().all(|claim| **claim == Some(false)) {
            tally.failures.push(format!(
                "{name}: PHPStan reports ${} {} on line {} (at {:?}); Rerail finds it defined (or guarded)",
                report.name, report.kind, report.line, report.at
            ));
        }
    }
}

/// Every variable read in a fixture's view against PHPStan's `variable.undefined` on the compiled
/// template, the view's data defined outside it (`extract()`), as `compare_definedness` reads them.
#[test]
fn blade_definedness_agrees_with_phpstan() {
    let expected = expected();
    let mut tally = Definedness::default();
    for (name, record) in expected["fixtures"].as_object().expect("fixtures") {
        let source = fixture(name);
        let reported = reports(&record["phpstan"]);
        compare_definedness(name, &source, &reported, &[], &mut tally);
    }
    let Definedness {
        reads,
        claims,
        undefined,
        failures,
        ..
    } = tally;
    assert!(
        failures.is_empty(),
        "{} disagreements:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(reads > 100, "only {reads} reads");
    assert!(
        claims > 0 && undefined > 0,
        "{claims} claims, {undefined} undefined"
    );
}

/// In a template's top level the view's data may define a variable some path leaves unset:
/// unknown, not maybe as PHPStan (which does not model the caller) reads it.
#[test]
fn a_template_variable_some_paths_define_is_unknown() {
    let source = "@if ($c)\n@php\n    $x = 1;\n@endphp\n@endif\n{{ $x }}\n";
    let doc = view(source).expect("an embedded program");
    let table = ScopeTable::build_with(
        doc.root(),
        GrammarDialect::Php,
        usize::MAX,
        &|| false,
        Reading::default(),
    )
    .expect("table builds");
    let flow = table.php.as_deref().expect("a PHP flow");
    let read = descendants(&doc.root())
        .into_iter()
        .filter(|node| node.kind() == "variable_name" && node.text() == "$x")
        .filter_map(|node| flow.read(node.range().start))
        .next_back()
        .expect("the echo's read");
    assert_eq!(read.defined, Defined::Unknown);
}

/// Each region that is an expression without variables, read by the view, has the value PHP gives
/// it as a constant expression (or at run time, where PHP refuses it as one); where PHP evaluates
/// it as a constant expression, Rerail knows its value, unless it reads a named constant or class
/// name, whose value needs PHP name resolution (as fixtures/php/value/php.cases.json records).
#[test]
fn blade_constant_expressions_agree_with_php() {
    let expected = expected();
    let mut failures = Vec::new();
    let (mut cases, mut known) = (0, 0);
    for (name, record) in expected["fixtures"].as_object().expect("fixtures") {
        let source = fixture(name);
        let doc = view(&source).expect("an embedded program");
        let program = doc.root();
        for case in record["values"].as_array().expect("values") {
            cases += 1;
            let span = range(&case["span"]);
            let text = &source[span.clone()];
            let trimmed = span.start + (text.len() - text.trim_start().len())
                ..span.end - (text.len() - text.trim_end().len());
            // The expression a `value` predicate reads there: the innermost, inside what Blade
            // wraps around it (`@unless(c)` is `if (!(c))`).
            let Some(node) = descendants(&program).into_iter().rev().find(|node| {
                crate::code_rule_dsl::origin::InFile::tape(doc.root().get_doc()).node(node)
                    == trimmed
                    && crate::code_rule_dsl::value::php::EXPRESSION_KINDS.contains(&&*node.kind())
            }) else {
                failures.push(format!(
                    "{name}:{trimmed:?}: the view reads no expression there"
                ));
                continue;
            };
            let mut spend = || true;
            let value = decode(&node, &mut Context::new(&mut spend));
            let constant = &case["const"];
            let refused = constant[0].as_str() == Some("error");
            let expected = if refused { &case["eval"] } else { constant };
            if value.is_known() {
                known += 1;
                if value.is_constant()
                    && (expected[0].as_str() == Some("error") || !agrees(&value, expected))
                {
                    failures.push(format!(
                        "{name}: {:?}: Rerail reads {value:?}, PHP gives {expected}",
                        case["src"]
                    ));
                }
            } else if !refused && case["names"] != true {
                failures.push(format!(
                    "{name}: {:?}: PHP gives {constant}, Rerail knows nothing",
                    case["src"]
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
        cases >= 15 && known * 4 >= cases * 3,
        "Rerail knows {known} of {cases} values"
    );
}

/// Region parity over a corpus: the regions `generate.php --corpus` recorded for each template
/// (`BLADE_REGIONS`) against the view's, written to `BLADE_REGIONS_REPORT`.
#[test]
#[ignore = "corpus benchmark: fixtures/blade/blade/generate.php --corpus output"]
fn benchmark_blade_region_parity() {
    let input = std::env::var("BLADE_REGIONS").expect("BLADE_REGIONS names the generator's output");
    let report =
        std::env::var("BLADE_REGIONS_REPORT").expect("BLADE_REGIONS_REPORT names the report");
    let recorded: Json =
        serde_json::from_str(&std::fs::read_to_string(input).expect("regions")).expect("JSON");
    let (mut files, mut regions, mut bytes, mut agreeing) = (0, 0, 0, 0);
    let mut unstable = Vec::new();
    let mut disagreements = Vec::new();
    for (path, record) in recorded["files"].as_object().expect("files") {
        if record.get("regions").is_none() {
            unstable.push(serde_json::json!({ "path": path, "record": record }));
            continue;
        }
        files += 1;
        regions += record["regions"].as_array().expect("regions").len();
        let source = std::fs::read_to_string(path).expect("template");
        let (blade, rerail) = region_bytes(&source, &record["regions"]);
        bytes += blade.len();
        agreeing += blade.intersection(&rerail).count();
        for (what, missing) in [
            ("blade-only", blade.difference(&rerail)),
            ("rerail-only", rerail.difference(&blade)),
        ] {
            for run in runs(&missing.copied().collect()) {
                disagreements.push(serde_json::json!({
                    "path": path, "side": what, "span": [run.start, run.end], "text": &source[run],
                }));
            }
        }
    }
    let out = serde_json::json!({
        "illuminate/view": recorded["illuminate/view"],
        "php": recorded["php"],
        "files": files,
        "regions": regions,
        "bladeCopiedBytes": bytes,
        "agreeingBytes": agreeing,
        "disagreements": disagreements,
        "unstable": unstable,
    });
    std::fs::write(
        &report,
        serde_json::to_string_pretty(&out).expect("JSON") + "\n",
    )
    .expect("report");
    eprintln!(
        "{files} files, {regions} regions, {agreeing}/{bytes} bytes agree, {} disagreements, {} unstable",
        disagreements.len(),
        unstable.len()
    );
}

/// Definedness over a corpus: the PHPStan reports `generate.php --definedness` recorded for each
/// template compiled with its application's directives (`BLADE_DEFINEDNESS`) against the view's
/// reads, as `compare_definedness` reads them, written to `BLADE_DEFINEDNESS_REPORT`. The
/// directives core Blade does not compile, the application's or text, are measured apart: Rerail
/// reads nothing in their arguments, and decides nothing there.
#[test]
#[ignore = "corpus benchmark: fixtures/blade/blade/generate.php --definedness output"]
fn benchmark_blade_definedness_corpus() {
    let input =
        std::env::var("BLADE_DEFINEDNESS").expect("BLADE_DEFINEDNESS names the generator's output");
    let report = std::env::var("BLADE_DEFINEDNESS_REPORT")
        .expect("BLADE_DEFINEDNESS_REPORT names the report");
    let recorded: Json =
        serde_json::from_str(&std::fs::read_to_string(input).expect("records")).expect("JSON");
    let mut tally = Definedness::default();
    let mut files = 0;
    let mut skipped = Vec::new();
    // Directives core Blade does not compile: occurrences by name and whether the application
    // compiles them (or Blade leaves them as text).
    let mut foreign: BTreeMap<(String, bool), usize> = BTreeMap::new();
    let (mut with_application, mut claims_with_application, mut failures_with_application) =
        (0, 0, 0);
    for (path, record) in recorded["files"].as_object().expect("files") {
        let Some(phpstan) = record.get("phpstan") else {
            skipped.push(serde_json::json!({ "path": path, "record": record }));
            continue;
        };
        files += 1;
        let source = std::fs::read_to_string(path).expect("template");
        let mut spans = Vec::new();
        let mut application = false;
        for directive in record["directives"].as_array().expect("directives") {
            if directive["core"] == true {
                continue;
            }
            let custom = directive["custom"] == true;
            application |= custom;
            let name = directive["directive"].as_str().expect("name").to_owned();
            *foreign.entry((name, custom)).or_default() += 1;
            spans.push(range(&directive["span"]));
        }
        let (claims, failures) = (tally.claims, tally.failures.len());
        compare_definedness(path, &source, &reports(phpstan), &spans, &mut tally);
        if application {
            with_application += 1;
            claims_with_application += tally.claims - claims;
            failures_with_application += tally.failures.len() - failures;
        }
    }
    let verdicts = |phpstan: &str| -> usize {
        tally
            .matrix
            .iter()
            .filter(|((verdict, _), _)| verdict == phpstan)
            .map(|(_, count)| count)
            .sum()
    };
    let agreeing = |phpstan: &str, rerail: &str| -> usize {
        tally
            .matrix
            .get(&(phpstan.to_owned(), rerail))
            .copied()
            .unwrap_or_default()
    };
    // PHPStan decides a read undefined (reported `undefined`) or defined (not reported).
    let decided = verdicts("undefined") + verdicts("defined");
    let recalled = agreeing("undefined", "undefined") + agreeing("defined", "defined");
    let ratio = |part: usize, whole: usize| {
        if whole == 0 {
            Json::Null
        } else {
            serde_json::json!((part as f64 / whole as f64 * 10_000.0).round() / 10_000.0)
        }
    };
    let out = serde_json::json!({
        "illuminate/view": recorded["illuminate/view"],
        "php": recorded["php"],
        "phpstan": recorded["phpstan"],
        "directives": recorded["directives"],
        "files": files,
        "skipped": skipped,
        "reads": tally.reads,
        "claims": tally.claims,
        "claimedUndefined": tally.undefined,
        "decidedRate": ratio(tally.claims, tally.reads),
        "matrix": tally.matrix.iter().map(|((phpstan, rerail), count)| serde_json::json!({
            "phpstan": phpstan, "rerail": rerail, "reads": count,
        })).collect::<Vec<_>>(),
        "phpstanDecidedReads": decided,
        "recall": ratio(recalled, decided),
        "phpstanReports": tally.reports,
        "phpstanUndefinedReports": tally.reported_undefined,
        "recalledUndefinedReports": tally.recalled_undefined,
        "unplacedReports": tally.unplaced,
        "foreignDirectives": {
            "occurrences": foreign.iter().map(|((name, custom), count)| serde_json::json!({
                "directive": name, "compiledByApplication": custom, "count": count,
            })).collect::<Vec<_>>(),
            "reportsInArguments": tally.in_foreign_directives,
            "filesWithApplicationDirectives": with_application,
            "claimsInThoseFiles": claims_with_application,
            "contradictionsInThoseFiles": failures_with_application,
        },
        "contradictions": tally.failures,
    });
    std::fs::write(
        &report,
        serde_json::to_string_pretty(&out).expect("JSON") + "\n",
    )
    .expect("report");
    eprintln!(
        "{files} files, {} reads, {} decided, recall {recalled}/{decided}, {} contradictions, {} skipped",
        tally.reads,
        tally.claims,
        tally.failures.len(),
        skipped.len()
    );
    assert!(
        tally.failures.is_empty(),
        "{} contradictions:\n{}",
        tally.failures.len(),
        tally.failures.join("\n")
    );
}
