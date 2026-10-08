use super::{NameKind, PhpNames, Reference, Resolved};
use crate::code_rule_dsl::CompiledCodeRuleDsl;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::oracle;
use crate::code_rule_dsl::scope::BuildError;
use crate::code_rule_dsl::test_support::*;
use ast_grep_language::LanguageExt;
use napi::Result;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

/// The file's names table, and the text of any syntax error tree-sitter recovered from.
fn build(source: &str) -> (PhpNames, Vec<String>) {
    let root = GrammarDialect::Php.ast_grep(source);
    let errors = root
        .root()
        .dfs()
        .filter(|node| node.is_error() || node.is_missing())
        .map(|node| format!("{:?} {:?}", node.range(), node.text()))
        .collect();
    let names = PhpNames::build(root.root(), usize::MAX, &|| false).expect("a names table");
    (names, errors)
}

fn kind(name: &str) -> NameKind {
    match name {
        "class" => NameKind::Class,
        "function" => NameKind::Function,
        "constant" => NameKind::Constant,
        other => panic!("kind {other}"),
    }
}

/// What nikic/PHP-Parser's NameResolver resolves a recorded name to, as Rerail spells a
/// resolution, and the bucket it falls in: an unqualified function or constant in a namespace
/// (`namespaced`) is the namespaced name where the file declares it, the global one where PHP
/// defines it, and either otherwise.
fn expected(record: &Value) -> (NameKind, Resolved, &'static str) {
    let text = record["text"].as_str().expect("text");
    let kind = kind(record["kind"].as_str().expect("kind"));
    let (resolved, bucket) = if record.get("special").is_some() {
        (Resolved::Special, "special")
    } else if let Some(resolved) = record["resolved"].as_str() {
        (Resolved::Name(resolved.into()), "resolved")
    } else if record["declared"] == true {
        (
            Resolved::Name(record["namespaced"].as_str().expect("namespaced").into()),
            "fallback: declared",
        )
    } else if record["builtin"] == true {
        (Resolved::Name(text.into()), "fallback: builtin")
    } else {
        (
            Resolved::Either {
                namespaced: record["namespaced"].as_str().expect("namespaced").into(),
                global: text.into(),
            },
            "fallback: unknown",
        )
    };
    (kind, resolved, bucket)
}

/// A recorded name's or use item's byte range, checked against the source.
fn recorded_range(source: &str, record: &Value) -> Range<usize> {
    let start = record["start"].as_u64().expect("start") as usize;
    let end = record["end"].as_u64().expect("end") as usize;
    let text = record["text"].as_str().expect("text");
    assert_eq!(
        source.get(start..end),
        Some(text),
        "oracle range {start}..{end}"
    );
    start..end
}

/// How Rerail reads one name or use item the reference records.
#[derive(Debug)]
enum Verdict {
    /// Rerail resolves it as the reference does.
    Agrees,
    /// Rerail reads no name there, or one it leaves unresolved (`Resolved::Unknown`, or a kind an
    /// error region hides).
    Undecided(String),
    /// Rerail resolves it otherwise.
    Contradicts(String),
}

/// Whether a reference is a claim: a kind and a resolution.
fn decides(reference: &Reference) -> bool {
    reference.kind.is_some() && reference.resolved != Resolved::Unknown
}

/// Rerail's reading of the name `record` (a NameResolver name) holds, and its bucket.
fn compare_name(names: &PhpNames, source: &str, record: &Value) -> (&'static str, Verdict) {
    let range = recorded_range(source, record);
    let (expected_kind, expected, bucket) = expected(record);
    let at = format!("{range:?} `{}`", &source[range.clone()]);
    let verdict = match names.reference(&range) {
        // `self::`, `static::` and `parent::` are scopes in the grammar, not names; they resolve
        // to nothing either way.
        None if expected == Resolved::Special => Verdict::Agrees,
        None => Verdict::Undecided(format!("{at}: not a reference")),
        Some(reference)
            if reference.kind == Some(expected_kind) && reference.resolved == expected =>
        {
            Verdict::Agrees
        }
        Some(reference) => {
            let message = format!(
                "{at}: {:?} {:?}, expected {expected_kind:?} {expected:?}",
                reference.kind, reference.resolved
            );
            if decides(reference) {
                Verdict::Contradicts(message)
            } else {
                Verdict::Undecided(message)
            }
        }
    };
    (bucket, verdict)
}

