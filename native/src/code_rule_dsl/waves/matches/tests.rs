//! 06-first-slice S4's gate: the Python pack's lowered clauses as one wave over the pack's
//! fixtures, ingested through the push path (S3). Every clause's prefilter candidates contain the
//! nodes W's matcher matches at (the prefilter never under-produces), and its match facts are
//! exactly those nodes (S1's oracle). `RERAIL_S1_FILES` and `RERAIL_S1_FROM` pick the fixtures as
//! in S1 (default: the first 8 of every eighth), `RERAIL_S1_CLAUSE` the clauses.

use super::super::owner::Backing;
use super::super::findings::{Layer, PROGRAM as FINDINGS, Shape};
use super::super::schedule::{Limits, family_name, run};
use crate::code_rule_dsl::findings::contract::Coverage;
use super::*;
use crate::code_rule_dsl::CodeRuleDslRuntime;
use crate::code_rule_dsl::facts::observe::roots::directory_root;
use crate::code_rule_dsl::matcher::{GrammarMatcher, match_tree, node_at};
use crate::code_rule_dsl::model::{ClauseKind, Query};
use crate::code_rule_dsl::test_support::CAPABILITIES;
use crate::content_store::ContentStore;
use ast_grep_core::AstGrep as Root;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const WS: &str = "/slice";

fn pack() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../packs/python")
}

fn files(directory: &Path, extension: &str, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files(&path, extension, found);
        } else if path.extension().is_some_and(|found| found == extension) {
            found.push(path);
        }
    }
}

fn temp(what: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("rerail-s4-{what}-{}-{nonce}", std::process::id()))
}

/// The fixtures, copied into a directory as `f<n>.py`: `RERAIL_S1_FILES` of them, else `default`.
fn fixtures(dir: &Path, default: usize) -> Vec<String> {
    let mut sources = Vec::new();
    files(&pack().join("upstream"), "py", &mut sources);
    sources.sort();
    let most = std::env::var("RERAIL_S1_FILES").ok().and_then(|n| n.parse().ok()).unwrap_or(default);
    let from = std::env::var("RERAIL_S1_FROM").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
    std::fs::create_dir_all(dir).unwrap();
    sources
        .iter()
        .step_by(8)
        .skip(from)
        .take(most)
        .enumerate()
        .map(|(n, source)| {
            let path = format!("f{n:03}.py");
            std::fs::copy(source, dir.join(&path)).unwrap();
            path
        })
        .collect()
}

/// The Python pack's lowered AST clauses, with W's matcher for each.
fn clauses() -> (Vec<Clause>, Vec<GrammarMatcher>) {
    let (clauses, grammars, _) = shaped();
    (clauses, grammars)
}

/// The Python pack's lowered AST clauses, with W's matcher and the findings layer's shape for each.
fn shaped() -> (Vec<Clause>, Vec<GrammarMatcher>, Vec<Shape>) {
    shaped_from(&policies())
}

/// The lowered AST clauses of `policies`, with W's matcher and the findings layer's shape for each.
fn shaped_from(policies: &[(String, crate::code_rule_dsl::CompiledCodeRuleDsl)]) -> (Vec<Clause>, Vec<GrammarMatcher>, Vec<Shape>) {
    let mut clauses = Vec::new();
    let mut grammars = Vec::new();
    let mut shapes = Vec::new();
    for (policy, compiled) in policies {
        for (at, clause) in compiled.inner.clauses.iter().enumerate() {
            let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &clause.kind else { continue };
            for (index, grammar) in matchers.iter().enumerate().filter(|(_, matcher)| matcher.language == GrammarDialect::Python) {
                let name = format!("{policy}/{}", clause.id);
                if std::env::var("RERAIL_S1_CLAUSE").is_ok_and(|part| !name.contains(&part)) {
                    continue;
                }
                let Ok(tree) = match_tree(grammar) else { continue };
                let Ok(matcher) = LoweredMatcher::lower(&tree) else { continue };
                clauses.push(Clause { name, plan: Arc::clone(&grammar.anchors), matcher });
                grammars.push(grammar.clone());
                shapes.push(Shape { rule: Arc::clone(&compiled.inner), clause: at, grammar: index });
            }
        }
    }
    (clauses, grammars, shapes)
}

