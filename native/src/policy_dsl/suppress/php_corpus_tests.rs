//! `benchmark_php_suppression_corpus`: the reports PHPStan, Psalm and PHP_CodeSniffer drop because
//! of a corpus file's local suppression annotations, against the reports Rerail's suppression of
//! the same dialect drops.

use super::{PhpcsIndex, PhpcsVersion, PhpstanIndex, PsalmIndex};
use crate::policy_dsl::language::GrammarDialect;
use crate::policy_dsl::lines::{LineTable, Terminators};
use crate::policy_dsl::suppress::psalm::parent_issue_type;
use ast_grep_core::tree_sitter::LanguageExt;
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

const TOOLS: [&str; 3] = ["phpstan", "psalm", "phpcs"];

/// Disagreements whose cause lies with the tool or the comparison, each with its evidence:
/// (corpus, file, line, tool, identifier, evidence). Empty: every disagreement is a failure.
const DIVERGENCES: &[(&str, &str, usize, &str, &str, &str)] = &[];

/// A tool report in the annotated file.
struct Report {
    identifier: String,
    /// One-based.
    line: u64,
    /// The line's start where the tool reports lines only (PHPStan, PHP_CodeSniffer).
    start: usize,
    message: String,
    /// The class PHPStan analysed a trait in (each one apart), or empty.
    context: String,
}

fn reports(value: &Json) -> Vec<Report> {
    value
        .as_array()
        .expect("reports")
        .iter()
        .map(|report| Report {
            identifier: report[0].as_str().expect("identifier").to_owned(),
            line: report[1].as_u64().expect("line"),
            start: report[2].as_u64().expect("start") as usize,
            message: report[3].as_str().expect("message").to_owned(),
            context: report[4].as_str().expect("context").to_owned(),
        })
        .collect()
}

/// A tool's report about its own annotations (unmatched, unparsable), never another
/// annotation's target.
fn about_annotations(tool: &str, identifier: &str) -> bool {
    match tool {
        "phpstan" => identifier.starts_with("ignore."),
        "psalm" => identifier == "UnusedPsalmSuppress",
        _ => false,
    }
}

/// The reports that can be another annotation's target.
fn targets<'r>(tool: &'r str, reports: &'r [Report]) -> impl Iterator<Item = &'r Report> {
    reports
        .iter()
        .filter(move |report| !about_annotations(tool, &report.identifier))
}

/// Which of `all` (the reports with the annotations neutralised) the tool drops when they are
/// in force: those `kept` lacks, by identifier, line, message and context.
fn dropped(tool: &str, all: &[Report], kept: &[Report]) -> Vec<bool> {
    let key = |report: &Report| {
        (
            report.identifier.clone(),
            report.line,
            report.message.clone(),
            report.context.clone(),
        )
    };
    let mut remaining = HashMap::<_, usize>::new();
    for report in kept {
        *remaining.entry(key(report)).or_default() += 1;
    }
    targets(tool, all)
        .map(|report| match remaining.get_mut(&key(report)) {
            Some(count) if *count > 0 => {
                *count -= 1;
                false
            }
            _ => true,
        })
        .collect()
}

#[derive(Default)]
struct Tally {
    annotations: usize,
    annotated_files: usize,
    forms: BTreeMap<String, usize>,
    compared_files: usize,
    annotated_files_with_syntax_errors: usize,
    rerail_indexed_files: usize,
    items: usize,
    reference_positives: usize,
    rerail_positives: usize,
    true_positives: usize,
    false_positives: usize,
    false_negatives: usize,
    effective_annotations: usize,
    unmatched_annotations: usize,
    invalid_annotations: usize,
    /// Reports the tool places in another file (a parent's property, a trait): items Rerail, which
    /// decides positions in the annotated file, leaves undecided.
    elsewhere: usize,
    dropped_elsewhere: usize,
}

fn ratio(numerator: usize, denominator: usize) -> Json {
    if denominator == 0 {
        Json::Null
    } else {
        json!(numerator as f64 / denominator as f64)
    }
}

impl Tally {
    fn add(&mut self, other: &Tally) {
        self.annotations += other.annotations;
        self.annotated_files += other.annotated_files;
        for (form, count) in &other.forms {
            *self.forms.entry(form.clone()).or_default() += count;
        }
        self.compared_files += other.compared_files;
        self.annotated_files_with_syntax_errors += other.annotated_files_with_syntax_errors;
        self.rerail_indexed_files += other.rerail_indexed_files;
        self.items += other.items;
        self.reference_positives += other.reference_positives;
        self.rerail_positives += other.rerail_positives;
        self.true_positives += other.true_positives;
        self.false_positives += other.false_positives;
        self.false_negatives += other.false_negatives;
        self.effective_annotations += other.effective_annotations;
        self.unmatched_annotations += other.unmatched_annotations;
        self.invalid_annotations += other.invalid_annotations;
        self.elsewhere += other.elsewhere;
        self.dropped_elsewhere += other.dropped_elsewhere;
    }

