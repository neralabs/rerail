use super::closure::Graph;
use super::names::Name;
use super::summary::{Callee, Summary, callee_at};
use crate::code_rule_dsl::language::{Family, GrammarDialect};
use crate::code_rule_dsl::module_path::ModulePath;
use crate::code_rule_dsl::scope::{Reading, ScopeTable};
use ast_grep_language::LanguageExt;
use serde_json::Value as Json;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn oracle_dir() -> PathBuf {
    PathBuf::from(crate::code_rule_dsl::fixtures_dir!()).join("python/calls")
}

fn line_of(source: &str, offset: usize) -> u64 {
    source.as_bytes()[..offset]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count() as u64
        + 1
}

/// A workspace file's module: `pkg/util.py` is `pkg.util`, `pkg/__init__.py` `pkg.__init__`.
fn module_of(path: &str) -> ModulePath {
    let stem = path
        .strip_suffix(".py")
        .or_else(|| path.strip_suffix(".pyi"))
        .expect("a Python path");
    ModulePath::names(&stem.split('/').collect::<Vec<_>>()).expect("module path")
}

fn table(source: &str, module: &ModulePath) -> Option<ScopeTable> {
    let root = GrammarDialect::Python.ast_grep(source);
    ScopeTable::build_in(
        root.root(),
        GrammarDialect::Python,
        usize::MAX,
        &|| false,
        Reading::default(),
        Some(module),
    )
    .ok()
}

/// What Rerail answers at one call node.
#[derive(Clone, Debug, PartialEq)]
enum Answer {
    /// The definitions the callee's name links to (file and `def` line).
    Reaches(BTreeSet<(String, u64)>),
    /// No static name denotes the callee.
    NoEdge,
    /// The name may be a definition Rerail cannot see (an open link or callee).
    Unknown,
}

#[derive(Debug)]
struct Site {
    start: usize,
    end: usize,
    line: u64,
    answer: Answer,
}

