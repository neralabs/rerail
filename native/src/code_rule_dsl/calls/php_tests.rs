use super::closure::Graph;
use super::names::Name;
use super::php::{self, Reader};
use super::summary::{Callee, Summary};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::scope::php_names::PhpNames;
use crate::code_rule_dsl::scope::{Reading, ScopeTable};
use ast_grep_language::LanguageExt;
use serde_json::Value as Json;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn oracle_dir() -> PathBuf {
    PathBuf::from(env!("RERAIL_FIXTURES_DIR")).join("php/calls/php")
}

fn line_of(source: &str, offset: usize) -> u64 {
    source.as_bytes()[..offset]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count() as u64
        + 1
}

/// What Rerail answers at one call node.
#[derive(Clone, Debug, PartialEq)]
enum Answer {
    /// The callee's definitions in the workspace (file, start line and label), and whether the
    /// call is late-bound (`$this->m()`, `static::m()`, `new static`): the declaration the class
    /// looks up and every override.
    Reaches(BTreeSet<(String, u64, String)>, bool),
    /// The node calls nothing statically named.
    NoEdge,
    Unknown,
}

#[derive(Debug)]
struct Site {
    start: usize,
    end: usize,
    line: u64,
    answer: Answer,
}

fn summarize(path: &str, source: &str) -> Summary {
    let root = GrammarDialect::Php.ast_grep(source);
    let errors = root.root().get_doc().tree.root_node().has_error();
    super::summarize(
        root.root(),
        GrammarDialect::Php,
        path,
        None,
        errors,
        source.len(),
        None,
    )
    .0
}