    fn json(&self, contradictions: usize) -> Json {
        // Rerail's suppression decides every report in a file it parses, none placed elsewhere.
        let decided = self.items - self.elsewhere;
        json!({
            "annotations": self.annotations,
            "annotatedFiles": self.annotated_files,
            "annotationForms": self.forms,
            "annotatedFilesWithSyntaxErrors": self.annotated_files_with_syntax_errors,
            "comparedFiles": self.compared_files,
            "rerailIndexedFiles": self.rerail_indexed_files,
            "items": self.items,
            "decided": decided,
            "decidedRate": ratio(decided, self.items),
            "referencePositives": self.reference_positives,
            "rerailPositives": self.rerail_positives,
            "truePositives": self.true_positives,
            "falsePositives": self.false_positives,
            "falseNegatives": self.false_negatives,
            "contradictions": contradictions,
            "agreements": decided - contradictions,
            "precision": ratio(decided - contradictions, decided),
            "suppressionPrecision": ratio(self.true_positives, self.rerail_positives),
            "recall": ratio(self.true_positives, self.reference_positives),
            "effectiveAnnotations": self.effective_annotations,
            "unmatchedAnnotations": self.unmatched_annotations,
            "invalidAnnotations": self.invalid_annotations,
            "reportsElsewhere": self.elsewhere,
            "droppedElsewhere": self.dropped_elsewhere,
        })
    }
}

struct Disagreement {
    corpus: String,
    file: String,
    line: u64,
    tool: &'static str,
    identifier: String,
    rerail: bool,
}

impl Disagreement {
    fn divergence(
        &self,
    ) -> Option<&'static (
        &'static str,
        &'static str,
        usize,
        &'static str,
        &'static str,
        &'static str,
    )> {
        DIVERGENCES
            .iter()
            .find(|(corpus, file, line, tool, identifier, _)| {
                *corpus == self.corpus
                    && *file == self.file
                    && *line as u64 == self.line
                    && *tool == self.tool
                    && *identifier == self.identifier
            })
    }

    fn describe(&self, tool_version: &str) -> (String, String) {
        let (rerail, reference) = if self.rerail {
            ("suppressed", "reported")
        } else {
            ("reported", "suppressed")
        };
        (
            format!("{} {}", self.identifier, rerail),
            format!("{} {} ({tool_version})", self.identifier, reference),
        )
    }
}

/// What Rerail's suppression of `tool` drops from `all`, a file's reports.
fn rerail_suppressed(tool: &str, source: &str, all: &[Report]) -> Vec<bool> {
    let root = GrammarDialect::Php.ast_grep(source);
    let root = root.root();
    let lines = LineTable::new(source, Terminators::Newlines);
    let all = targets(tool, all).collect::<Vec<_>>();
    match tool {
        "phpstan" => {
            let index = PhpstanIndex::read(&root, source, &lines);
            // Each class a trait is analysed in is an analysis of its own, consuming the
            // comments' identifiers afresh: its reports are a kind of error of their own.
            all.iter()
                .map(|report| {
                    index.suppresses(
                        &report.context,
                        std::slice::from_ref(&report.identifier),
                        report.start,
                        &lines,
                    )
                })
                .collect()
        }
        "psalm" => {
            let index = PsalmIndex::read(&root, source);
            all.iter()
                .map(|report| {
                    let issue = &report.identifier;
                    let names = [Some(issue.clone()), parent_issue_type(issue)]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>();
                    index.suppresses(&names, false, report.start)
                })
                .collect()
        }
        _ => {
            let index = PhpcsIndex::read(&root, source, &lines);
            all.iter()
                .map(|report| {
                    index.suppresses(
                        std::slice::from_ref(&report.identifier),
                        PhpcsVersion::Four,
                        report.start,
                        &lines,
                    )
                })
                .collect()
        }
    }
}

