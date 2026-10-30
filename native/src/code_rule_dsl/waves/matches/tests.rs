//! 06-first-slice S4's gate: the Python pack's lowered clauses as one wave over the pack's
//! fixtures, ingested through the push path (S3). Every clause's prefilter candidates contain the
//! nodes W's matcher matches at (the prefilter never under-produces), and its match facts are
//! exactly those nodes (S1's oracle). `RERAIL_S1_FILES` and `RERAIL_S1_FROM` pick the fixtures as
//! in S1 (default: the first 8 of every eighth), `RERAIL_S1_CLAUSE` the clauses.

use super::super::owner::Backing;
use super::super::schedule::{Limits, family_name, run};
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
    let mut documents = Vec::new();
    files(&pack().join("rules"), "json", &mut documents);
    files(&pack().join("candidates/rules"), "json", &mut documents);
    documents.sort();
    let mut clauses = Vec::new();
    let mut grammars = Vec::new();
    for path in documents {
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let Ok(compiled) = CodeRuleDslRuntime::compile(text, "json".to_owned(), CAPABILITIES.to_owned()) else { continue };
        let policy = path.file_stem().unwrap().to_string_lossy().into_owned();
        for clause in &compiled.inner.clauses {
            let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &clause.kind else { continue };
            for grammar in matchers.iter().filter(|matcher| matcher.language == GrammarDialect::Python) {
                let name = format!("{policy}/{}", clause.id);
                if std::env::var("RERAIL_S1_CLAUSE").is_ok_and(|part| !name.contains(&part)) {
                    continue;
                }
                let Ok(tree) = match_tree(grammar) else { continue };
                let Ok(matcher) = LoweredMatcher::lower(&tree) else { continue };
                clauses.push(Clause { name, plan: Arc::clone(&grammar.anchors), matcher });
                grammars.push(grammar.clone());
            }
        }
    }
    (clauses, grammars)
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
    let store = ContentStore::open(&temp("store").with_extension("sqlite")).unwrap();
    let root = directory_root(&store, &dir).unwrap();
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
    let program = wave.program();
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