/// Rerail's reading of the name the use item `record` writes.
fn compare_import(names: &PhpNames, source: &str, record: &Value) -> Verdict {
    let range = recorded_range(source, record);
    let expected = (
        kind(record["kind"].as_str().expect("kind")),
        record["imported"].as_str().expect("imported"),
    );
    let at = format!("{range:?} `{}`", &source[range.clone()]);
    match names.import(&range) {
        Some(import) if import == expected => Verdict::Agrees,
        Some(import) => Verdict::Contradicts(format!("{at}: {import:?}, expected {expected:?}")),
        None => Verdict::Undecided(format!("{at}: no use item, expected {expected:?}")),
    }
}

/// Rerail's references at no range of `records`.
fn unrecorded_references<'n>(
    names: &'n PhpNames,
    records: &[Value],
) -> Vec<(Range<usize>, &'n Reference)> {
    let seen = recorded_ranges(records);
    let mut extra = names
        .references()
        .filter(|(range, _)| !seen.contains(&(range.start, range.end)))
        .collect::<Vec<_>>();
    extra.sort_by_key(|(range, _)| range.start);
    extra
}

/// Rerail's use items at no range of `records`.
fn unrecorded_imports<'n>(
    names: &'n PhpNames,
    records: &[Value],
) -> Vec<(Range<usize>, NameKind, &'n str)> {
    let seen = recorded_ranges(records);
    let mut extra = names
        .imports()
        .filter(|(range, _, _)| !seen.contains(&(range.start, range.end)))
        .collect::<Vec<_>>();
    extra.sort_by_key(|(range, _, _)| range.start);
    extra
}

fn recorded_ranges(records: &[Value]) -> HashSet<(usize, usize)> {
    records
        .iter()
        .map(|record| {
            (
                record["start"].as_u64().expect("start") as usize,
                record["end"].as_u64().expect("end") as usize,
            )
        })
        .collect()
}