/// The nodes W's matcher matches at in `tape`.
fn oracle(grammar: &GrammarMatcher, tape: &crate::code_rule_dsl::tape::TapeDoc) -> BTreeSet<u32> {
    let root = Root::doc(tape.clone());
    let mut work = usize::MAX;
    (0..tape.transducer_len() as u32)
        .filter(|&ordinal| grammar.match_root(node_at(&root, ordinal), &mut work).ok().flatten().is_some())
        .collect()
}

/// The fixtures pushed through the families (S3) into a fresh store.
fn ingested(default: usize) -> (ContentStore, Vec<String>) {
    let dir = temp("fixtures");
    let paths = fixtures(&dir, default);
    pushed(&dir, paths)
}

/// `sources`, written as `f<n>.py`, pushed through the families into a fresh store.
fn ingested_sources(sources: &[&str]) -> (ContentStore, Vec<String>) {
    let dir = temp("sources");
    std::fs::create_dir_all(&dir).unwrap();
    let paths = sources
        .iter()
        .enumerate()
        .map(|(n, source)| {
            let path = format!("f{n:03}.py");
            std::fs::write(dir.join(&path), source).unwrap();
            path
        })
        .collect();
    pushed(&dir, paths)
}

/// The files `paths` of `dir` pushed through the families into a fresh store.
fn pushed(dir: &Path, paths: Vec<String>) -> (ContentStore, Vec<String>) {
    let store = ContentStore::open(&temp("store").with_extension("sqlite")).unwrap();
    let root = directory_root(&store, dir).unwrap();
    Families::default().push(&store, WS, root, 8, |_| Ok(Vec::new())).unwrap();
    (store, paths)
}

#[test]
fn the_wave_finds_ws_matches_from_the_prefilters_candidates() {
    gate(8, Limits::default(), ExecutionBackend::CpuJit);
}

/// On Metal: the labels in one command buffer, the roots through `demand(batch)` on Metal's
/// compiled kernel, or in the background mode its interpreter until the kernel is built.
#[cfg(all(target_os = "macos", feature = "metal-tests"))]
#[test]
fn the_wave_finds_ws_matches_on_metal() {
    gate(8, Limits::default(), ExecutionBackend::MetalJit);
    gate(4, Limits { chunk_bytes: 1, ..Limits::default() }, ExecutionBackend::MetalBackground);
}

/// In the background mode the first chunks run on the interpreter, with the same matches.
#[test]
fn the_wave_runs_while_its_programs_compile() {
    gate(4, Limits { chunk_bytes: 1, ..Limits::default() }, ExecutionBackend::CpuBackground);
}

/// Chunks of one document each: a chunk's batch maps its files to its own documents.
#[test]
fn the_wave_runs_chunk_by_chunk() {
    gate(4, Limits { chunk_bytes: 1, ..Limits::default() }, ExecutionBackend::CpuJit);
}