/// Rerail's answer at every call node of `files` (path, source), linked as one workspace.
fn answers(files: &[(String, String)]) -> BTreeMap<String, Vec<Site>> {
    let parsed = files
        .iter()
        .map(|(path, source)| {
            let module = module_of(path);
            let root = GrammarDialect::Python.ast_grep(source.as_str());
            let errors = root.root().get_doc().tree.root_node().has_error();
            let table = table(source, &module);
            let summary = Summary::extract(&root.root(), table.as_ref(), Some(&module), errors);
            (module, table, Arc::new(summary))
        })
        .collect::<Vec<_>>();
    let graph = Graph::link(
        files
            .iter()
            .zip(&parsed)
            .map(|((path, _), (_, _, summary))| {
                (Arc::<str>::from(path.as_str()), Arc::clone(summary))
            })
            .collect(),
    );
    let sources = files
        .iter()
        .map(|(path, source)| (path.as_str(), source.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut answers = BTreeMap::new();
    for ((path, source), (module, table, summary)) in files.iter().zip(&parsed) {
        let table = table.as_ref().expect("every oracle file's table builds");
        let root = GrammarDialect::Python.ast_grep(source.as_str());
        let mut sites = Vec::new();
        for node in root.root().dfs().filter(|node| node.kind() == "call") {
            let answer = match callee_at(table, Some(module), summary, &node) {
                None => Answer::NoEdge,
                Some(Callee::Open) => Answer::Unknown,
                Some(Callee::Name(name)) => {
                    let (defs, open) = graph.link_of(&Name::new(Family::Python, &name));
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
                                    )
                                })
                                .collect(),
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

fn python_files(root: &Path, directory: &Path, out: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(directory).expect("project directory") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            python_files(root, &path, out);
        } else if path.extension().is_some_and(|extension| extension == "py") {
            out.push((
                path.strip_prefix(root)
                    .expect("inside the project")
                    .to_string_lossy()
                    .into_owned(),
                std::fs::read_to_string(&path).expect("source"),
            ));
        }
    }
}

/// How Rerail's answer at a call CPython ran departs from what CPython dispatched to there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Departure {
    /// Rerail decides nothing (`calls` is unknown there): sound.
    Unknown,
    /// Rerail reads no static callee, so `calls` is false there, while CPython ran a workspace
    /// function or lambda.
    NoEdge,
    /// Rerail names some of the workspace functions CPython ran there, not all.
    Fewer,
    /// Rerail names a workspace function CPython did not dispatch to there.
    Other,
}

/// Every call where Rerail's answer is not what CPython ran: the call (`file:line` and its text),
/// the departure and why. `Unknown` is Rerail declining to decide; the other kinds are where Rerail
/// decides otherwise than the runtime, recorded so that a change to either side shows here.
const DEPARTURES: &[(&str, Departure, &str)] = &[
    (
        "pkg/aliases.py:21 `defined_later()`",
        Departure::Unknown,
        "a name a star import (`from .util import *`) may bind",
    ),
    (
        "pkg/decorators.py:12 `func(*args, **kwargs)`",
        Departure::Unknown,
        "a call of a parameter",
    ),
    (
        "pkg/decorators.py:34 `wrapped()`",
        Departure::Unknown,
        "a function a wrapping decorator replaced: CPython calls the decorator's `wrapper`, which calls it",
    ),
    (
        "pkg/decorators.py:35 `cached(1)`",
        Departure::Unknown,
        "a function `functools.lru_cache` replaced: CPython calls the cache's C wrapper, which calls it",
    ),
    (
        "pkg/dynamic.py:18 `HANDLERS[name]()`",
        Departure::Unknown,
        "a call of a subscript",
    ),
    (
        "pkg/dynamic.py:22 `callback()`",
        Departure::Unknown,
        "a call of a parameter",
    ),
    (
        "pkg/dynamic.py:26 `getattr(util, name)(1)`",
        Departure::Unknown,
        "a call of `getattr`'s result",
    ),
    (
        "pkg/dynamic.py:31 `square(3)`",
        Departure::Unknown,
        "a lambda bound to a local name",
    ),
    (
        "pkg/dynamic.py:31 `(lambda: first())()`",
        Departure::Unknown,
        "a lambda called where it is written",
    ),
    (
        "pkg/dynamic.py:35 `functools.partial(util.helper, 1)()`",
        Departure::Unknown,
        "a call of a call's result: CPython calls the partial object, which calls `util.helper`",
    ),
    (
        "pkg/models.py:27 `super().__init__(value)`",
        Departure::Unknown,
        "a method of `super()`",
    ),
    (
        "pkg/models.py:30 `super().describe()`",
        Departure::Unknown,
        "a method of `super()`",
    ),
    (
        "pkg/models.py:56 `b.describe()`",
        Departure::Unknown,
        "a method of a local an instantiation assigned (type inference)",
    ),
    (
        "pkg/models.py:58 `NoInit().go()`",
        Departure::Unknown,
        "a method of a call's result (type inference)",
    ),
    (
        "pkg/models.py:59 `Inherits().go()`",
        Departure::Unknown,
        "a method of a call's result (type inference)",
    ),
    (
        "pkg/models.py:60 `Callable()(6)`",
        Departure::Unknown,
        "an instance's `__call__`",
    ),
    (
        "pkg/nested.py:9 `closure(1)`",
        Departure::Unknown,
        "a closure another function returned",
    ),
];

/// Every call of the oracle workspace that CPython ran: the workspace functions it dispatched to
/// are those Rerail names at that call, or the call is one `DEPARTURES` lists with how and why
/// Rerail answers otherwise; and every function Rerail names at a call is one CPython ran there.
#[test]
fn python_calls_agree_with_the_runtime() {
    let expected: Json = serde_json::from_str(
        &std::fs::read_to_string(oracle_dir().join("python.expected.json")).expect("oracle"),
    )
    .expect("JSON");
    let project = oracle_dir().join("project");
    let mut files = Vec::new();
    python_files(&project, &project, &mut files);
    files.sort();
    let sources = files.iter().cloned().collect::<BTreeMap<_, _>>();
    let answers = answers(&files);
    let mut failures = Vec::new();
    let mut departures = BTreeMap::new();
    let mut compared = 0;
    let mut ran_spans = BTreeSet::new();
    for (file, calls) in expected["runtime"].as_object().expect("runtime") {
        let sites = &answers[file];
        for call in calls.as_array().expect("calls") {
            let (start, end) = (
                call["start"].as_u64().expect("start") as usize,
                call["end"].as_u64().expect("end") as usize,
            );
            let at = format!("{file}:{} `{}`", call["line"], &sources[file][start..end]);
            ran_spans.insert((file.clone(), start, end));
            let Some(site) = sites
                .iter()
                .find(|site| (site.start, site.end) == (start, end))
            else {
                failures.push(format!("{at}: no call node"));
                continue;
            };
            let callees = call["callees"].as_array().expect("callees");
            let ran = callees
                .iter()
                .filter(|callee| callee[0] == "def")
                .map(|callee| {
                    (
                        callee[1].as_str().expect("file").to_owned(),
                        callee[2].as_u64().expect("line"),
                    )
                })
                .collect::<BTreeSet<_>>();
            // A lambda is no definition Rerail can name.
            let lambda = callees.iter().any(|callee| callee[0] == "lambda");
            let departure = match &site.answer {
                Answer::Reaches(named) if *named == ran && !lambda => None,
                Answer::NoEdge if ran.is_empty() && !lambda => None,
                Answer::Unknown => Some(Departure::Unknown),
                Answer::NoEdge => Some(Departure::NoEdge),
                Answer::Reaches(named) if named.is_subset(&ran) => Some(Departure::Fewer),
                Answer::Reaches(_) => Some(Departure::Other),
            };
            match departure {
                None => compared += 1,
                Some(departure) => {
                    departures.insert(at, departure);
                }
            }
        }
    }
    // A function Rerail names at a call CPython never ran is unchecked: main.py runs every call
    // the workspace names a function for.
    for (file, sites) in &answers {
        for site in sites {
            if matches!(&site.answer, Answer::Reaches(defs) if !defs.is_empty())
                && !ran_spans.contains(&(file.clone(), site.start, site.end))
            {
                failures.push(format!("{file}:{}: named, never run", site.line));
            }
        }
    }
    let listed = DEPARTURES
        .iter()
        .map(|(at, departure, _)| ((*at).to_owned(), *departure))
        .collect::<BTreeMap<_, _>>();
    assert!(failures.is_empty(), "{failures:#?}");
    assert_eq!(
        departures, listed,
        "calls where Rerail answers otherwise than CPython ran"
    );
    assert!(compared > 60, "{compared} comparisons");
}

/// A file's module as the workspace index gives it from disk (`python_module_path_on_disk`).
fn module_on_disk(path: &Path) -> Option<ModulePath> {
    crate::code_rule_dsl::model::python_module_path_on_disk(path)
        .and_then(|value| match value {
            Json::String(dotted) => ModulePath::dotted(&dotted),
            Json::Array(names) => {
                ModulePath::names(&names.iter().filter_map(Json::as_str).collect::<Vec<_>>())
            }
            _ => None,
        })
        .or_else(|| ModulePath::stem(&path.to_string_lossy()))
}

/// Disagreements with Jedi on the corpora whose cause is understood: Jedi's target (as recorded),
/// the one definition Rerail names (`file:line`), and why.
const CORPUS_DIVERGENCES: &[(&str, &str, &str)] = &[(
    r#"["external","copy.replace"]"#,
    "django/utils/copy.py:7",
    "not comparable: django/utils/copy.py binds `replace` by Python version (`if PY313: from copy import replace`, \
     else a backport `def replace`); Jedi, run on CPython 3.13, follows the import, Rerail names the backport. This \
     checkout requires Python >= 3.12 (pyproject.toml), where the backport is what runs",
)];

/// Jedi's call targets over Python corpora (`RERAIL_PYTHON_CALLS`, written by
/// `fixtures/python/calls/generate_corpus.py`), against what Rerail names at the same calls
/// with every file of the corpus linked; the summary goes to `RERAIL_PYTHON_CALLS_REPORT`. Rerail may
/// decide nothing (a callee no static name denotes, an open link); where it names a definition,
/// Jedi's must be among those it names.
#[test]
#[ignore = "Jedi call targets over Python corpora"]
fn benchmark_python_calls_corpus() {
    let input = std::env::var("RERAIL_PYTHON_CALLS").expect("RERAIL_PYTHON_CALLS");
    let output = std::env::var("RERAIL_PYTHON_CALLS_REPORT").expect("report path");
    let corpora: Json =
        serde_json::from_str(&std::fs::read_to_string(input).expect("corpora")).expect("JSON");
    let mut summary = Vec::new();
    let mut all_failures = 0;
    for corpus in corpora["corpora"].as_array().expect("corpora") {
        let checkout = PathBuf::from(corpus["checkout"].as_str().expect("checkout"));
        let mut files = Vec::new();
        for path in corpus["paths"].as_array().expect("paths") {
            python_files(
                &checkout,
                &checkout.join(path.as_str().expect("path")),
                &mut files,
            );
        }
        files.retain(|(path, _)| {
            !path
                .split('/')
                .any(|part| part.starts_with('.') || part == "__pycache__")
        });
        files.sort();
        let records = corpus["records"].as_object().expect("records");
        let parsed = files
            .iter()
            .map(|(path, source)| {
                let module = module_on_disk(&checkout.join(path)).expect("module");
                let root = GrammarDialect::Python.ast_grep(source.as_str());
                let errors = root.root().get_doc().tree.root_node().has_error();
                let table = table(source, &module);
                let summary = Summary::extract(&root.root(), table.as_ref(), Some(&module), errors);
                (module, table, errors, Arc::new(summary))
            })
            .collect::<Vec<_>>();
        let graph = Graph::link(
            files
                .iter()
                .zip(&parsed)
                .map(|((path, _), parsed)| (Arc::<str>::from(path.as_str()), Arc::clone(&parsed.3)))
                .collect(),
        );
        let sources = files
            .iter()
            .map(|(path, source)| (path.as_str(), source.as_str()))
            .collect::<BTreeMap<_, _>>();
        // Every definition's label by file and line (a class's at its `class` line).
        let mut labels = BTreeMap::<(String, u64), Vec<String>>::new();
        for def in 0..graph.def_count() as u32 {
            let def = graph.def_ref(def);
            let (file, start) = graph.def_site(def);
            labels
                .entry((
                    file.to_string(),
                    line_of(sources[file.as_ref()], start as usize),
                ))
                .or_default()
                .push(graph.def_label(def).into_owned());
        }
        let mut tally = BTreeMap::<&str, usize>::new();
        let mut failures = Vec::new();
        let mut unparsed = 0;
        let mut documented = BTreeSet::new();
        for ((path, source), (module, table, errors, summary)) in files.iter().zip(&parsed) {
            let Some(file_records) = records.get(path) else {
                continue;
            };
            if *errors {
                // The grammar reads the file with errors (its summary is open): not compared.
                unparsed += 1;
                continue;
            }
            let Some(table) = table.as_ref() else {
                *tally.entry("fileWithoutTable").or_default() += 1;
                continue;
            };
            let root = GrammarDialect::Python.ast_grep(source.as_str());
            let mut sites = BTreeMap::new();
            for node in root.root().dfs().filter(|node| node.kind() == "call") {
                let range = node.range();
                sites.insert(
                    (range.start, range.end),
                    callee_at(table, Some(module), summary, &node),
                );
            }
            for record in file_records.as_array().expect("records") {
                let record = record.as_array().expect("record");
                let span = (
                    record[0].as_u64().expect("start") as usize,
                    record[1].as_u64().expect("end") as usize,
                );
                let targets = record[3].as_array().expect("targets");
                *tally.entry("calls").or_default() += 1;
                let Some(callee) = sites.get(&span) else {
                    *tally.entry("withoutNode").or_default() += 1;
                    continue;
                };
                let [target] = targets.as_slice() else {
                    *tally.entry("jediUndecided").or_default() += 1;
                    continue;
                };
                let kind = target[0].as_str().expect("kind");
                if kind == "other" {
                    *tally.entry("jediUndecided").or_default() += 1;
                    continue;
                }
                let answer = match callee {
                    None => Answer::NoEdge,
                    Some(Callee::Open) => Answer::Unknown,
                    Some(Callee::Name(name)) => {
                        let (defs, open) = graph.link_of(&Name::new(Family::Python, name));
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
                                        )
                                    })
                                    .collect(),
                            )
                        }
                    }
                };
                let named = match &answer {
                    Answer::Unknown => {
                        *tally.entry("rerailUnknown").or_default() += 1;
                        continue;
                    }
                    // No edge claims the call runs no workspace definition: Jedi's function, or
                    // its class's `__init__`, contradicts it.
                    Answer::NoEdge if kind == "def" || kind == "class" => {
                        *tally.entry("rerailNoEdgeJediWorkspace").or_default() += 1;
                        failures.push(format!(
                            "{path}:{}: Jedi {target}, Rerail no edge",
                            record[2]
                        ));
                        continue;
                    }
                    Answer::NoEdge => {
                        *tally.entry("rerailNoEdge").or_default() += 1;
                        continue;
                    }
                    Answer::Reaches(named) => named,
                };
                let agreed = match kind {
                    "external" => named.is_empty(),
                    "def" => {
                        let at = (
                            target[1].as_str().expect("file").to_owned(),
                            target[2].as_u64().expect("line"),
                        );
                        // Naming no definition claims an external callee.
                        named.contains(&at)
                    }
                    "class" => {
                        let at = (
                            target[1].as_str().expect("file").to_owned(),
                            target[2].as_u64().expect("line"),
                        );
                        let Some(classes) = labels.get(&at) else {
                            *tally.entry("jediClassUnlinked").or_default() += 1;
                            continue;
                        };
                        // What instantiating Jedi's class runs (its `__new__` and `__init__` as
                        // its method resolution order gives them) is among what Rerail names: a
                        // late-bound `cls()` names subclasses' too; a class whose instantiation
                        // runs nothing of the workspace may name nothing.
                        classes.iter().all(|class| {
                            let (defs, open) = graph.link_of(&Name::new(Family::Python, class));
                            open || defs.iter().all(|def| {
                                let (file, start) = graph.def_site(*def);
                                named.contains(&(
                                    file.to_string(),
                                    line_of(sources[file.as_ref()], start as usize),
                                ))
                            })
                        })
                    }
                    other => panic!("unexpected Jedi target {other}"),
                };
                if agreed {
                    *tally.entry("agreed").or_default() += 1;
                } else if let Some((_, _, why)) =
                    CORPUS_DIVERGENCES.iter().find(|(jedi, rerail, _)| {
                        serde_json::from_str::<Json>(jedi).is_ok_and(|jedi| jedi == *target)
                            && named
                                .iter()
                                .all(|(file, line)| format!("{file}:{line}") == *rerail)
                    })
                {
                    *tally.entry("documented").or_default() += 1;
                    documented.insert(format!("{target} / Rerail {named:?}: {why}"));
                } else {
                    *tally.entry("disagreed").or_default() += 1;
                    failures.push(format!(
                        "{path}:{}: Jedi {target}, Rerail {named:?}",
                        record[2]
                    ));
                }
            }
        }
        all_failures += failures.len();
        summary.push(serde_json::json!({
            "corpus": corpus["name"],
            "linkedFiles": files.len(),
            "sampledFiles": records.len(),
            "filesWithSyntaxErrors": unparsed,
            "tally": tally,
            "disagreements": failures,
            "documented": documented,
        }));
    }
    let report = serde_json::json!({
        "benchmark": "Python calls: the definitions Rerail's call graph names against Jedi's inference",
        "jedi": corpora["jedi"],
        "python": corpora["python"],
        "corpora": summary,
    });
    std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).expect("write report");
    assert_eq!(all_failures, 0, "{all_failures} disagreements");
}
