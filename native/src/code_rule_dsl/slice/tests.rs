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

/// S7's sample, copied into `dir`. It holds every third fixture of the pack (`RERAIL_S7_FIXTURES`,
/// default 150) as `f<n>.py`, and an evenly spaced slice of a real repository as `r<n>.py`:
/// `RERAIL_S7_FILES` (default 100) of the `.py` files under `RERAIL_S7_REPO`, test directories
/// left out. The default repository is `python3`'s standard library (CPython, PSF licence), read in
/// place and never vendored. Returns the paths and where the repository slice came from.
fn sample(dir: &Path) -> (Vec<String>, String) {
    let number = |name: &str, default: usize| std::env::var(name).ok().and_then(|n| n.parse().ok()).unwrap_or(default);
    std::fs::create_dir_all(dir).unwrap();
    let mut sources = Vec::new();
    files(&pack().join("upstream"), "py", &mut sources);
    sources.sort();
    let mut paths = Vec::new();
    for (n, source) in sources.iter().step_by(3).take(number("RERAIL_S7_FIXTURES", 150)).enumerate() {
        let path = format!("f{n:03}.py");
        std::fs::copy(source, dir.join(&path)).unwrap();
        paths.push(path);
    }
    let repo = std::env::var("RERAIL_S7_REPO").unwrap_or_else(|_| {
        let out = std::process::Command::new("python3")
            .args(["-c", "import sysconfig; print(sysconfig.get_paths()['stdlib'])"])
            .output()
            .expect("python3 for S7's default repository (or set RERAIL_S7_REPO)");
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    });
    let mut found = Vec::new();
    files(Path::new(&repo), "py", &mut found);
    found.retain(|path| {
        !path.strip_prefix(&repo).unwrap_or(path).components().any(|part| {
            let part = part.as_os_str().to_string_lossy();
            part == "test" || part == "tests" || part == "site-packages" || part.starts_with("idle_test")
        })
    });
    found.sort();
    let most = number("RERAIL_S7_FILES", 100);
    assert!(found.len() >= most, "{repo} has {} Python files, fewer than {most}", found.len());
    let step = found.len() / most;
    let mut n = 0;
    for source in found.iter().step_by(step) {
        // W reads a snapshot's sources as JSON strings, so a file must be UTF-8.
        let Ok(text) = std::fs::read_to_string(source) else { continue };
        if n == most {
            break;
        }
        let path = format!("r{n:03}.py");
        std::fs::write(dir.join(&path), text).unwrap();
        paths.push(path);
        n += 1;
    }
    (paths, repo)
}

/// The keys of W's violations of the clauses `names` of `policies` on `files` (path, source), each
/// file its own snapshot as in S5's gate, with the (rule, file, code)s where W's clause fails.
fn ws_keys(
    policies: &[(String, CompiledCodeRuleDsl)],
    names: &BTreeSet<String>,
    files: &[(String, String)],
) -> (Keys, BTreeSet<(String, String, String)>) {
    use rayon::prelude::*;
    use serde_json::json;
    let policies = policies
        .iter()
        .filter(|(policy, compiled)| compiled.inner.clauses.iter().any(|c| names.contains(&format!("{policy}/{}", c.id))))
        .collect::<Vec<_>>();
    let per_file = files
        .par_iter()
        .map(|(path, source)| {
            let snapshot = json!({
                "facts": [], "format": "rerail.repository-snapshot/v1", "revision": 0,
                "files": [{ "language": "python", "path": path, "source": source }],
            })
            .to_string();
            let (mut keys, mut failed) = (Keys::new(), BTreeSet::new());
            for (policy, compiled) in &policies {
                let result: serde_json::Value = serde_json::from_str(&compiled.evaluate(snapshot.clone(), "report".to_owned()).unwrap()).unwrap();
                for v in result["violations"].as_array().unwrap() {
                    if names.contains(&format!("{policy}/{}", v["clauseId"].as_str().unwrap())) {
                        keys.insert((
                            v["ruleId"].as_str().unwrap().to_owned(),
                            v["code"].as_str().unwrap().to_owned(),
                            v["path"].as_str().unwrap().to_owned(),
                            v["span"]["startLine"].as_u64().unwrap() as usize,
                        ));
                    }
                }
                for f in result["failures"].as_array().unwrap() {
                    if names.contains(&format!("{policy}/{}", f["clauseId"].as_str().unwrap_or_default())) {
                        failed.insert((
                            compiled.inner.id.clone(),
                            f["path"].as_str().unwrap_or_default().to_owned(),
                            f["code"].as_str().unwrap_or_default().to_owned(),
                        ));
                    }
                }
            }
            (keys, failed)
        })
        .collect::<Vec<_>>();
    per_file.into_iter().fold((Keys::new(), BTreeSet::new()), |(mut keys, mut failed), (k, f)| {
        keys.extend(k);
        failed.extend(f);
        (keys, failed)
    })
}

