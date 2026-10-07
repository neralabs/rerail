//! Blade's embedded PHP against illuminate/view's `BladeCompiler` (`fixtures/blade`): the PHP
//! regions the view reads, the definedness of their variables (PHPStan on the compiled template)
//! and the values of their constant expressions (PHP).

use super::view::View;
use crate::policy_dsl::language::GrammarDialect;
use crate::policy_dsl::scope::php::{Defined, Quiet};
use crate::policy_dsl::scope::{Reading, ScopeTable};
use crate::policy_dsl::tape::{Grammar, TapeDoc, TapeImage};
use crate::policy_dsl::value::Context;
use crate::policy_dsl::value::php::{decode, tests::agrees};
use ast_grep_core::tree_sitter::StrDoc;
use ast_grep_core::{AstGrep, Doc, Node};
use serde_json::Value as Json;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

fn oracle_root() -> PathBuf {
    PathBuf::from(crate::policy_dsl::fixtures_dir!()).join("blade/view")
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
    let native = StrDoc::new(source, GrammarDialect::Blade);
    let rerail = View::build(native.tree.root_node(), source)
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

/// A Blade tape of `source`: the template and its embedded program.
fn tape(source: &str) -> AstGrep<TapeDoc> {
    let grammar = Grammar::new(GrammarDialect::Blade).expect("grammar");
    let native = StrDoc::new(source, GrammarDialect::Blade);
    let image =
        TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).expect("encodes");
    AstGrep::doc(
        TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false)
            .expect("restores"),
    )
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

/// Every variable read in a fixture's view against PHPStan's `variable.undefined` on the compiled
/// template, the view's data defined outside it (`extract()`): a read Rerail finds defined on every
/// path (or guarded by `isset`, `empty` or `??`) is not reported; one no path defines is reported
/// undefined. Where Rerail cannot tell (the view's data, a variable some paths define), it claims
/// nothing. Every report is at a read Rerail records, and not every read it may be at is one Rerail
/// finds defined.
#[test]
fn blade_definedness_agrees_with_phpstan() {
    let expected = expected();
    let mut failures = Vec::new();
    let (mut claims, mut reads, mut undefined) = (0, 0, 0);
    for (name, record) in expected["fixtures"].as_object().expect("fixtures") {
        let source = fixture(name);
        let reported = reports(&record["phpstan"]);
        let doc = tape(&source);
        let table = ScopeTable::build_with(
            doc.root(),
            GrammarDialect::Blade,
            usize::MAX,
            &|| false,
            Reading::default(),
        )
        .expect("table builds");
        let flow = table.php.as_deref().expect("a PHP flow");
        // Each read's claim: `Some(true)` undefined on every path, `Some(false)` defined or
        // guarded, `None` nothing.
        let mut claimed: BTreeMap<(usize, String), Option<bool>> = BTreeMap::new();
        let program = super::program(&doc.root()).expect("an embedded program");
        for node in descendants(&program) {
            if node.kind() != "variable_name" {
                continue;
            }
            let start = node.range().start;
            let Some(read) = flow.read(start) else {
                continue;
            };
            if read.superglobal {
                continue;
            }
            let claim = match (read.quiet, read.defined) {
                (Quiet::Silent, _) | (_, Defined::Yes) => Some(false),
                (Quiet::No, Defined::No) => Some(true),
                _ => None,
            };
            reads += 1;
            claims += usize::from(claim.is_some());
            undefined += usize::from(claim == Some(true));
            let variable = read.name.trim_start_matches('$').to_owned();
            if claim == Some(true)
                && !reported.iter().any(|report| {
                    report.kind == "undefined"
                        && report.name == variable
                        && report.at.contains(&start)
                })
            {
                failures.push(format!(
                    "{name}:{start}: Rerail finds ${variable} undefined; PHPStan does not"
                ));
            }
            claimed.insert((start, variable), claim);
        }
        for report in &reported {
            let claims: Vec<_> = report
                .at
                .iter()
                .filter_map(|&at| claimed.get(&(at, report.name.clone())))
                .collect();
            if claims.is_empty() {
                failures.push(format!(
                    "{name}: PHPStan reports ${} {} on line {} (at {:?}); Rerail records no read there",
                    report.name, report.kind, report.line, report.at
                ));
            } else if claims.iter().all(|claim| **claim == Some(false)) {
                failures.push(format!(
                    "{name}: PHPStan reports ${} {} on line {} (at {:?}); Rerail finds it defined (or guarded)",
                    report.name, report.kind, report.line, report.at
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
    let doc = tape(source);
    let table = ScopeTable::build_with(
        doc.root(),
        GrammarDialect::Blade,
        usize::MAX,
        &|| false,
        Reading::default(),
    )
    .expect("table builds");
    let flow = table.php.as_deref().expect("a PHP flow");
    let program = super::program(&doc.root()).expect("an embedded program");
    let read = descendants(&program)
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
        let doc = tape(&source);
        let program = super::program(&doc.root()).expect("an embedded program");
        for case in record["values"].as_array().expect("values") {
            cases += 1;
            let span = range(&case["span"]);
            let text = &source[span.clone()];
            let trimmed = span.start + (text.len() - text.trim_start().len())
                ..span.end - (text.len() - text.trim_end().len());
            // The expression a `value` predicate reads there: the innermost, inside what Blade
            // wraps around it (`@unless(c)` is `if (!(c))`).
            let Some(node) = descendants(&program).into_iter().rev().find(|node| {
                node.range() == trimmed
                    && crate::policy_dsl::value::php::EXPRESSION_KINDS.contains(&&*node.kind())
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
#[ignore = "corpus benchmark: fixtures/blade/view/generate.php --corpus output"]
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