/// The reports PHPStan 2.2.16, Psalm 6.19.1 and PHP_CodeSniffer 4.0.4 drop because of each local
/// suppression annotation of pinned PHP corpora (`RERAIL_PHP_SUPPRESSION`, written by
/// `rerail-fixtures/php/suppress/generate_corpus.py`), against those Rerail's `suppress.phpstan`,
/// `suppress.psalm` and `suppress.phpcs` drop from the same reports; the T3 section goes to
/// `RERAIL_PHP_SUPPRESSION_REPORT`. Every file of the corpora is also read for directives: Rerail
/// must find none in a file without an annotation of that tool.
#[test]
#[ignore = "PHPStan, Psalm and PHP_CodeSniffer suppression over PHP corpora"]
fn benchmark_php_suppression_corpus() {
    let input = std::env::var("RERAIL_PHP_SUPPRESSION").expect("RERAIL_PHP_SUPPRESSION");
    let output = std::env::var("RERAIL_PHP_SUPPRESSION_REPORT").expect("report path");
    let recorded: Json =
        serde_json::from_str(&std::fs::read_to_string(input).expect("input")).expect("JSON");
    let versions = TOOLS.map(|tool| recorded[tool].as_str().expect("tool version").to_owned());
    assert!(versions[0].contains("2.2.16"), "{}", versions[0]);
    assert!(versions[1].starts_with("Psalm 6.19.1@"), "{}", versions[1]);
    assert!(versions[2].contains("4.0.4"), "{}", versions[2]);
    let mut rows = Vec::new();
    let mut disagreements = Vec::<Disagreement>::new();
    let mut misread = Vec::new();
    for corpus in recorded["corpora"].as_array().expect("corpora") {
        let name = corpus["name"].as_str().expect("name");
        let root = Path::new(corpus["root"].as_str().expect("root"));
        let files = corpus["files"].as_array().expect("files");
        let mut tallies = TOOLS.map(|_| Tally::default());
        let mut syntax_errors = 0;
        let mut not_utf8 = 0;
        let mut corpus_disagreements = Vec::new();
        for file in files {
            let file = file.as_str().expect("file");
            let bytes = std::fs::read(root.join(file)).expect("corpus file");
            // Rerail reads UTF-8 text only: a file in another encoding is outside what it decides.
            let Ok(source) = String::from_utf8(bytes) else {
                not_utf8 += 1;
                assert!(
                    TOOLS
                        .iter()
                        .all(|tool| corpus["annotations"][tool][file].is_null()),
                    "{name}/{file}: an annotated file that is not UTF-8"
                );
                continue;
            };
            let tree = GrammarDialect::Php.ast_grep(&source);
            let errors = tree
                .root()
                .dfs()
                .any(|node| node.is_error() || node.is_missing());
            for (tally, tool) in tallies.iter_mut().zip(TOOLS) {
                let Some(annotations) = corpus["annotations"][tool][file].as_array() else {
                    continue;
                };
                tally.annotated_files += 1;
                tally.annotations += annotations.len();
                for annotation in annotations {
                    let form = annotation[0].as_str().expect("form");
                    *tally.forms.entry(form.to_owned()).or_default() += 1;
                }
                tally.annotated_files_with_syntax_errors += usize::from(errors);
            }
            if errors {
                // Outside what Rerail decides: not compared.
                syntax_errors += 1;
                continue;
            }
            let lines = LineTable::new(&source, Terminators::Newlines);
            let root_node = tree.root();
            let empty = [
                PhpstanIndex::read(&root_node, &source, &lines).is_empty(),
                PsalmIndex::read(&root_node, &source).is_empty(),
                PhpcsIndex::read(&root_node, &source, &lines).is_empty(),
            ];
            for ((tally, tool), empty) in tallies.iter_mut().zip(TOOLS).zip(empty) {
                if empty {
                    continue;
                }
                tally.rerail_indexed_files += 1;
                if corpus["annotations"][tool][file].is_null() {
                    misread.push(format!(
                        "{name}/{file}: Rerail reads a {tool} directive in a file the tokenizer finds none in"
                    ));
                }
            }
            for (tally, tool) in tallies.iter_mut().zip(TOOLS) {
                let run = &corpus["runs"][tool][file];
                if run.is_null() {
                    continue;
                }
                tally.compared_files += 1;
                let all = reports(&run["all"]);
                let kept = reports(&run["kept"]);
                for report in &kept {
                    let identifier = &report.identifier;
                    if !about_annotations(tool, identifier) {
                        continue;
                    }
                    if identifier.starts_with("ignore.unmatched")
                        || identifier == "UnusedPsalmSuppress"
                    {
                        tally.unmatched_annotations += 1;
                    } else {
                        tally.invalid_annotations += 1;
                    }
                }
                for single in run["single"].as_array().expect("single runs") {
                    let single = reports(single);
                    if dropped(tool, &single, &kept).iter().any(|dropped| *dropped) {
                        tally.effective_annotations += 1;
                    }
                }
                let elsewhere = |key: &str| {
                    run[key]
                        .as_array()
                        .map(|reports| {
                            reports
                                .iter()
                                .filter(|report| {
                                    !about_annotations(
                                        tool,
                                        report[1].as_str().expect("identifier"),
                                    )
                                })
                                .map(|report| report.to_string())
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default()
                };
                let mut kept_elsewhere = elsewhere("keptElsewhere");
                for report in elsewhere("allElsewhere") {
                    tally.items += 1;
                    tally.elsewhere += 1;
                    match kept_elsewhere.iter().position(|kept| *kept == report) {
                        Some(at) => {
                            kept_elsewhere.swap_remove(at);
                        }
                        None => {
                            tally.dropped_elsewhere += 1;
                            tally.reference_positives += 1;
                        }
                    }
                }
                let reference = dropped(tool, &all, &kept);
                let rerail = rerail_suppressed(tool, &source, &all);
                // Rerail decides a report by its identifier and position (a PHPStan or
                // PHP_CodeSniffer report's line), so reports sharing both (and a trait context)
                // are interchangeable: compared as counts. Which of two such PHPStan reports a
                // named identifier consumes follows PHPStan's internal order, which its JSON
                // (ordered by message) does not keep.
                let mut groups = BTreeMap::<(String, u64, usize, String), [usize; 3]>::new();
                for ((report, reference), rerail) in targets(tool, &all).zip(reference).zip(rerail) {
                    let key = (
                        report.identifier.clone(),
                        report.line,
                        report.start,
                        report.context.clone(),
                    );
                    let group = groups.entry(key).or_default();
                    group[0] += 1;
                    group[1] += usize::from(reference);
                    group[2] += usize::from(rerail);
                }
                for ((identifier, line, ..), [items, reference, rerail]) in groups {
                    let agreed = reference.min(rerail);
                    tally.items += items;
                    tally.reference_positives += reference;
                    tally.rerail_positives += rerail;
                    tally.true_positives += agreed;
                    tally.false_positives += rerail - agreed;
                    tally.false_negatives += reference - agreed;
                    for rerail in std::iter::repeat_n(true, rerail - agreed)
                        .chain(std::iter::repeat_n(false, reference - agreed))
                    {
                        corpus_disagreements.push(Disagreement {
                            corpus: name.to_owned(),
                            file: file.to_owned(),
                            line,
                            tool,
                            identifier: identifier.clone(),
                            rerail,
                        });
                    }
                }
            }
        }
        let mut total = Tally::default();
        let mut tool_rows = Vec::new();
        for ((tally, tool), version) in tallies.iter().zip(TOOLS).zip(&versions) {
            let contradictions = corpus_disagreements
                .iter()
                .filter(|disagreement| disagreement.tool == tool)
                .count();
            let mut row = tally.json(contradictions);
            row["tool"] = json!(tool);
            row["version"] = json!(version);
            tool_rows.push(row);
            total.add(tally);
        }
        let mut row = total.json(corpus_disagreements.len());
        let row_object = row.as_object_mut().expect("row");
        row_object.insert("name".into(), json!(name));
        row_object.insert("commit".into(), corpus["commit"].clone());
        row_object.insert("files".into(), json!(files.len()));
        row_object.insert("filesWithSyntaxErrors".into(), json!(syntax_errors));
        row_object.insert("filesNotUtf8".into(), json!(not_utf8));
        row_object.insert(
            "excluded".into(),
            json!(corpus["excluded"].as_array().map_or(0, Vec::len)),
        );
        row_object.insert("unbuilt".into(), json!([]));
        row_object.insert(
            "dependencies".into(),
            json!(
                corpus["dependencies"]["packages"]
                    .as_array()
                    .map_or(0, Vec::len)
            ),
        );
        row_object.insert("tools".into(), json!(tool_rows));
        rows.push(row);
        disagreements.extend(corpus_disagreements);
    }
    let version_of =
        |tool: &str| versions[TOOLS.iter().position(|t| *t == tool).expect("tool")].clone();
    let divergences = disagreements
        .iter()
        .filter_map(|disagreement| {
            let (_, _, _, _, _, evidence) = disagreement.divergence()?;
            let (rerail, reference) = disagreement.describe(&version_of(disagreement.tool));
            Some(json!({
                "corpus": disagreement.corpus, "path": disagreement.file, "line": disagreement.line,
                "tool": disagreement.tool, "rerail": rerail, "reference": reference, "evidence": evidence,
            }))
        })
        .collect::<Vec<_>>();
    let unexplained = disagreements
        .iter()
        .filter(|disagreement| disagreement.divergence().is_none())
        .map(|disagreement| {
            let (rerail, reference) = disagreement.describe(&version_of(disagreement.tool));
            format!(
                "{}/{}:{} {}: Rerail {rerail}, tool {reference}",
                disagreement.corpus, disagreement.file, disagreement.line, disagreement.tool
            )
        })
        .collect::<Vec<_>>();
    let report = json!({
        "capability": "suppression",
        "reference": {
            "tool": "PHPStan / Psalm / PHP_CodeSniffer",
            "version": versions.join("; "),
            "signal": "the reports (line + identifier: PHPStan error identifier, Psalm issue type, PHP_CodeSniffer sniff code) each tool stops emitting because of a local suppression annotation of its own dialect (@phpstan-ignore*, @psalm-suppress, phpcs:/@codingStandardsIgnore*)",
        },
        "method": METHOD,
        "php": recorded["php"],
        "corpora": rows,
        "divergences": divergences,
        "unexplained": unexplained,
        "misread": misread,
    });
    std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).expect("write report");
    assert!(misread.is_empty(), "{misread:#?}");
    assert!(unexplained.is_empty(), "{unexplained:#?}");
}

const METHOD: &str = concat!(
    "Enumerated: PHP's tokenizer inventories every comment token of the corpus files (Blade templates and files `php -l` rejects excluded) holding @phpstan-ignore (-line, -next-line), @psalm-suppress, phpcs: (any case, @ optional) or @codingStandardsIgnore*. ",
    "Each annotated file is analysed by the tool of its dialect from a clone of the corpus with its Composer dependencies installed (so the tool resolves the classes the annotated code uses) and without the project's configuration (no baselines, ignoreErrors or issue handlers): ",
    "PHPStan 2.2.16 at level max with phpstan-deprecation-rules 2.0.5 (symfony's CI extension), a trait file together with the first corpus class using it (PHPStan analyses a trait only in a class's context; that class's own reports are dropped, each context's reports kept apart); ",
    "Psalm 6.19.1 at errorLevel 1 with --find-unused-psalm-suppress; PHP_CodeSniffer 4.0.4 with PSR12. ",
    "Each file runs as written and with every annotation of that dialect neutralised in place (the marker renamed, byte offsets and lines unchanged; PHP_CodeSniffer --ignore-annotations), and for PHPStan and Psalm with each annotation neutralised alone. ",
    "Items are the reports of the neutralised run, less the tools' reports about their own annotations (PHPStan ignore.*, Psalm UnusedPsalmSuppress); a reference positive is an item the as-written run lacks (matched as a multiset by identifier, line, message and trait context): the tool's suppressed set. ",
    "Rerail's claim is the T2 decision: Rerail reads the same file's annotations (PhpstanIndex; PsalmIndex without suppress-all; PhpcsIndex for PHP_CodeSniffer 4) and decides each item by its identifier at its position (PHPStan, PHP_CodeSniffer: the reported line's start; Psalm: the issue's start byte, by its name and Config::getParentIssueType parent; PHPStan named identifiers consumed once per analysis, as PHPStan applies them); a Rerail positive is an item Rerail suppresses. ",
    "Items sharing identifier, position and trait context are interchangeable to Rerail and compared as counts (true positives = min(reference, Rerail) per group): which of two such PHPStan reports a named identifier consumes follows PHPStan's internal order, which its JSON (ordered by message) does not keep. ",
    "Decided = items in the annotated file (Rerail decides every one; a report the tool places in another file, such as a parent class's property, is an item Rerail leaves undecided: reportsElsewhere, droppedElsewhere); decidedRate = decided / items; ",
    "contradictions = decided items where Rerail and the tool disagree (falsePositives + falseNegatives); precision = agreements / decided; suppressionPrecision = truePositives / rerailPositives; recall = truePositives / referencePositives (null where a denominator is 0). ",
    "Files tree-sitter-php parses with errors (filesWithSyntaxErrors) or that are not UTF-8 (filesNotUtf8, which Rerail does not read as text) are counted and not compared. ",
    "Every compared file is also read by all three Rerail indexes: a directive in a file without an annotation of that dialect fails the benchmark (misread), so Rerail is checked to suppress nothing in the unannotated files, where no tool drops anything; rerailIndexedFiles counts the files where Rerail reads a directive. ",
    "effectiveAnnotations counts annotations whose neutralisation alone restores a report; unmatchedAnnotations the tools' unmatched reports in the as-written run (PHPStan ignore.unmatched*, Psalm UnusedPsalmSuppress); invalidAnnotations other PHPStan ignore.* reports.",
);