/// How a run's findings differ from W's: the (rule, code)s whose key sets differ, with how many
/// keys only the run has and only W has; and the (rule, file)s the run leaves `Unknown` where W's
/// clause does not fail.
struct Difference {
    clauses: BTreeMap<(String, String), (usize, usize)>,
    unexplained: Vec<String>,
}

impl Difference {
    /// Whether the run differs from W beyond `KNOWN`.
    fn beyond_known(&self) -> bool {
        !self.unexplained.is_empty() || self.clauses.keys().any(|(rule, code)| !KNOWN.iter().any(|known| known.0 == rule && known.1 == code))
    }

    fn of(run: &Run, ws: &Keys, failed: &BTreeSet<(String, String, String)>) -> Self {
        let ours = keys(run);
        let mut clauses = BTreeMap::<_, (usize, usize)>::new();
        for key in ours.difference(ws) {
            clauses.entry((key.0.clone(), key.1.clone())).or_default().0 += 1;
        }
        for key in ws.difference(&ours) {
            clauses.entry((key.0.clone(), key.1.clone())).or_default().1 += 1;
        }
        let unexplained = coverage(run)
            .into_iter()
            .filter_map(|((rule, path), coverage)| match coverage {
                Coverage::Unknown(reason) if !failed.contains(&(rule.clone(), path.clone(), reason.clone())) => {
                    Some(format!("{rule} in {path}: {reason}, W did not fail"))
                }
                _ => None,
            })
            .collect();
        Self { clauses, unexplained }
    }

}

/// The Python AST `forbid` clauses of `policies`: how many there are (S1 counts them the same way).
fn python_clauses(policies: &[(String, CompiledCodeRuleDsl)]) -> usize {
    policies
        .iter()
        .flat_map(|(_, compiled)| compiled.inner.clauses.iter())
        .filter_map(|clause| match &clause.kind {
            crate::code_rule_dsl::model::ClauseKind::Forbid(crate::code_rule_dsl::model::Query::Ast { matchers, .. }) => Some(matchers),
            _ => None,
        })
        .map(|matchers| matchers.iter().filter(|m| m.language == GrammarDialect::Python).count())
        .sum()
}

/// `policies` with the first `kind` of `id`'s AST rules made `pass_statement` (or
/// `break_statement`, where it already is): a broken rule, for S7's mutation check.
fn broken(id: &str) -> Option<(String, CompiledCodeRuleDsl)> {
    fn first_kind(value: &mut serde_json::Value) -> bool {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(serde_json::Value::String(kind)) = map.get_mut("kind") {
                    *kind = if kind == "pass_statement" { "break_statement" } else { "pass_statement" }.to_owned();
                    return true;
                }
                map.values_mut().any(first_kind)
            }
            serde_json::Value::Array(items) => items.iter_mut().any(first_kind),
            _ => false,
        }
    }
    let text = std::fs::read_to_string(pack().join(format!("rules/{id}.json")))
        .or_else(|_| std::fs::read_to_string(pack().join(format!("candidates/rules/{id}.json"))))
        .ok()?;
    let mut document: serde_json::Value = serde_json::from_str(&text).ok()?;
    let changed = document["clauses"].as_array_mut()?.iter_mut().any(|clause| clause.pointer_mut("/forbid/ast").is_some_and(first_kind));
    if !changed {
        return None;
    }
    let compiled = CodeRuleDslRuntime::compile(document.to_string(), "json".to_owned(), CAPABILITIES.to_owned()).ok()?;
    Some((id.to_owned(), compiled))
}

/// The (rule, code)s whose keys S7's sample shows differ from W's, each with its open item and an
/// ignored test of its own: the gate requires every other (rule, code) to be equal.
const KNOWN: [(&str, &str, &str); 1] = [(
    "ruff-sim300-yoda-conditions",
    "SIM300",
    "O29: the recursive label `probably` settles one level deep: `X == 1 + 2 + 3`, `X == [[1]]`",
)];

