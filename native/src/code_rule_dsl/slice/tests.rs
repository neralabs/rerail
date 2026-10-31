//! 06-first-slice S6's check: an edit to one file re-runs exactly that file's derivations and its
//! clauses' matches, and its findings change only for that file; a whitespace-only edit cuts off at
//! the tape or at the matches. Every warm run's findings equal a cold audit of the same snapshot in
//! a fresh store.

use super::*;
use crate::code_rule_dsl::CodeRuleDslRuntime;
use crate::code_rule_dsl::facts::observe::roots::directory_root;
use crate::code_rule_dsl::findings::contract::Coverage;
use crate::code_rule_dsl::test_support::CAPABILITIES;
use std::path::{Path, PathBuf};

const WS: &str = "/slice";

fn pack() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../packs/python")
}

fn temp(what: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("rerail-s6-{what}-{}-{nonce}", std::process::id()))
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

/// The Python pack's compiled policies, by file stem.
fn policies() -> Vec<(String, CompiledCodeRuleDsl)> {
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

/// The keys of a run, by path rather than document key, and its coverage likewise.
type Keys = BTreeSet<(String, String, String, usize)>;
fn keys(run: &Run) -> Keys {
    run.findings.keys.iter().map(|k| (k.rule.clone(), k.code.clone(), run.paths[&k.doc].clone(), k.line)).collect()
}
fn coverage(run: &Run) -> BTreeMap<(String, String), Coverage> {
    run.findings.coverage.iter().map(|((rule, doc), c)| ((rule.clone(), run.paths[doc].clone()), c.clone())).collect()
}

/// A cold audit of `dir` in a fresh store: what a warm run over the same snapshot must find.
fn cold(dir: &Path, policies: &[(String, CompiledCodeRuleDsl)]) -> Run {
    let store = ContentStore::open(&temp("cold").with_extension("sqlite")).unwrap();
    let mut rules = Rules::lower(policies, ExecutionBackend::CpuJit).unwrap();
    let root = directory_root(&store, dir).unwrap();
    audit(&store, WS, root, &mut rules).unwrap()
}

fn write(dir: &Path, path: &str, source: &str) {
    std::fs::write(dir.join(path), source).unwrap();
}

#[test]
fn an_edit_reruns_its_file_and_a_whitespace_edit_cuts_off() {
    let policies = policies();
    let dir = temp("sources");
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir, "a.py", "import os, sys\nf = lambda: 0\n");
    write(&dir, "b.py", "try:\n    pass\nexcept:\n    pass\n");
    write(&dir, "c.py", "x = 1\n");
    write(&dir, "notes.txt", "not code\n");
    let store = ContentStore::open(&temp("store").with_extension("sqlite")).unwrap();
    let mut rules = Rules::lower(&policies, ExecutionBackend::CpuJit).unwrap();
    assert!(rules.clauses() > 80, "the pack's clauses lower: {}", rules.clauses());
    let same = |run: &Run, what: &str| {
        let expected = cold(&dir, &policies);
        assert_eq!(keys(run), keys(&expected), "{what}: the warm keys are a cold audit's");
        assert_eq!(coverage(run), coverage(&expected), "{what}: and so is the coverage");
    };
    let by = |run: &Run, path: &str| keys(run).into_iter().filter(|k| k.2 == path).collect::<Keys>();

    // Cold: every document evaluated.
    let root = directory_root(&store, &dir).unwrap();
    let first = audit(&store, WS, root, &mut rules).unwrap();
    assert_eq!(first.evaluated, ["a.py", "b.py", "c.py"]);
    assert!(!keys(&first).is_empty(), "the sources violate the pack");
    assert!(keys(&first).iter().all(|k| k.2 != "c.py"), "c.py is clean");
    same(&first, "cold");

    // The same snapshot: nothing re-runs, nothing changes.
    let again = apply(&store, WS, root, &mut rules).unwrap();
    assert_eq!(again.ingested.push.rebound, 0);
    assert!(again.evaluated.is_empty() && again.matches.is_empty() && again.changed.is_empty());
    assert_eq!(keys(&again), keys(&first));

    // One file edited: its four derivations re-run and only it is evaluated; only its findings move.
    write(&dir, "c.py", "x = 1\ng = lambda: 1\n");
    let edited = apply(&store, WS, directory_root(&store, &dir).unwrap(), &mut rules).unwrap();
    assert_eq!(edited.ingested.push.work.rerun, 4, "c.py's tape, core, prefilter and kinds");
    assert_eq!(edited.evaluated, ["c.py"]);
    assert!(!by(&edited, "c.py").is_empty(), "the edit adds a finding");
    for path in ["a.py", "b.py"] {
        assert_eq!(by(&edited, path), by(&first, path), "{path}'s findings are unchanged");
    }
    assert!(!edited.matches.is_empty() && !edited.changed.is_empty());
    same(&edited, "edit");

    // Whitespace only: a trailing space changes no node, so the matches cut off.
    write(&dir, "a.py", "import os, sys \nf = lambda: 0\n");
    let spaced = apply(&store, WS, directory_root(&store, &dir).unwrap(), &mut rules).unwrap();
    assert!(spaced.evaluated.len() <= 1, "{:?}", spaced.evaluated);
    assert!(spaced.matches.iter().all(|family| !family.contains(":match:")), "no clause's matches change: {:?}", spaced.matches);
    assert!(spaced.changed.is_empty(), "nor do the findings: {:?}", spaced.changed);
    assert_eq!(keys(&spaced), keys(&edited));
    same(&spaced, "whitespace");

    // A file removed: its findings go, and nothing is evaluated.
    std::fs::remove_file(dir.join("b.py")).unwrap();
    let removed = apply(&store, WS, directory_root(&store, &dir).unwrap(), &mut rules).unwrap();
    assert!(removed.evaluated.is_empty());
    assert!(by(&removed, "b.py").is_empty() && !by(&spaced, "b.py").is_empty());
    same(&removed, "removal");
    eprintln!(
        "S6: {} clauses; cold {} keys; edit {} rerun, {} match families and {} findings families changed; whitespace {:?} evaluated, {:?} changed",
        rules.clauses(),
        keys(&first).len(),
        edited.ingested.push.work.rerun,
        edited.matches.len(),
        edited.changed.len(),
        spaced.evaluated,
        spaced.matches,
    );
}