fn array(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

/// Every name nikic/PHP-Parser's NameResolver resolves in the fixtures resolves alike here, and
/// every reference read here is one it resolves.
#[test]
fn names_resolve_as_php_parser_name_resolver() {
    let oracle: Value =
        serde_json::from_str(oracle!("php/scope/php/names.expected.json")).expect("oracle JSON");
    assert_eq!(oracle["php"], "8.5.11");
    assert_eq!(oracle["phpParser"], "v5.9.0");
    let directory = Path::new(env!("RERAIL_FIXTURES_DIR")).join("php/scope/php/names");
    let mut failures = Vec::new();
    let mut counts = BTreeMap::<&str, usize>::new();
    for (file, records) in oracle["files"].as_object().expect("files") {
        let source = std::fs::read_to_string(directory.join(file)).expect("fixture");
        let (names, errors) = build(&source);
        assert!(errors.is_empty(), "{file} has syntax errors: {errors:?}");
        for record in array(records) {
            let (bucket, verdict) = compare_name(&names, &source, record);
            *counts.entry(bucket).or_default() += 1;
            match verdict {
                Verdict::Agrees => {}
                Verdict::Undecided(message) | Verdict::Contradicts(message) => {
                    failures.push(format!("{file} {message}"));
                }
            }
        }
        for (range, reference) in unrecorded_references(&names, array(records)) {
            failures.push(format!(
                "{file} {range:?} `{}`: {reference:?} is no name NameResolver resolves",
                &source[range.clone()]
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    let total: usize = counts.values().sum();
    assert!(total >= 300, "{counts:?}");
    for bucket in [
        "resolved",
        "special",
        "fallback: declared",
        "fallback: builtin",
        "fallback: unknown",
    ] {
        assert!(
            counts.get(bucket).is_some_and(|&count| count > 0),
            "{counts:?}"
        );
    }
}

/// Every name a `use` item writes imports what nikic/PHP-Parser's UseItem says (a group's prefix
/// joined with the item's name), of the item's kind; and every import site read here is one.
#[test]
fn use_items_import_what_php_parser_reads() {
    let oracle: Value =
        serde_json::from_str(oracle!("php/scope/php/names.expected.json")).expect("oracle JSON");
    let directory = Path::new(env!("RERAIL_FIXTURES_DIR")).join("php/scope/php/names");
    let mut failures = Vec::new();
    let mut total = 0;
    for (file, records) in oracle["imports"].as_object().expect("imports") {
        let source = std::fs::read_to_string(directory.join(file)).expect("fixture");
        let (names, _) = build(&source);
        for record in array(records) {
            total += 1;
            match compare_import(&names, &source, record) {
                Verdict::Agrees => {}
                Verdict::Undecided(message) | Verdict::Contradicts(message) => {
                    failures.push(format!("{file} {message}"));
                }
            }
        }
        for (range, kind, name) in unrecorded_imports(&names, array(records)) {
            failures.push(format!(
                "{file} {range:?}: {kind:?} {name} is no use item PHP-Parser reads"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(total >= 40, "{total} use items");
}

/// Where nikic/PHP-Parser's reading is not Rerail's and Rerail is not wrong, by corpus, path
/// relative to the corpus, byte offset of the name, and the evidence.
const NAME_DIVERGENCES: &[(&str, &str, usize, &str)] = &[];

/// One kind of comparison's counts over a corpus.
#[derive(Default)]
struct Counts {
    /// What the reference records.
    items: usize,
    /// Of those, what Rerail resolves (agreeing or not).
    decided: usize,
    agreed: usize,
    /// Rerail claims at no range the reference records.
    rerail_only: usize,
    /// Rerail references at no recorded range that resolve nothing (no claim).
    rerail_only_undecided: usize,
    contradictions: usize,
    evidenced: usize,
    by_bucket: BTreeMap<&'static str, [usize; 3]>,
}

impl Counts {
    fn add(&mut self, other: &Counts) {
        self.items += other.items;
        self.decided += other.decided;
        self.agreed += other.agreed;
        self.rerail_only += other.rerail_only;
        self.rerail_only_undecided += other.rerail_only_undecided;
        self.contradictions += other.contradictions;
        self.evidenced += other.evidenced;
    }

    fn summary(&self) -> Value {
        let claims = self.decided + self.rerail_only;
        let ratio = |part: usize, whole: usize| {
            if whole == 0 {
                1.0
            } else {
                part as f64 / whole as f64
            }
        };
        let mut summary = json!({
            "items": self.items,
            "decided": self.decided,
            "decidedRate": ratio(self.decided, self.items),
            "referencePositives": self.items,
            "rerailPositives": claims,
            "truePositives": self.agreed,
            "contradictions": self.contradictions,
            "evidencedContradictions": self.evidenced,
            "precision": ratio(self.agreed, claims),
            "recall": ratio(self.agreed, self.items),
            "undecided": self.items - self.decided,
            "rerailOnly": self.rerail_only,
            "rerailOnlyUndecided": self.rerail_only_undecided,
        });
        if !self.by_bucket.is_empty() {
            summary["byResolution"] = self
                .by_bucket
                .iter()
                .map(|(bucket, [items, decided, agreed])| {
                    (
                        bucket.to_string(),
                        json!({ "items": items, "decided": decided, "agreed": agreed }),
                    )
                })
                .collect::<serde_json::Map<_, _>>()
                .into();
        }
        summary
    }
}

/// nikic/PHP-Parser's NameResolver over whole corpora (`RERAIL_PHP_NAMES`, written by
/// `rerail-fixtures/php/scope/php/generate_names.php --corpus`), against Rerail's names table of
/// every file, compared as `names_resolve_as_php_parser_name_resolver` and
/// `use_items_import_what_php_parser_reads` compare the fixtures; the report section goes to
/// `RERAIL_PHP_NAMES_REPORT`. Rerail may leave a name undecided; where it resolves one it must be
/// PHP-Parser's resolution, and every name it resolves must be one PHP-Parser resolves,
/// `NAME_DIVERGENCES` aside.
#[test]
#[ignore = "nikic/PHP-Parser name resolution over full PHP corpora"]
fn benchmark_php_names_corpus() {
    let input = std::env::var("RERAIL_PHP_NAMES").expect("RERAIL_PHP_NAMES");
    let output = std::env::var("RERAIL_PHP_NAMES_REPORT").expect("RERAIL_PHP_NAMES_REPORT");
    let corpora: Value =
        serde_json::from_str(&std::fs::read_to_string(input).expect("corpora")).expect("JSON");
    assert_eq!(corpora["php"], "8.5.11");
    assert_eq!(corpora["phpParser"], "v5.9.0");
    let mut sections = Vec::new();
    let mut divergences = Vec::new();
    let mut failures = Vec::new();
    let mut matched = HashSet::new();
    for corpus in array(&corpora["corpora"]) {
        let corpus_name = corpus["name"].as_str().expect("name");
        let roots = array(&corpus["paths"])
            .iter()
            .map(|path| PathBuf::from(path.as_str().expect("path")))
            .collect::<Vec<_>>();
        // Paths relative to the corpus checkout: the parent of its first path.
        let base = roots[0].parent().expect("corpus root").to_path_buf();
        let mut excluded = array(&corpus["excluded"])
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<HashSet<_>>();
        excluded.extend(
            array(&corpus["parserErrors"])
                .iter()
                .filter_map(|error| error["path"].as_str())
                .map(str::to_owned),
        );
        let mut files = Vec::new();
        for root in &roots {
            php_files(root, &mut files);
        }
        files.sort();
        files.retain(|file| !excluded.contains(file.to_string_lossy().as_ref()));
        let (mut unparsed, mut unreadable) = (0usize, Vec::new());
        let mut unbuilt = Vec::new();
        let (mut names_counts, mut import_counts) = (Counts::default(), Counts::default());
        let mut undecided_examples = Vec::new();
        for file in &files {
            let key = file.to_string_lossy();
            let relative = file
                .strip_prefix(&base)
                .unwrap_or(file)
                .to_string_lossy()
                .into_owned();
            let records = &corpus["files"][key.as_ref()];
            assert!(records.is_array(), "{key}: not recorded");
            let Ok(source) = std::fs::read_to_string(file) else {
                unreadable.push(relative);
                continue;
            };
            let root = GrammarDialect::Php.ast_grep(&source);
            // A file the grammar reads with errors is outside what the names table decides, as
            // tape parity reports it.
            if root
                .root()
                .dfs()
                .any(|node| node.is_error() || node.is_missing())
            {
                unparsed += 1;
                continue;
            }
            let names = match PhpNames::build(root.root(), usize::MAX, &|| false) {
                Ok(names) => names,
                Err(error) => {
                    unbuilt.push(format!("{relative}: {error:?}"));
                    continue;
                }
            };
            let line = |offset: usize| source[..offset].matches('\n').count() + 1;
            let mut contradiction = |offset: usize, message: String, counts: &mut Counts| {
                counts.contradictions += 1;
                match NAME_DIVERGENCES.iter().find(|(name, path, start, _)| {
                    *name == corpus_name && *path == relative && *start == offset
                }) {
                    Some(divergence) => {
                        counts.evidenced += 1;
                        matched.insert((divergence.0, divergence.1, divergence.2));
                        divergences.push(json!({
                            "corpus": corpus_name,
                            "path": relative,
                            "line": line(offset),
                            "rerail": message,
                            "reference": "see evidence",
                            "evidence": divergence.3,
                        }));
                    }
                    None => failures.push(format!(
                        "{corpus_name} {relative}:{} {message}",
                        line(offset)
                    )),
                }
            };
            let name_records = array(records);
            for record in name_records {
                let (bucket, verdict) = compare_name(&names, &source, record);
                let counts = names_counts.by_bucket.entry(bucket).or_default();
                counts[0] += 1;
                names_counts.items += 1;
                match verdict {
                    Verdict::Agrees => {
                        counts[1] += 1;
                        counts[2] += 1;
                        names_counts.decided += 1;
                        names_counts.agreed += 1;
                    }
                    Verdict::Contradicts(message) => {
                        counts[1] += 1;
                        names_counts.decided += 1;
                        let offset = record["start"].as_u64().expect("start") as usize;
                        contradiction(offset, message, &mut names_counts);
                    }
                    Verdict::Undecided(message) => {
                        if undecided_examples.len() < 40 {
                            undecided_examples.push(format!(
                                "{relative}:{} {message}",
                                line(record["start"].as_u64().expect("start") as usize)
                            ));
                        }
                    }
                }
            }
            for (range, reference) in unrecorded_references(&names, name_records) {
                if decides(reference) {
                    names_counts.rerail_only += 1;
                    let message = format!(
                        "{range:?} `{}`: {reference:?} is no name NameResolver resolves",
                        &source[range.clone()]
                    );
                    contradiction(range.start, message, &mut names_counts);
                } else {
                    names_counts.rerail_only_undecided += 1;
                }
            }
            let import_records = array(&corpus["imports"][key.as_ref()]);
            for record in import_records {
                import_counts.items += 1;
                match compare_import(&names, &source, record) {
                    Verdict::Agrees => {
                        import_counts.decided += 1;
                        import_counts.agreed += 1;
                    }
                    Verdict::Contradicts(message) => {
                        import_counts.decided += 1;
                        let offset = record["start"].as_u64().expect("start") as usize;
                        contradiction(offset, message, &mut import_counts);
                    }
                    Verdict::Undecided(message) => {
                        if undecided_examples.len() < 40 {
                            undecided_examples.push(format!("{relative} {message}"));
                        }
                    }
                }
            }
            for (range, kind, name) in unrecorded_imports(&names, import_records) {
                import_counts.rerail_only += 1;
                let message = format!("{range:?}: {kind:?} {name} is no use item PHP-Parser reads");
                contradiction(range.start, message, &mut import_counts);
            }
        }
        let mut total = Counts::default();
        total.add(&names_counts);
        total.add(&import_counts);
        let mut section = json!({
            "name": corpus_name,
            "files": files.len(),
            "filesWithSyntaxErrors": unparsed,
            "unbuilt": unbuilt,
            "excluded": excluded.len(),
            "unreadable": unreadable,
        });
        let fields = section.as_object_mut().expect("object");
        if let Value::Object(summary) = total.summary() {
            fields.extend(summary);
        }
        fields.insert("names".into(), names_counts.summary());
        fields.insert("useItems".into(), import_counts.summary());
        fields.insert("undecidedExamples".into(), undecided_examples.into());
        sections.push(section);
    }
    let report = json!({
        "capability": "names",
        "reference": {
            "tool": "nikic/php-parser",
            "version": corpora["phpParser"],
            "php": corpora["php"],
            "signal": "NameResolver (replaceNodes: false) resolvedName / namespacedName of every Name node, with the file's own declarations and `php -n` builtins settling the unqualified function and constant fallback; UseItem names with their imported fully qualified name",
        },
        "method": "Every *.php file of each corpus's paths (Blade templates and files `php -n -l` rejects excluded; files tree-sitter-php reads with errors counted as filesWithSyntaxErrors, and files that are not UTF-8, which Rerail does not read as source, listed as unreadable; neither compared) is read by PHP-Parser 5.9.0 under PHP 8.5.11 (rerail-fixtures/php/scope/php/generate_names.php --corpus) and by Rerail's names table (PhpNames::build). Items are the reference's names (each Name NameResolver resolves) and use items; `names` and `useItems` split the totals, `names.byResolution` by how the reference resolves. A name is decided where Rerail's reference at the same byte range has a kind and a resolution (Resolved::Name, Special or Either; self/static/parent before `::`, which the grammar reads as scopes and Rerail does not name, agree with Special as in the T2 fixture test); it agrees where kind and resolution equal the reference's as the fixture test maps them (resolvedName; special; an unqualified namespaced function or constant is the namespaced name where the file declares it, the global one where `php -n` defines it, Either otherwise). A use item is decided where Rerail reads an import at its range, agreeing where kind and imported name are equal. A Rerail reference or import at a range the reference does not record is a Rerail-only claim (rerailOnly). contradictions = decided items that disagree + Rerail-only claims, each failing the benchmark unless listed in divergences with evidence. decidedRate = decided / items; precision = truePositives / (truePositives + contradictions) = truePositives / rerailPositives (every Rerail claim); recall = truePositives / referencePositives (every item).",
        "corpora": sections,
        "divergences": divergences,
    });
    std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).expect("write report");
    let stale = NAME_DIVERGENCES
        .iter()
        .filter(|(name, path, start, _)| !matched.contains(&(*name, *path, *start)))
        .collect::<Vec<_>>();
    assert!(stale.is_empty(), "divergences no longer seen: {stale:?}");
    assert!(
        failures.is_empty(),
        "{} contradictions:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

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

#[test]
fn a_build_stops_at_its_node_budget() {
    let source = "<?php\nnamespace A;\nexec();\n";
    let root = GrammarDialect::Php.ast_grep(source);
    assert!(PhpNames::build(root.root(), 3, &|| false).is_err());
    let names = PhpNames::build(root.root(), usize::MAX, &|| false).expect("a table");
    assert!(names.cost > 3);
}

#[test]
fn a_braced_namespace_body_is_charged_to_the_budget() {
    let source = "<?php\nnamespace A {\nexec();\n}\n";
    let root = GrammarDialect::Php.ast_grep(source);
    assert_eq!(
        PhpNames::build(root.root(), 3, &|| false).err(),
        Some(BuildError::Budget)
    );
    let names = PhpNames::build(root.root(), usize::MAX, &|| false).expect("a table");
    assert!(names.cost > 3);
}

#[test]
fn a_braced_namespace_body_reads_the_deadline() {
    let source = format!("<?php\nnamespace A {{\n{}}}\n", "exec();\n".repeat(600));
    let root = GrammarDialect::Php.ast_grep(&source);
    assert_eq!(
        PhpNames::build(root.root(), usize::MAX, &|| true).err(),
        Some(BuildError::Deadline)
    );
}

fn php_rule(rule: Value) -> Result<CompiledCodeRuleDsl> {
    compile_document_with(
        clause_document(json!({
            "id": "php-names",
            "forbid": { "ast": { "language": "php", "rule": rule } },
            "violation": violation(),
        })),
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
}

fn matches(rule: &Value, source: &str) -> Vec<String> {
    let compiled = php_rule(rule.clone()).unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, "php", "a.php", source);
    assert_eq!(result["failures"], json!([]), "{result}");
    matched_texts(&result, source)
}

fn calls_resolving_to(import: Value) -> Value {
    json!({ "kind": "function_call_expression", "has": { "field": "function", "resolves": { "import": import } } })
}

#[test]
fn a_call_matches_where_its_callee_resolves_to_a_listed_function() {
    let exec = calls_resolving_to(json!(["exec"]));
    assert_eq!(
        matches(
            &exec,
            "<?php\nexec('a');\n\\exec('b');\nEXEC('c');\nstrlen('d');\n"
        ),
        ["exec('a')", "\\exec('b')", "EXEC('c')"]
    );
    // In a namespace an unqualified call falls back to the builtin, unless the file declares the
    // namespaced function.
    assert_eq!(
        matches(
            &exec,
            "<?php\nnamespace App;\nexec('a');\nApp\\exec('b');\n"
        ),
        ["exec('a')"]
    );
    assert_eq!(
        matches(
            &exec,
            "<?php\nnamespace App;\nfunction exec() {}\nexec('a');\n\\exec('b');\n"
        ),
        ["\\exec('b')"]
    );
    // An alias names what it imports.
    let aliased = "<?php\nnamespace App;\nuse function Tools\\run as exec;\nexec('a');\n";
    assert_eq!(matches(&exec, aliased), Vec::<String>::new());
    assert_eq!(
        matches(&calls_resolving_to(json!(["\\Tools\\RUN"])), aliased),
        ["exec('a')"]
    );
}

#[test]
fn a_fallback_php_resolves_at_run_time_is_unknown_where_only_one_candidate_is_listed() {
    let source = "<?php\nnamespace App;\nhelper();\nother();\n";
    // `helper()` is `App\helper` if another file declares it, else `helper`.
    let listed = calls_resolving_to(json!(["helper"]));
    assert_eq!(matches(&listed, source), Vec::<String>::new());
    let negated = json!({ "kind": "function_call_expression", "has": { "field": "function", "not": { "resolves": { "import": ["helper"] } } } });
    assert_eq!(matches(&negated, source), ["other()"]);
    // Both candidates listed, it holds either way.
    let both = calls_resolving_to(json!(["helper", "App\\helper"]));
    assert_eq!(matches(&both, source), ["helper()"]);
    assert_eq!(
        matches(&calls_resolving_to(json!(true)), source),
        ["helper()", "other()"]
    );
}

#[test]
fn classes_and_namespaces_compare_case_insensitively_and_constant_names_do_not() {
    let classes = json!({ "kind": "object_creation_expression", "has": { "resolves": { "import": ["App\\Models"], "prefix": true } } });
    let source = "<?php\nnamespace App;\nuse App\\Models as M;\nnew models\\User();\nnew M\\Post();\nnew \\Apple\\Pie();\nnew self();\n";
    assert_eq!(
        matches(&classes, source),
        ["new models\\User()", "new M\\Post()"]
    );
    let constants = json!({ "resolves": { "import": ["PHP_EOL", "app\\LIMIT"] } });
    let source = "<?php\nnamespace App;\nconst LIMIT = 1;\necho PHP_EOL, php_eol, LIMIT, \\APP\\LIMIT, \\App\\limit;\n";
    assert_eq!(
        matches(&constants, source),
        ["PHP_EOL", "LIMIT", "\\APP\\LIMIT"]
    );
}

#[test]
fn imported_matches_the_names_use_clauses_import() {
    let source = "<?php\nnamespace App;\nuse Illuminate\\Support\\Facades\\DB, Illuminate\\Support\\Str as S;\nuse function Illuminate\\Support\\{collect, value as v};\nuse const App\\Limits\\MAX;\nnew DB();\n";
    let imported = |listed: Value, prefix: bool| {
        let mut resolves = json!({ "imported": listed });
        if prefix {
            resolves["prefix"] = json!(true);
        }
        matches(&json!({ "resolves": resolves }), source)
    };
    // The clause's name, not its alias; a function and a class compared case-insensitively.
    assert_eq!(
        imported(
            json!([
                "illuminate\\support\\facades\\db",
                "\\Illuminate\\Support\\Str"
            ]),
            false
        ),
        [
            "Illuminate\\Support\\Facades\\DB",
            "Illuminate\\Support\\Str"
        ]
    );
    assert_eq!(
        imported(json!(["Illuminate\\Support\\collect"]), false),
        ["collect"]
    );
    assert_eq!(
        imported(json!(["Illuminate\\Support"]), true),
        [
            "Illuminate\\Support\\Facades\\DB",
            "Illuminate\\Support\\Str",
            "collect",
            "value"
        ]
    );
    // Constants compare their own name case-sensitively.
    assert_eq!(
        imported(json!(["App\\Limits\\max"]), false),
        Vec::<String>::new()
    );
    assert_eq!(
        imported(json!(["app\\limits\\MAX"]), false),
        ["App\\Limits\\MAX"]
    );
    // `import` reads the references, `imported` the use clauses; either may match.
    assert_eq!(
        matches(
            &json!({ "resolves": { "import": ["Illuminate\\Support\\Facades\\DB"], "imported": ["App\\Limits\\MAX"] } }),
            source
        ),
        ["App\\Limits\\MAX", "DB"]
    );
}

#[test]
fn php_resolves_refuses_what_php_does_not_answer() {
    let at = "/clauses/0/forbid/ast/rule/resolves";
    for (rule, message) in [
        (
            json!({ "kind": "variable_name", "resolves": { "global": ["exec"] } }),
            format!(
                "{at}/global/0: `exec` is no PHP superglobal; expected one of GLOBALS, _SERVER, _GET, _POST, _FILES, _COOKIE, _SESSION, _REQUEST, _ENV"
            ),
        ),
        (
            json!({ "kind": "name", "resolves": { "imported": ["exec"], "relative": "text" } }),
            format!(
                "{at}/relative: a `use` statement names what it imports fully qualified (`use namespace\\A;` does not parse); `import` resolves a `namespace\\A` reference; php has none"
            ),
        ),
        (
            json!({ "kind": "name", "resolves": { "imported": ["Foo"], "seenBefore": true } }),
            format!(
                "{at}/seenBefore: Ruff's `seen_module` asks whether a module was imported before; a PHP `use` only aliases a name for the rest of its namespace and loads nothing; php has none"
            ),
        ),
        (
            json!({ "kind": "variable_name", "resolves": { "this": "global" } }),
            format!(
                "{at}/this: `this` asks whether JavaScript's `this` is the global object; PHP's `$this` never is: outside an object context it is undefined, which `unresolved: true` reports; php has none"
            ),
        ),
        (
            json!({ "kind": "variable_name", "resolves": { "global": ["_GET"], "declared": false } }),
            format!(
                "{at}/declared: declared globals come from ESLint's and Ruff's configuration; PHP's superglobals are always declared, and no languageOptions configure others; php has none"
            ),
        ),
        (
            json!({ "kind": "variable_name", "resolves": { "global": ["_GET"], "directives": "eslint" } }),
            format!(
                "{at}/directives: /* global */ comments are ESLint's; PHP's superglobals are fixed, and a function declares a global with a `global` statement, which `binding` reads; php has none"
            ),
        ),
        (
            json!({ "kind": "variable_name", "resolves": { "global": ["_GET"], "scriptDeclarations": true } }),
            format!(
                "{at}/scriptDeclarations: scripts are ESLint's; a PHP file has no script mode, and a superglobal is never a file's declaration; php has none"
            ),
        ),
        (
            json!({ "kind": "variable_name", "resolves": { "beforeLocal": true } }),
            format!(
                "{at}/beforeLocal: a PHP function sees no variable of an enclosing scope, so a read before the function binds the name is undefined, which `unresolved: true` reports; php has none"
            ),
        ),
        (
            json!({ "kind": "variable_name", "resolves": { "imported": ["exec"], "global": ["_GET"] } }),
            format!(
                "{at}: `import` and `imported` resolve class, function and constant names; `global` and `unresolved` resolve variables; one `resolves` asks one of them"
            ),
        ),
        (
            json!({ "kind": "name", "resolves": { "imported": ["os.path"] } }),
            format!("{at}/imported/0: `os.path` is a dotted name; PHP names join labels with `\\`"),
        ),
    ] {
        assert_eq!(rejection(php_rule(rule)), message);
    }
    let python = compile_document_with(
        clause_document(json!({
            "id": "python-names",
            "forbid": { "ast": { "language": "python", "rule": { "kind": "identifier", "resolves": { "import": ["os\\path"] } } } },
            "violation": violation(),
        })),
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    );
    assert_eq!(
        rejection(python),
        "/clauses/0/forbid/ast/rule/resolves/import/0: `os\\path` is a PHP name; Python imports are dotted names"
    );
}