fn gate(default: usize, limits: Limits, backend: ExecutionBackend) {
    let (store, paths) = ingested(default);
    let (clauses, grammars) = clauses();
    let documents = documents(&store, WS, &paths).unwrap();
    assert_eq!(documents.len(), paths.len(), "every fixture has core facts");
    let mut wave = Wave::new(&clauses, backend).unwrap();
    let program = wave.program(&store).unwrap();
    // One step per output (`match`, `span`), neither reading the other: they share the wave.
    assert_eq!(program.steps.len(), 2, "{:?}", program.steps.iter().map(|s| &s.sql).collect::<Vec<_>>());
    assert!(program.steps.iter().all(|s| s.reads.iter().all(|r| !program.relations.iter().any(|(rel, _)| r.ends_with(rel.as_str())))));
    let backing = Backing::default();
    let scope = backing.enter(&store);
    let lease = scope.pool().take().unwrap();
    // A file-local program of one step: one wave per chunk.
    let limits = Limits { max_waves: 1, ..limits };
    let started = std::time::Instant::now();
    let outcome = run(&lease, &store, &program, WS, documents.clone(), &limits, &mut |chunk| wave.demand(&store, WS, chunk)).unwrap();
    let elapsed = started.elapsed();
    assert_eq!(outcome.waves as usize, outcome.chunks);
    wave.statistics(&lease, WS).unwrap();

    let position = documents.iter().enumerate().map(|(at, d)| (i64::from(d.key), at)).collect::<BTreeMap<_, _>>();
    let mut matches = vec![vec![BTreeSet::new(); documents.len()]; clauses.len()];
    let rows: Vec<(i64, i64, i64)> = lease
        .prepare("SELECT c0, c1, c2 FROM match")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for (clause, d, node) in rows {
        matches[clause as usize][position[&d]].insert(node as u32);
    }
    let candidates = wave.chunks.iter().fold(vec![Vec::new(); clauses.len()], |mut all, chunk| {
        for (clause, files) in chunk.candidates.iter().enumerate() {
            all[clause].extend(files.iter().cloned());
        }
        all
    });

    let mut under = Vec::new();
    let mut differ = Vec::new();
    let (mut expected_nodes, mut candidate_nodes, mut all_nodes) = (0, 0, 0);
    for (at, document) in documents.iter().enumerate() {
        let tape = Families::stored(&store, WS, &document.path).unwrap().unwrap();
        all_nodes += tape.doc.transducer_len() * clauses.len();
        for (clause, grammar) in grammars.iter().enumerate() {
            let expected = oracle(grammar, &tape.doc);
            let selected = candidates[clause][at].iter().copied().collect::<BTreeSet<_>>();
            expected_nodes += expected.len();
            candidate_nodes += selected.len();
            if !expected.is_subset(&selected) {
                under.push(format!("{} in {}: {} roots not candidates", clauses[clause].name, document.path, expected.difference(&selected).count()));
            }
            if matches[clause][at] != expected {
                differ.push(format!(
                    "{} in {}: {} missing, {} extra",
                    clauses[clause].name,
                    document.path,
                    expected.difference(&matches[clause][at]).count(),
                    matches[clause][at].difference(&expected).count()
                ));
            }
        }
    }
    for name in clauses.iter().map(|clause| &clause.name) {
        let family = family_name(WS, PROGRAM, &format!("match:{name}"));
        assert!(store.derivation_valid(&family).unwrap(), "{family}");
    }
    let estimate: i64 = lease.query_row("SELECT rows FROM fact_stats WHERE tbl = 'rerail_terminal_match'", [], |row| row.get(0)).unwrap();
    eprintln!(
        "S4 wave: {} clauses ({} passes) on {} fixtures in {} chunks, {:?}: {expected_nodes} matched nodes; \
         candidates {candidate_nodes} of {all_nodes} clause-nodes; {} under-produced, {} differ; terminal estimate {estimate}",
        clauses.len(),
        wave.lowered().passes(),
        documents.len(),
        outcome.chunks,
        elapsed,
        under.len(),
        differ.len(),
    );
    for line in under.iter().chain(&differ).take(20) {
        eprintln!("  {line}");
    }
    assert!(under.is_empty() && differ.is_empty());
}

/// On Metal the wave is one evaluation: every clause's passes and root programs, from the same
/// prefilter candidates, in one command buffer, with W's matches.
#[cfg(all(target_os = "macos", feature = "metal-tests"))]
#[test]
fn the_wave_is_one_evaluation_on_metal() {
    let (store, paths) = ingested(15);
    let (clauses, grammars) = clauses();
    let wave = Wave::new(&clauses, ExecutionBackend::MetalBackground).unwrap();
    let tapes = paths.iter().map(|path| Families::stored(&store, WS, path).unwrap().unwrap()).collect::<Vec<_>>();
    let candidates = wave.candidates(&store, &tapes).unwrap();
    let docs = tapes.iter().map(|tape| tape.doc.clone()).collect::<Vec<_>>();
    let started = std::time::Instant::now();
    let (found, outcome) = wave.lowered().roots_metal(&docs, &candidates).unwrap();
    let elapsed = started.elapsed();
    let mut differ = Vec::new();
    let mut matched = 0;
    for (clause, grammar) in grammars.iter().enumerate() {
        for (at, doc) in docs.iter().enumerate() {
            let expected = oracle(grammar, doc);
            matched += expected.len();
            if found[clause][at] != expected {
                differ.push(format!("{} in {}", clauses[clause].name, paths[at]));
            }
        }
    }
    eprintln!(
        "S4 Metal: {} clauses on {} fixtures ({matched} matched nodes) in {elapsed:?}: {} command buffers, {} launches, \
         {} rounds at most; host waits for growth, reruns, uploads {:?}; {} differ",
        clauses.len(),
        docs.len(),
        outcome.command_buffers,
        outcome.dispatches,
        outcome.rounds,
        outcome.waits,
        differ.len(),
    );
    for line in differ.iter().take(10) {
        eprintln!("  {line}");
    }
    assert!(differ.is_empty());
    assert_eq!(outcome.command_buffers, 1, "one evaluation");
    assert!(outcome.dispatches < 100, "the launch budget's target");
}