/// The user and system CPU time this process has spent.
fn cpu() -> std::time::Duration {
    let mut usage = unsafe { std::mem::zeroed::<libc::rusage>() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    let time = |t: libc::timeval| std::time::Duration::new(t.tv_sec as u64, t.tv_usec as u32 * 1000);
    time(usage.ru_utime) + time(usage.ru_stime)
}

/// The slice's bench (06 S6; S8 measures on S7's sample): a cold audit of `RERAIL_S6_FILES`
/// fixtures (default 48), then a warm one-file edit and a warm whitespace-only edit, each with its
/// CPU time (user + sys) and wall time. `RERAIL_S6_BACKEND=metal` runs the wave on Metal. Build with
/// `--profile profiling`: an unoptimized build's host code is many times slower.
#[test]
#[ignore = "the slice's bench; run with --ignored --nocapture"]
fn slice_bench() {
    let mut sources = Vec::new();
    files(&pack().join("upstream"), "py", &mut sources);
    sources.sort();
    let most = std::env::var("RERAIL_S6_FILES").ok().and_then(|n| n.parse().ok()).unwrap_or(48);
    let dir = temp("bench");
    std::fs::create_dir_all(&dir).unwrap();
    let paths = sources
        .iter()
        .step_by(8)
        .take(most)
        .enumerate()
        .map(|(n, source)| {
            let path = format!("f{n:03}.py");
            std::fs::copy(source, dir.join(&path)).unwrap();
            path
        })
        .collect::<Vec<_>>();
    let backend = match std::env::var("RERAIL_S6_BACKEND").as_deref() {
        #[cfg(target_os = "macos")]
        Ok("metal") => ExecutionBackend::MetalJit,
        _ => ExecutionBackend::CpuJit,
    };
    let policies = policies();
    let store = ContentStore::open(&temp("bench-store").with_extension("sqlite")).unwrap();
    let mut rules = Rules::lower(&policies, backend).unwrap();
    let measure = |what: &str, rules: &mut Rules, cold: bool| {
        let (cpu0, wall0) = (cpu(), std::time::Instant::now());
        let root = directory_root(&store, &dir).unwrap();
        let run = if cold { audit(&store, WS, root, rules) } else { apply(&store, WS, root, rules) }.unwrap();
        eprintln!(
            "S6 bench {what}: {} files, {} keys, {} evaluated, {} re-run; cpu {:?}, wall {:?}",
            run.paths.len(),
            run.findings.keys.len(),
            run.evaluated.len(),
            run.ingested.push.work.rerun,
            cpu() - cpu0,
            wall0.elapsed()
        );
        crate::code_rule_dsl::prof::report(what);
    };
    measure("cold", &mut rules, true);
    let first = dir.join(&paths[0]);
    let source = std::fs::read_to_string(&first).unwrap();
    std::fs::write(&first, format!("{source}\ng = lambda: 1\n")).unwrap();
    measure("one-file edit", &mut rules, false);
    std::fs::write(&first, format!("{source}\ng = lambda: 1 \n")).unwrap();
    measure("whitespace edit", &mut rules, false);
}