/// Rerail's answer at every call node of `files` (path, source) that `wanted` holds (all without
/// it), linked as one workspace.
fn answers(
    files: &[(String, String)],
    wanted: Option<&BTreeSet<String>>,
) -> BTreeMap<String, Vec<Site>> {
    let summaries = files
        .iter()
        .map(|(path, source)| {
            (
                Arc::<str>::from(path.as_str()),
                Arc::new(summarize(path, source)),
            )
        })
        .collect::<Vec<_>>();
    let graph = Graph::link(summaries.clone());
    let sources = files
        .iter()
        .map(|(path, source)| (path.as_str(), source.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut answers = BTreeMap::new();
    for ((path, source), (_, summary)) in files.iter().zip(&summaries) {
        if wanted.is_some_and(|wanted| !wanted.contains(path)) {
            continue;
        }
        let root = GrammarDialect::Php.ast_grep(source);
        let Ok(table) = ScopeTable::build_in(
            root.root(),
            GrammarDialect::Php,
            usize::MAX,
            &|| false,
            Reading::default(),
            None,
        ) else {
            continue;
        };
        let Ok(names) = PhpNames::build(root.root(), usize::MAX, &|| false) else {
            continue;
        };
        let reader = Reader {
            names: &names,
            table: &table,
            root: root.root(),
        };
        let mut sites = Vec::new();
        for node in root.root().dfs() {
            if !php::CALL_KINDS.contains(&node.kind().as_ref()) {
                continue;
            }
            let answer = match php::callee_at(&reader, summary, &node) {
                None => Answer::NoEdge,
                Some(Callee::Open) => Answer::Unknown,
                Some(Callee::Name(name)) => {
                    let (defs, open) = graph.link_of(&Name::new(&name));
                    if open {
                        Answer::Unknown
                    } else {
                        Answer::Reaches(
                            defs.iter()
                                .map(|def| {
                                    let (file, start) = graph.def_site(*def);
                                    (
                                        file.to_string(),
                                        line_of(sources[file.as_ref()], start as usize),
                                        graph.def_label(*def).into_owned(),
                                    )
                                })
                                .collect(),
                            name.ends_with('^'),
                        )
                    }
                }
            };
            let range = node.range();
            sites.push(Site {
                start: range.start,
                end: range.end,
                line: line_of(source, range.start),
                answer,
            });
        }
        answers.insert(path.clone(), sites);
    }
    answers
}

fn workspace_files(directory: &Path) -> Vec<(String, String)> {
    let mut files = std::fs::read_dir(directory)
        .expect("project")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "php"))
        .map(|path| {
            (
                path.file_name()
                    .expect("name")
                    .to_string_lossy()
                    .into_owned(),
                std::fs::read_to_string(&path).expect("source"),
            )
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

/// Whether Rerail's answer is PHPStan's `targets` (one class's): `Some(true)` agrees, `Some(false)`
/// disagrees, `None` where Rerail decides nothing. A declaration outside the workspace (`linked`
/// tells the workspace's files) is no definition Rerail can name. PHPStan names the one
/// declaration it loads; Rerail names every declaration of that name the workspace holds, and a
/// late-bound call reaches it and the overrides of the classes derived from it.
fn agrees(answer: &Answer, targets: &[Json], linked: &dyn Fn(&str) -> bool) -> Option<bool> {
    let [target] = targets else {
        return None;
    };
    let kind = target[0].as_str().expect("target kind");
    let (reaches, late) = match answer {
        Answer::Unknown => return None,
        Answer::NoEdge => (None, false),
        Answer::Reaches(defs, late) => (Some(defs), *late),
    };
    let none = || reaches.is_none_or(BTreeSet::is_empty);
    let declared = |reaches: &BTreeSet<(String, u64, String)>,
                    is: &dyn Fn(&(String, u64, String)) -> bool| {
        let label = |def: &(String, u64, String)| def.2.to_ascii_lowercase();
        reaches
            .iter()
            .find(|def| is(def))
            .is_some_and(|found| late || reaches.iter().all(|def| label(def) == label(found)))
    };
    match kind {
        "unresolved" => None,
        "callable" => Some(reaches.is_none()),
        // PHP's own, or no constructor: no definition, but for a late-bound call the overrides
        // of the classes the workspace derives.
        "builtin" | "builtin-method" | "none" => Some(late || none()),
        "function" | "method" => {
            let file = target[if kind == "function" { 2 } else { 3 }]
                .as_str()
                .expect("file");
            if !linked(file) {
                return Some(none());
            }
            // No edge: a receiver whose class needs type inference.
            let reaches = reaches?;
            Some(if kind == "function" {
                let name = target[1].as_str().expect("name").to_ascii_lowercase();
                declared(reaches, &|(path, _, label)| {
                    path == file && label.to_ascii_lowercase() == name
                })
            } else {
                let line = target[4].as_u64().expect("line");
                declared(reaches, &|(path, at, _)| path == file && *at == line)
            })
        }
        other => panic!("unexpected PHPStan target {other}"),
    }
}

/// Whether PHPStan's reflection decides what the call runs: not where it resolves nothing, nor
/// for `call_user_func` and `call_user_func_array`, whose callback (what Rerail follows) the PHP
/// runtime decides.
fn phpstan_decides(targets: &[Json]) -> bool {
    targets.iter().all(|target| {
        target[0] != "unresolved"
            && !(target[0] == "builtin"
                && matches!(
                    target[1].as_str(),
                    Some("call_user_func" | "call_user_func_array")
                ))
    })
}

/// Calls PHPStan resolves where Rerail, by design, decides nothing: `file:line` and why.
const UNDECIDED: &[(&str, &str)] = &[
    (
        "model.php:157",
        "a method of a variable a function call assigned (type inference)",
    ),
    ("model.php:161", "a method of an enum case (type inference)"),
    ("more.php:36", "$this in a trait: the using class's"),
    (
        "more.php:83",
        "self in a closure passed to a call, which may bind it elsewhere",
    ),
    ("more.php:98", "$this in a trait: the using class's"),
    (
        "more.php:144",
        "$this in a closure passed to a call: PHP binds it to Other, PHPStan says Host",
    ),
    (
        "more.php:149",
        "$this in a closure a variable passes to a call: PHP binds it to Other, PHPStan says Host",
    ),
];

/// Calls PHP ran where Rerail, by design, decides nothing (a callee it leaves unknown, or one
/// `UNDECIDED` lists): `file:line` and why.
const RUNTIME_UNDECIDED: &[(&str, &str)] = &[
    ("callables.php:66", "a dynamic method name, `$o->$name()`"),
    ("callables.php:71", "a callable parameter, `$cb()`"),
    (
        "callables.php:72",
        "`call_user_func` of a callable parameter",
    ),
    ("model.php:157", "as UNDECIDED: type inference"),
    ("model.php:161", "as UNDECIDED: type inference"),
    ("more.php:36", "as UNDECIDED: $this in a trait"),
    (
        "more.php:77",
        "a variable that holds a different closure on each path",
    ),
    (
        "more.php:79",
        "a variable a closure's `use` copies, whose value it does not follow",
    ),
    ("more.php:83", "as UNDECIDED: self in a passed closure"),
    ("more.php:98", "as UNDECIDED: $this in a trait"),
    (
        "more.php:132",
        "the closure `Closure::bind` returns, called at once",
    ),
    ("more.php:144", "as UNDECIDED: $this in a passed closure"),
    (
        "more.php:149",
        "as UNDECIDED: $this in a closure a variable passes",
    ),
];

/// Every call of the oracle workspace: where PHPStan's reflection resolves the callee, Rerail
/// names the same declaration (or none, for a builtin or a class without a constructor), or
/// decides nothing for a reason `UNDECIDED` lists; where PHP ran a call, the declarations it
/// dispatched to are those Rerail names at that line, and every declaration Rerail names is one
/// PHP dispatched to, or Rerail decides nothing for a reason `RUNTIME_UNDECIDED` lists.
#[test]
fn php_calls_agree_with_phpstan_and_the_runtime() {
    let expected: Json = serde_json::from_str(
        &std::fs::read_to_string(oracle_dir().join("php.expected.json")).expect("oracle"),
    )
    .expect("JSON");
    let files = workspace_files(&oracle_dir().join("project"));
    let answers = answers(&files, None);
    let mut failures = Vec::new();
    let mut undecided = BTreeSet::new();
    let mut compared = 0;
    for (file, records) in expected["phpstan"].as_object().expect("phpstan") {
        let sites = &answers[file];
        for record in records.as_array().expect("records") {
            let (start, end) = (
                record["start"].as_u64().expect("start") as usize,
                record["end"].as_u64().expect("end") as usize,
            );
            let at = format!("{file}:{}", record["line"]);
            let Some(site) = sites
                .iter()
                .find(|site| (site.start, site.end) == (start, end))
            else {
                failures.push(format!("{at}: no call node at {start}..{end}"));
                continue;
            };
            let targets = record["targets"].as_array().expect("targets");
            if !phpstan_decides(targets) {
                continue;
            }
            match agrees(&site.answer, targets, &|_| true) {
                Some(true) => compared += 1,
                Some(false) => failures.push(format!(
                    "{at}: PHPStan {targets:?}, Rerail {:?}",
                    site.answer
                )),
                None => {
                    undecided.insert(at);
                }
            }
        }
    }
    let allowed = UNDECIDED
        .iter()
        .map(|(at, _)| (*at).to_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(undecided, allowed, "calls Rerail leaves undecided");
    let mut runtime_lines = BTreeSet::new();
    let mut runtime_undecided = BTreeSet::new();
    for (file, calls) in expected["runtime"].as_object().expect("runtime") {
        let sites = &answers[file];
        for call in calls.as_array().expect("calls") {
            let line = call["line"].as_u64().expect("line");
            runtime_lines.insert((file.clone(), line));
            let ran = call["callees"]
                .as_array()
                .expect("callees")
                .iter()
                .map(|callee| {
                    (
                        callee[0].as_str().expect("file").to_owned(),
                        callee[1].as_u64().expect("line"),
                    )
                })
                .collect::<BTreeSet<_>>();
            let on_line = sites.iter().filter(|site| site.line == line);
            let mut named = BTreeSet::new();
            let mut decided = true;
            let mut late = false;
            for site in on_line {
                match &site.answer {
                    Answer::Reaches(defs, bound) => {
                        named.extend(defs.iter().map(|(path, at, _)| (path.clone(), *at)));
                        late |= bound;
                    }
                    Answer::Unknown => decided = false,
                    Answer::NoEdge => {
                        decided &= !allowed.contains(&format!("{file}:{line}"));
                    }
                }
            }
            // A late-bound call names every override; PHP ran those its objects' classes look up.
            let at = format!("{file}:{line}");
            let agree = if late {
                ran.is_subset(&named)
            } else {
                named == ran
            };
            if decided && !agree || !late && !named.is_subset(&ran) {
                failures.push(format!("{at}: PHP ran {ran:?}, Rerail names {named:?}"));
            } else if decided {
                compared += 1;
            } else {
                runtime_undecided.insert(at);
            }
        }
    }
    let runtime_allowed = RUNTIME_UNDECIDED
        .iter()
        .map(|(at, _)| (*at).to_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        runtime_undecided, runtime_allowed,
        "calls PHP ran that Rerail leaves undecided"
    );
    // A declaration Rerail names at a call PHP never ran there is unchecked: the workspace runs
    // every call it names one for.
    for (file, sites) in &answers {
        for site in sites {
            if matches!(&site.answer, Answer::Reaches(defs, _) if !defs.is_empty())
                && !runtime_lines.contains(&(file.clone(), site.line))
            {
                failures.push(format!("{file}:{}: named, never run", site.line));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
    assert!(compared > 100, "{compared} comparisons");
}

/// Rerail's answers at the calls of the one-file workspace `source` whose text is `call`, in order.
fn answers_at(source: &str, call: &str) -> Vec<Answer> {
    let files = [("t.php".to_owned(), source.to_owned())];
    answers(&files, None)["t.php"]
        .iter()
        .filter(|site| &source[site.start..site.end] == call)
        .map(|site| site.answer.clone())
        .collect()
}

/// A trait adaptation decides the overrides a late-bound call in the using class reaches: a
/// method imported `as private` is the class's own whatever a derived class declares; a private
/// one imported `as public` (or aliased so) gives way to an override. PHP 8.3's `m as final` is
/// a modifier, not an alias named `final`.
#[test]
fn trait_adaptations_decide_late_bound_overrides() {
    let source = r#"<?php
namespace App;
trait T
{
    public function open(): void {}
    private function shut(): void {}
    public function fixed(): void {}
}
class C
{
    use T {
        open as private;
        open as private hidden;
        shut as public;
        shut as protected ajar;
        fixed as final;
    }
    public function run(): void
    {
        $this->open();
        $this->hidden();
        $this->shut();
        $this->ajar();
        $this->final();
    }
}
class D extends C
{
    public function open(): void {}
    public function hidden(): void {}
    public function shut(): void {}
    public function ajar(): void {}
}
"#;
    let def = |at: Option<usize>, label: &str| {
        (
            "t.php".to_owned(),
            line_of(source, at.expect("declaration")),
            label.to_owned(),
        )
    };
    let late = |defs: &[&(String, u64, String)]| {
        vec![Answer::Reaches(
            defs.iter().map(|def| (*def).clone()).collect(),
            true,
        )]
    };
    let trait_open = def(source.find("function open"), "App\\T::open");
    let trait_shut = def(source.find("function shut"), "App\\T::shut");
    let shut = def(source.rfind("function shut"), "App\\D::shut");
    let ajar = def(source.find("function ajar"), "App\\D::ajar");
    assert_eq!(answers_at(source, "$this->open()"), late(&[&trait_open]));
    assert_eq!(answers_at(source, "$this->hidden()"), late(&[&trait_open]));
    assert_eq!(
        answers_at(source, "$this->shut()"),
        late(&[&trait_shut, &shut])
    );
    assert_eq!(
        answers_at(source, "$this->ajar()"),
        late(&[&trait_shut, &ajar])
    );
    assert_eq!(answers_at(source, "$this->final()"), late(&[]));
}

/// `$this` in a closure is the class's only where the closure cannot reach code that binds it
/// elsewhere: PHP calls it where it is made, or it is held by a variable every read of which
/// calls it. Returned, captured by `use` (by value or reference), read for `->bindTo()`,
/// assigned on, or read by `compact`, it is unknown.
#[test]
fn closures_that_may_be_rebound_leave_this_unknown() {
    let source = r#"<?php
namespace App;
final class H
{
    public function hi(): void {}
    public function kept(): void
    {
        $f = function (): void { $this->hi(); };
        $f();
        $f();
        (function (): void { $this->hi(); })();
    }
    public function returned(): \Closure
    {
        $f = function (): void { $this->hi(); };
        return $f;
    }
    public function captured(): void
    {
        $f = fn () => $this->hi();
        $g = function () use ($f): void {};
        $g();
    }
    public function referenced(): void
    {
        $f = fn () => $this->hi();
        $g = function () use (&$f): void {};
        $g();
    }
    public function rebound(): void
    {
        $f = function (): void { $this->hi(); };
        $f->bindTo(null);
        $f();
    }
    public function chained(): void
    {
        $f = $g = function (): void { $this->hi(); };
        $f();
    }
    public function listed(): void
    {
        $f = function (): void { $this->hi(); };
        $f();
        compact('f');
    }
}
"#;
    let hi = (
        "t.php".to_owned(),
        line_of(source, source.find("function hi").expect("hi")),
        "App\\H::hi".to_owned(),
    );
    let bound = Answer::Reaches(BTreeSet::from([hi]), false);
    let mut expected = vec![bound.clone(), bound];
    expected.resize(8, Answer::Unknown);
    assert_eq!(answers_at(source, "$this->hi()"), expected);
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

/// PHPStan's call targets over sampled corpora (`RERAIL_PHP_CALLS`, written by
/// `rerail-fixtures/php/calls/php/generate_corpus.py`), against what Rerail names at the same calls with every file of
/// the corpus linked; the summary goes to `RERAIL_PHP_CALLS_REPORT`. Rerail may decide nothing (an
/// unknown callee, a receiver whose class needs type inference); where it names a declaration it
/// must be PHPStan's.
#[test]
#[ignore = "PHPStan call targets over PHP corpora"]
fn benchmark_php_calls_corpus() {
    let input = std::env::var("RERAIL_PHP_CALLS").expect("RERAIL_PHP_CALLS");
    let output = std::env::var("RERAIL_PHP_CALLS_REPORT").expect("report path");
    let corpora: Json =
        serde_json::from_str(&std::fs::read_to_string(input).expect("corpora")).expect("JSON");
    let mut summary = Vec::new();
    let mut all_failures = Vec::new();
    for corpus in corpora["corpora"].as_array().expect("corpora") {
        let excluded = corpus["excluded"]
            .as_array()
            .expect("excluded")
            .iter()
            .filter_map(Json::as_str)
            .collect::<BTreeSet<_>>();
        let mut paths = Vec::new();
        for root in corpus["roots"].as_array().expect("roots") {
            php_files(Path::new(root.as_str().expect("root")), &mut paths);
        }
        paths.sort();
        let files = paths
            .iter()
            .filter(|path| !excluded.contains(path.to_string_lossy().as_ref()))
            .filter_map(|path| {
                Some((
                    path.to_string_lossy().into_owned(),
                    std::fs::read_to_string(path).ok()?,
                ))
            })
            .collect::<Vec<_>>();
        let records = corpus["records"].as_object().expect("records");
        // Files the grammar reads with errors are outside what Rerail decides (their summaries
        // are open): not compared.
        let unparsed = files
            .iter()
            .filter(|(path, source)| {
                records.contains_key(path)
                    && GrammarDialect::Php
                        .ast_grep(source)
                        .root()
                        .dfs()
                        .any(|node| node.is_error() || node.is_missing())
            })
            .map(|(path, _)| path.clone())
            .collect::<BTreeSet<_>>();
        let wanted = records
            .keys()
            .filter(|path| !unparsed.contains(*path))
            .cloned()
            .collect::<BTreeSet<_>>();
        let answers = answers(&files, Some(&wanted));
        let linked = files
            .iter()
            .map(|(path, _)| path.as_str())
            .collect::<BTreeSet<_>>();
        let mut tally = BTreeMap::<&str, usize>::new();
        let mut failures = Vec::new();
        for (file, sites) in &answers {
            for record in records[file].as_array().expect("records") {
                let record = record.as_array().expect("record");
                let (start, end) = (
                    record[0].as_u64().expect("start") as usize,
                    record[1].as_u64().expect("end") as usize,
                );
                let targets = record[4].as_array().expect("targets");
                *tally.entry("calls").or_default() += 1;
                let Some(site) = sites
                    .iter()
                    .find(|site| (site.start, site.end) == (start, end))
                else {
                    // PHPStan's second reading of a nullsafe call shares its range; a record
                    // without a node is a construct the grammar reads otherwise.
                    *tally.entry("withoutNode").or_default() += 1;
                    continue;
                };
                if !phpstan_decides(targets) || targets.len() != 1 {
                    *tally.entry("phpstanUndecided").or_default() += 1;
                    continue;
                }
                let key = match agrees(&site.answer, targets, &|file| linked.contains(file)) {
                    Some(true) => "agreed",
                    Some(false) => {
                        failures.push(format!(
                            "{file}:{}: PHPStan {targets:?}, Rerail {:?}",
                            record[2], site.answer
                        ));
                        "disagreed"
                    }
                    None if site.answer == Answer::Unknown => "rerailUnknown",
                    None => "rerailNoEdge",
                };
                *tally.entry(key).or_default() += 1;
            }
        }
        summary.push(serde_json::json!({
            "corpus": corpus["name"],
            "linkedFiles": files.len(),
            "sampledFiles": records.len(),
            "filesWithSyntaxErrors": unparsed.len(),
            "tally": tally,
            "disagreements": failures,
        }));
        all_failures.extend(failures);
    }
    let report = serde_json::json!({
        "benchmark": "PHP calls: the declarations Rerail's call graph names against PHPStan's reflection",
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