/// The Python pack's compiled policies, by file stem (the clauses' `policy` prefix).
fn policies() -> Vec<(String, crate::code_rule_dsl::CompiledCodeRuleDsl)> {
    let mut documents = Vec::new();
    files(&pack().join("rules"), "json", &mut documents);
    files(&pack().join("candidates/rules"), "json", &mut documents);
    documents.sort();
    documents
        .into_iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            let compiled = CodeRuleDslRuntime::compile(text, "json".to_owned(), CAPABILITIES.to_owned()).ok()?;
            Some((path.file_stem().unwrap().to_string_lossy().into_owned(), compiled))
        })
        .collect()
}

/// 06 S5's gate: the findings layer over the wave's matches gives the keys of W's violations of the
/// same clauses on the same files, as sets, over every (rule, file); the (rule, file)s it leaves
/// `Unknown` are those where W's clause fails, and W reports none there either. W is evaluated one file at a
/// time (a file W cannot read fails its evaluation, which then reports no other file's violations).
#[test]
fn the_findings_are_ws_violations() {
    let (store, paths) = ingested(8);
    findings_gate(&store, &paths, &policies(), false);
}

/// The anchors, suppressions and report ceilings the fixtures do not reach, on sources written for
/// them: a format spec's backslashes (W605's regex group, one violation each), `noqa` comments
/// (codes, blanket, file exemptions, the enclosing statement's), `nosec`, `rerail-ignore`, and two
/// copies of E722 whose report ceilings lie on either side of their count. W reads the files in one
/// snapshot, so a ceiling counts over all of them.
#[test]
fn the_findings_follow_ws_anchors_suppressions_and_ceilings() {
    let sources = [
        "x = f\"{1:\\d}\"\ny = f\"{2:\\q\\w}\"\nz = f\"{3:>10}\"\nw = f\"{4:\\d}\"  # noqa: W605\n",
        "try:\n    pass\nexcept:  # noqa: E722\n    pass\ntry:\n    pass\nexcept:  # noqa\n    pass\ntry:\n    pass\nexcept:  # noqa: F401\n    pass\ntry:\n    pass\nexcept:\n    pass\n",
        "# ruff: noqa: E731\nf = lambda: 0\ng = lambda: 1\n",
        "f = lambda: 0  # noqa: E731\ng = (  # noqa: E731\n    lambda: 1\n)\nh = lambda: 2\nimport os, sys  # noqa: E401\nimport os, sys\n",
        "import pickle  # nosec\nimport subprocess  # nosec B404\nimport telnetlib\nassert True  # nosec B101\nassert True  # nosec B999\n",
        "try:\n    pass\nexcept:  # rerail-ignore ruff-e722-bare-except\n    pass\ntry:\n    pass\nexcept:  # rerail-ignore probe-e722-low -- reason\n    pass\n",
        "s = \"\"\"\nmultiline\n\"\"\"  # noqa: W605\nt = '\\d'\n",
        "k = lambda: (\n    1\n)\nprint(\n    1,\n    2)\nprint()\nm = map(lambda: 0, [])\n",
    ];
    let (store, paths) = ingested_sources(&sources);
    let mut policies = policies();
    let e722 = std::fs::read_to_string(pack().join("candidates/rules/ruff-e722-bare-except.json"))
        .or_else(|_| std::fs::read_to_string(pack().join("rules/ruff-e722-bare-except.json")))
        .unwrap();
    let e731 = std::fs::read_to_string(pack().join("candidates/rules/ruff-e731-lambda-assignment.json"))
        .or_else(|_| std::fs::read_to_string(pack().join("rules/ruff-e731-lambda-assignment.json")))
        .unwrap();
    let probe = |text: &str, id: &str, edit: &dyn Fn(&mut serde_json::Value)| {
        let mut document: serde_json::Value = serde_json::from_str(text).unwrap();
        document["id"] = serde_json::json!(id);
        edit(&mut document);
        let compiled = CodeRuleDslRuntime::compile(document.to_string(), "json".to_owned(), CAPABILITIES.to_owned()).unwrap();
        (id.to_owned(), compiled)
    };
    let ceiling = |gt: u64| {
        move |document: &mut serde_json::Value| {
            let clauses = document["clauses"].as_array_mut().unwrap();
            clauses.push(serde_json::json!({ "id": "report", "define": { "report_when": { "count": { "gt": gt } } } }));
        }
    };
    let parent = |document: &mut serde_json::Value| {
        for clause in document["clauses"].as_array_mut().unwrap() {
            if let Some(noqa) = clause.pointer_mut("/violation/suppress/noqa") {
                noqa["parent"] = serde_json::json!("statement");
            }
        }
    };
    policies.push(probe(&e722, "probe-e722-low", &ceiling(2)));
    policies.push(probe(&e722, "probe-e722-high", &ceiling(50)));
    policies.push(probe(&e731, "probe-e731-parent", &parent));
    // Metavariable anchors: a single capture, a multi-capture's first node, and one that may not bind.
    for (id, rule, anchor) in [
        ("probe-capture-value", serde_json::json!({ "pattern": "$F = lambda: $B" }), "capture.B"),
        ("probe-capture-args", serde_json::json!({ "pattern": "print($$$ARGS)" }), "capture.ARGS"),
        ("probe-capture-maybe", serde_json::json!({ "any": [{ "pattern": "$F = lambda: $B" }, { "kind": "lambda" }] }), "capture.B"),
    ] {
        let document = serde_json::json!({
            "format": "rerail.policy/v1", "id": id, "revision": 1, "mode": "block",
            "clauses": [{ "id": "c", "forbid": { "ast": { "language": "python", "rule": rule } },
                          "violation": { "anchor": anchor, "code": "X", "message": "m" } }],
        });
        policies.push(probe(&document.to_string(), id, &|_| {}));
    }
    let (clauses, ..) = shaped_from(&policies);
    for id in ["probe-capture-value", "probe-capture-args", "probe-capture-maybe"] {
        assert!(clauses.iter().any(|clause| clause.name.starts_with(id)), "{id} lowers");
    }
    findings_gate(&store, &paths, &policies, true);
}