/// 06 S7's gate. On the sample (`sample`), the slice's cold audit (`audit`, every Python clause
/// S1/S2 lowers, on the CPU, and on Metal with `RERAIL_S7_BACKENDS`) gives the same finding keys as W's engine for
/// every (rule, code). Every (rule, file) it leaves `Unknown` is one where W's clause fails. It
/// reports the coverage of the pack's Python clauses and the answer count beside each time.
///
/// The mutation check (spike 2.17) shows the gate can fail. Moving one kept span's start back one
/// byte, onto the previous line, and re-running the layer over the kept rows (`apply` on the same
/// root) differs from W. So does one rule broken (`broken`).
#[test]
fn the_slice_finds_ws_findings_on_real_files() {
    let policies = policies();
    let dir = temp("s7");
    let (paths, repo) = sample(&dir);
    let sources = paths.iter().map(|p| (p.clone(), std::fs::read_to_string(dir.join(p)).unwrap())).collect::<Vec<_>>();
    let bytes = sources.iter().map(|(_, s)| s.len()).sum::<usize>();
    let (clauses, _) = lowered(&policies);
    let names = clauses.iter().map(|c| c.name.clone()).collect::<BTreeSet<_>>();
    let total = python_clauses(&policies);

    let (cpu0, wall0) = (cpu(), std::time::Instant::now());
    let (ws, failed) = ws_keys(&policies, &names, &sources);
    let w_time = (cpu() - cpu0, wall0.elapsed());

    // `RERAIL_S7_BACKENDS`: the backends, of `cpu` and (on macOS) `metal`; default `cpu`, which
    // keeps the gate within a minute (Metal adds 15 s and agrees; decision 0017).
    let wanted = std::env::var("RERAIL_S7_BACKENDS").unwrap_or_else(|_| "cpu".to_owned());
    let mut backends = Vec::new();
    if wanted.contains("cpu") {
        backends.push(("cpu", ExecutionBackend::CpuJit));
    }
    #[cfg(target_os = "macos")]
    if wanted.contains("metal") {
        backends.push(("metal", ExecutionBackend::MetalJit));
    }
    let mut cpu_store = None;
    for (name, backend) in backends {
        let store = ContentStore::open(&temp(&format!("s7-{name}")).with_extension("sqlite")).unwrap();
        let mut rules = Rules::lower(&policies, backend).unwrap();
        let (cpu0, wall0) = (cpu(), std::time::Instant::now());
        let root = directory_root(&store, &dir).unwrap();
        let run = audit(&store, WS, root, &mut rules).unwrap();
        let (spent, wall) = (cpu() - cpu0, wall0.elapsed());
        crate::code_rule_dsl::prof::report(name);
        let difference = Difference::of(&run, &ws, &failed);
        let ours = keys(&run);
        let exercised = ours.iter().map(|k| (&k.0, &k.1)).collect::<BTreeSet<_>>();
        let unknown = coverage(&run).values().filter(|c| matches!(c, Coverage::Unknown(_))).count();
        eprintln!(
            "S7 {name}: {} files ({} fixtures, {} from {repo}; {} KB); {} of {total} Python AST clauses lowered, \
             {} (rule, code)s with findings; {} keys (W {}) in cpu {spent:?}, wall {wall:?}; W {} keys in cpu {:?}, wall {:?}; \
             {} (rule, code)s differ; {} (rule, file)s Unknown, {} where W does not fail",
            run.paths.len(),
            paths.iter().filter(|p| p.starts_with('f')).count(),
            paths.iter().filter(|p| p.starts_with('r')).count(),
            bytes >> 10,
            rules.clauses(),
            exercised.len(),
            ours.len(),
            ws.len(),
            ws.len(),
            w_time.0,
            w_time.1,
            difference.clauses.len(),
            unknown,
            difference.unexplained.len(),
        );
        for ((rule, code), (only_ours, only_ws)) in difference.clauses.iter().take(20) {
            eprintln!("  {rule} {code}: {only_ours} only ours, {only_ws} only W");
        }
        let text = sources.iter().map(|(path, source)| (path.as_str(), source.as_str())).collect::<BTreeMap<_, _>>();
        for key in ours.symmetric_difference(&ws).take(20) {
            let line = text[key.2.as_str()].lines().nth(key.3).unwrap_or_default();
            eprintln!("    {} {key:?}: {}", if ours.contains(key) { "only ours" } else { "only W" }, line.trim());
        }
        for line in difference.unexplained.iter().take(10) {
            eprintln!("  {line}");
        }
        let known = KNOWN.iter().map(|(rule, code, _)| (rule.to_string(), code.to_string())).collect::<BTreeSet<_>>();
        let unknown_differences = difference.clauses.keys().filter(|clause| !known.contains(*clause)).collect::<Vec<_>>();
        assert!(unknown_differences.is_empty() && difference.unexplained.is_empty(), "{name}: the slice's findings are W's but {KNOWN:?}");
        for (rule, code, item) in KNOWN {
            if !difference.clauses.contains_key(&(rule.to_owned(), code.to_owned())) {
                eprintln!("  {rule} {code} agrees on this sample ({item}); a_recursive_label_settles_at_every_depth tells whether it is fixed");
            }
        }
        if name == "cpu" {
            cpu_store = Some((store, rules, run, root));
        }
    }

    // Mutation: one kept span's start one byte earlier, onto the line before its finding's. A key is
    // a line, so the span is an anchor at a line's first byte (the first such finding's).
    let Some((store, mut rules, run, root)) = cpu_store else { return };
    let text = sources.iter().cloned().collect::<BTreeMap<_, _>>();
    let line_start = |path: &str, line: usize| -> Option<usize> {
        let source = &text[path];
        let mut at = 0;
        for _ in 0..line {
            at += source[at..].find('\n')? + 1;
        }
        Some(at)
    };
    let doc = run.paths.iter().map(|(d, p)| (p.clone(), *d)).collect::<BTreeMap<_, _>>();
    let shifted = {
        let entered = rules.backing.enter(&store);
        let lease = entered.pool().take().unwrap();
        keys(&run).iter().filter(|k| k.3 > 0).find_map(|(_, _, path, line)| {
            let start = line_start(path, *line).filter(|&s| s > 0)?;
            let moved = lease
                .execute("UPDATE slice_span SET c2 = c2 - 1 WHERE scope = ?1 AND c0 = ?2 AND c2 = ?3", rusqlite::params![WS, doc[path], start as i64])
                .unwrap();
            (moved > 0).then(|| format!("{path} byte {start} ({moved} spans)"))
        })
    };
    let shifted = shifted.expect("a finding whose anchor starts its line");
    let mutated = apply(&store, WS, root, &mut rules).unwrap();
    assert!(mutated.evaluated.is_empty(), "the layer runs over the kept, shifted spans");
    let span_difference = Difference::of(&mutated, &ws, &failed);
    eprintln!("S7 mutation, span at {shifted}: {} (rule, code)s differ", span_difference.clauses.len());
    assert!(span_difference.beyond_known(), "a span one byte off fails the gate");

    // Mutation: one rule with findings broken.
    let (id, broken) = keys(&run)
        .iter()
        .map(|k| k.0.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .find_map(|id| broken(&id).map(|b| (id, b)))
        .expect("a rule with findings whose AST rule names a kind");
    let mut broken = Some(broken);
    let mutated_policies = self::policies().into_iter().map(|policy| if policy.0 == id { broken.take().unwrap() } else { policy }).collect::<Vec<_>>();
    // On the files with the rule's findings, so the mutation costs a fraction of the audit.
    let with = keys(&run).into_iter().filter(|k| k.0 == id).map(|k| k.2).collect::<BTreeSet<_>>();
    let subset = temp("s7-broken-files");
    std::fs::create_dir_all(&subset).unwrap();
    for path in &with {
        std::fs::write(subset.join(path), &text[path]).unwrap();
    }
    let ws_subset = ws.iter().filter(|k| with.contains(&k.2)).cloned().collect::<Keys>();
    let store = ContentStore::open(&temp("s7-broken").with_extension("sqlite")).unwrap();
    let mut rules = Rules::lower(&mutated_policies, ExecutionBackend::CpuJit).unwrap();
    let root = directory_root(&store, &subset).unwrap();
    let broken_run = audit(&store, WS, root, &mut rules).unwrap();
    let rule_difference = Difference::of(&broken_run, &ws_subset, &failed);
    eprintln!("S7 mutation, {id} broken, on its {} files: {} (rule, code)s differ", with.len(), rule_difference.clauses.len());
    assert!(rule_difference.beyond_known(), "a broken rule fails the gate");
}

/// O29's case on its own: a recursive label through `not has not` (SIM300's `probably`) at two
/// levels of the same construct. W finds no yoda condition in any of these; the lowered clause
/// finds one in each, on the CPU and on Metal alike, so the plan, not a backend, is at fault.
#[test]
#[ignore = "O29: the recursive label settles one level deep; S7's gate lists SIM300 in KNOWN"]
fn a_recursive_label_settles_at_every_depth() {
    let policies = policies();
    let dir = temp("o29");
    std::fs::create_dir_all(&dir).unwrap();
    let source = "X == 4 * 365 + 1\nX == [[1]]\nX == 1 + 2 + 3\nX == (4 + 1)\n4 * 365 + 1 == x\n";
    write(&dir, "a.py", source);
    let (clauses, _) = lowered(&policies);
    let names = clauses.iter().map(|c| c.name.clone()).collect::<BTreeSet<_>>();
    let (ws, failed) = ws_keys(&policies, &names, &[("a.py".to_owned(), source.to_owned())]);
    let run = cold(&dir, &policies);
    let difference = Difference::of(&run, &ws, &failed);
    assert!(difference.clauses.is_empty(), "{:?}", difference.clauses);
}