/// Runs the wave and the findings layer over `paths` with the clauses of `policies`, and compares
/// the keys with W's violations (one snapshot of every file when `together`, else one per file).
fn findings_gate(store: &ContentStore, paths: &[String], policies: &[(String, crate::code_rule_dsl::CompiledCodeRuleDsl)], together: bool) {
    use serde_json::json;
    let (clauses, _, shapes) = shaped_from(policies);
    let documents = documents(store, WS, paths).unwrap();
    let mut wave = Wave::new(&clauses, ExecutionBackend::CpuJit).unwrap();
    let program = wave.program(store).unwrap();
    let backing = Backing::default();
    let scope = backing.enter(store);
    let lease = scope.pool().take().unwrap();
    let limits = Limits { max_waves: 1, ..Limits::default() };
    run(&lease, store, &program, WS, documents.clone(), &limits, &mut |chunk| wave.demand(store, WS, chunk)).unwrap();
    let layer = Layer::new(shapes).unwrap();
    let findings_program = layer.program(store).unwrap();
    assert_eq!(findings_program.program.steps.len(), 1, "the layer is one statement");
    let started = std::time::Instant::now();
    let (findings, outcome) = layer.findings(&lease, store, WS, &documents, &findings_program).unwrap();
    let elapsed = started.elapsed();
    let path = documents.iter().map(|d| (d.key, d.path.clone())).collect::<BTreeMap<_, _>>();
    let ours = findings.keys.iter().map(|k| (k.rule.clone(), k.code.clone(), path[&k.doc].clone(), k.line)).collect::<BTreeSet<_>>();

    let names = clauses.iter().map(|c| c.name.clone()).collect::<BTreeSet<_>>();
    let tapes = paths.iter().map(|path| Families::stored(store, WS, path).unwrap().unwrap()).collect::<Vec<_>>();
    let file = |path: &String, tape: &Arc<crate::code_rule_dsl::facts::families::Tape>| {
        json!({ "language": "python", "path": path, "source": ast_grep_core::Doc::get_source(&tape.doc) })
    };
    let snapshot = |files: Vec<serde_json::Value>| json!({ "facts": [], "files": files, "format": "rerail.repository-snapshot/v1", "revision": 0 }).to_string();
    let snapshots = if together {
        vec![snapshot(paths.iter().zip(&tapes).map(|(path, tape)| file(path, tape)).collect())]
    } else {
        paths.iter().zip(&tapes).map(|(path, tape)| snapshot(vec![file(path, tape)])).collect()
    };
    let complete = findings
        .coverage
        .iter()
        .filter(|(_, coverage)| **coverage == Coverage::Complete)
        .map(|((rule, doc), _)| (rule.clone(), path[doc].clone()))
        .collect::<BTreeSet<_>>();
    let mut ws = BTreeSet::new();
    let mut failed = BTreeSet::new();
    for (policy, compiled) in policies {
        for snapshot in &snapshots {
            let result: serde_json::Value = serde_json::from_str(&compiled.evaluate(snapshot.clone(), "report".to_owned()).unwrap()).unwrap();
            for v in result["violations"].as_array().unwrap() {
                let (rule, file) = (v["ruleId"].as_str().unwrap().to_owned(), v["path"].as_str().unwrap().to_owned());
                if names.contains(&format!("{policy}/{}", v["clauseId"].as_str().unwrap())) {
                    ws.insert((rule, v["code"].as_str().unwrap().to_owned(), file, v["span"]["startLine"].as_u64().unwrap() as usize));
                }
            }
            for f in result["failures"].as_array().unwrap() {
                if names.contains(&format!("{policy}/{}", f["clauseId"].as_str().unwrap_or_default())) {
                    failed.insert((compiled.inner.id.clone(), f["path"].as_str().unwrap_or_default().to_owned(), f["code"].as_str().unwrap_or_default().to_owned()));
                }
            }
        }
    }
    let mut unknown = BTreeMap::new();
    let mut unexplained = Vec::new();
    for ((rule, doc), coverage) in &findings.coverage {
        if let Coverage::Unknown(reason) = coverage {
            *unknown.entry(reason.clone()).or_insert(0) += 1;
            if !failed.contains(&(rule.clone(), path[doc].clone(), reason.clone())) {
                unexplained.push(format!("{rule} in {}: {reason}, W did not fail", path[doc]));
            }
        }
    }
    eprintln!(
        "S5 findings: {} clauses on {} files in {elapsed:?} ({} waves): {} keys, W {}; {} only ours, {} only W; \
         {} (rule, file)s complete, unknown {unknown:?} ({} where W does not fail); {} families changed",
        clauses.len(),
        documents.len(),
        outcome.waves,
        ours.len(),
        ws.len(),
        ours.difference(&ws).count(),
        ws.difference(&ours).count(),
        complete.len(),
        unexplained.len(),
        outcome.changed.len(),
    );
    let text = paths.iter().zip(&tapes).map(|(path, tape)| (path.clone(), ast_grep_core::Doc::get_source(&tape.doc).clone())).collect::<BTreeMap<_, _>>();
    for key in ours.symmetric_difference(&ws).take(20) {
        let line = text[&key.2].lines().nth(key.3).unwrap_or_default();
        eprintln!("  {} {key:?}: {}", if ours.contains(key) { "only ours" } else { "only W" }, line.trim());
    }
    for line in unexplained.iter().take(10) {
        eprintln!("  {line}");
    }
    assert_eq!(ours, ws);
    assert!(unexplained.is_empty());
    for rule in findings.coverage.keys().map(|(rule, _)| rule).collect::<BTreeSet<_>>() {
        let family = family_name(WS, FINDINGS, &format!("finding:{rule}"));
        assert!(store.derivation_valid(&family).unwrap(), "{family}");
    }
    // The same matches again: no finding family changes, so nothing downstream reruns.
    let (again, outcome) = layer.findings(&lease, store, WS, &documents, &findings_program).unwrap();
    assert_eq!(again.keys, findings.keys);
    assert!(outcome.changed.is_empty(), "{:?}", outcome.changed);
}
