//! 06-first-slice S6's check: an edit to one file re-runs exactly that file's derivations and its
//! clauses' matches, and its findings change only for that file; a whitespace-only edit cuts off at
//! the tape or at the matches. Every warm run's findings equal a cold audit of the same snapshot in
//! a fresh store.

use super::*;
use crate::code_rule_dsl::CodeRuleDslRuntime;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::model::{ClauseKind, Query};
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
            // A policy that does not compile is left out, and named.
            let compiled = CodeRuleDslRuntime::compile(text, "json".to_owned(), CAPABILITIES.to_owned())
                .map_err(|error| eprintln!("SKIP {}: {}", path.display(), error.reason))
                .ok()?;
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
    // A module docstring: the pack's D100 flags a public module without one.
    write(&dir, "c.py", "\"\"\"The c module.\"\"\"\nx = 1\n");
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
    write(&dir, "c.py", "\"\"\"The c module.\"\"\"\nx = 1\ng = lambda: 1\n");
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
/// (O29, SIM300, was the one entry until the sweeps set and cleared their verdicts.)
const KNOWN: [(&str, &str, &str); 0] = [];

/// The packs' census (decision 0028): over every pack's rules and candidates, how many `forbid`
/// `ast` clauses lower (any language), and of their programs (root and label) how many are micro
/// walks, the device's shape (0024), against those that need a general executor.
#[test]
#[ignore = "the packs' census; run with --ignored --nocapture"]
fn pack_census() {
    use crate::code_rule_dsl::matcher::match_tree;
    use crate::code_rule_dsl::transducer::packed::{LoweredMatcher, Micro};
    let packs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../packs");
    let mut names = std::fs::read_dir(&packs).unwrap().flatten().filter(|e| e.path().is_dir()).map(|e| e.path()).collect::<Vec<_>>();
    names.sort();
    // The data-layer pack, the architecture's hardest test, lives in its own worktree (read only).
    let data_layer = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Data-layer-rule-expressivity/packs/data-layer");
    if data_layer.is_dir() {
        names.push(data_layer);
    }
    let mut total = BTreeMap::<String, [usize; 2]>::new();
    for dir in names {
        let pack = dir.file_name().unwrap().to_string_lossy().into_owned();
        let mut documents = Vec::new();
        files(&dir.join("rules"), "json", &mut documents);
        files(&dir.join("candidates/rules"), "json", &mut documents);
        let (mut rules, mut refused, mut clauses, mut unlowered) = (0, 0, 0, 0);
        let mut by = BTreeMap::<String, [usize; 2]>::new();
        let mut examples = Vec::new();
        let mut reasons = Vec::new();
        for path in documents {
            let text = std::fs::read_to_string(&path).unwrap();
            let compiled = match CodeRuleDslRuntime::compile(text, "json".to_owned(), CAPABILITIES.to_owned()) {
                Ok(compiled) => compiled,
                Err(error) => {
                    refused += 1;
                    if reasons.len() < 3 {
                        reasons.push(error.reason.chars().take(160).collect::<String>());
                    }
                    continue;
                }
            };
            rules += 1;
            for clause in &compiled.inner.clauses {
                let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &clause.kind else { continue };
                for grammar in matchers.iter() {
                    clauses += 1;
                    let Some(matcher) = match_tree(grammar).ok().and_then(|tree| LoweredMatcher::lower(&tree).ok()) else {
                        unlowered += 1;
                        continue;
                    };
                    for (kind, program) in matcher.census() {
                        let micro = Micro::of(program).is_some();
                        let entry = by.entry(format!("{kind}/{}", crate::code_rule_dsl::language::label(grammar.language).unwrap_or("?"))).or_default();
                        entry[usize::from(!micro)] += 1;
                        total.entry(kind.to_owned()).or_default()[usize::from(!micro)] += 1;
                        if !micro && kind != "pattern" && kind != "constrained" && examples.len() < 3 {
                            examples.push(format!("{}#{} {kind}", path.file_stem().unwrap().to_string_lossy(), clause.id));
                        }
                    }
                }
            }
        }
        eprintln!("census {pack}: {rules} rules ({refused} refused), {clauses} forbid-ast matchers, {unlowered} not lowered; [micro, general] by label kind/language: {by:?}; general examples {examples:?}; refusals {reasons:?}");
    }
    eprintln!("census total [micro, general] by kind: {total:?}");
}

/// A rule naming a grammar supertype (O17: N's semantics) lowers to `SUPERTYPE` over the tape's
/// memberships, and every backend finds the same members: the two calls, `expression`s holding an
/// identifier (the assigned `x` is a `pattern`, and no identifier holds one).
#[test]
fn a_supertype_rule_finds_the_same_members_on_every_backend() {
    let policy = |rule: serde_json::Value| {
        let document = serde_json::json!({
            "format": "rerail.policy/v1", "id": "supertype", "revision": 1, "mode": "block",
            "clauses": [{ "id": "c", "forbid": { "ast": { "language": "python", "rule": rule } },
                          "violation": { "anchor": "witness.span", "code": "X", "message": "m" } }]
        });
        ("supertype".to_owned(), CodeRuleDslRuntime::compile(document.to_string(), "json".to_owned(), CAPABILITIES.to_owned()).unwrap_or_else(|error| panic!("{}", error.reason)))
    };
    let policies = vec![policy(serde_json::json!({ "kind": "expression", "has": { "kind": "identifier" } }))];
    let (clauses, _) = lowered(&policies).unwrap();
    assert_eq!(clauses.len(), 1, "the supertype clause lowers");
    let dir = temp("supertype");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "x = f(y)\nreturn g(x)\n").unwrap();
    let mut backends = vec![ExecutionBackend::CpuInterpreter, ExecutionBackend::CpuJit];
    #[cfg(target_os = "macos")]
    backends.extend([ExecutionBackend::MetalInterpreter, ExecutionBackend::MetalJit]);
    let mut reference = None;
    for backend in backends {
        let store = ContentStore::open(&temp("supertype-store").with_extension("sqlite")).unwrap();
        let mut rules = Rules::lower(&policies, backend).unwrap();
        let root = directory_root(&store, &dir).unwrap();
        let found = keys(&audit(&store, WS, root, &mut rules).unwrap());
        let lines = found.iter().map(|key| key.3).collect::<Vec<_>>();
        match &reference {
            None => {
                assert_eq!(lines, [0, 1], "{backend:?}: `f(y)` (line 0) and `g(x)` (line 1)");
                reference = Some(found);
            }
            Some(first) => assert_eq!(&found, first, "{backend:?}"),
        }
    }
}

/// Phase 4's check (list step 19): the interpreter, the JIT and (on macOS) Metal's interpreter and
/// compiled kernel find the same finding keys and coverage over S7's sample of real files
/// (`RERAIL_S7_FIXTURES`, default 150), each cold in its own store.
#[test]
fn the_backends_find_the_same_keys_on_real_files() {
    let policies = policies();
    let dir = temp("phase4");
    let (paths, _) = sample(&dir);
    assert!((100..=300).contains(&paths.len()) || std::env::var_os("RERAIL_S7_FIXTURES").is_some(), "{} files", paths.len());
    let mut backends = vec![("interpreter", ExecutionBackend::CpuInterpreter), ("jit", ExecutionBackend::CpuJit)];
    #[cfg(target_os = "macos")]
    backends.extend([("metal-interpreter", ExecutionBackend::MetalInterpreter), ("metal-jit", ExecutionBackend::MetalJit)]);
    let mut reference: Option<(Keys, BTreeMap<(String, String), Coverage>)> = None;
    for (name, backend) in backends {
        let store = ContentStore::open(&temp(&format!("phase4-{name}")).with_extension("sqlite")).unwrap();
        let mut rules = Rules::lower(&policies, backend).unwrap();
        let start = cpu();
        let root = directory_root(&store, &dir).unwrap();
        let run = audit(&store, WS, root, &mut rules).inspect_err(|_| crate::code_rule_dsl::prof::report(name)).unwrap();
        let (found, covered) = (keys(&run), coverage(&run));
        eprintln!("phase 4 {name}: {} files, {} clauses, {} keys, cpu {:?}", run.paths.len(), rules.clauses(), found.len(), cpu() - start);
        match &reference {
            None => {
                if let Ok(path) = std::env::var("RERAIL_DUMP_KEYS") {
                    let lines: String = found.iter().map(|k| format!("{}\t{}\t{}\t{}\n", k.0, k.1, k.2, k.3)).collect();
                    std::fs::write(&path, lines).unwrap();
                    eprintln!("phase 4 {name}: footprint {} MB", crate::code_rule_dsl::prof::footprint_mb());
                }
                assert!(!found.is_empty(), "the sample finds nothing");
                reference = Some((found, covered));
            }
            Some((keys0, coverage0)) => {
                let missing = keys0.difference(&found).take(5).collect::<Vec<_>>();
                let extra = found.difference(keys0).take(5).collect::<Vec<_>>();
                assert!(missing.is_empty() && extra.is_empty(), "{name} differs from the interpreter: missing {missing:?}, extra {extra:?}");
                assert_eq!(&covered, coverage0, "{name}: coverage differs from the interpreter");
            }
        }
    }
}

/// 06 S7's gate. On the sample (`sample`), the slice's cold audit (`audit`, every Python clause
/// S1/S2 lowers, on the CPU, and on Metal with `RERAIL_S7_BACKENDS`) gives the same finding keys as W's engine for
/// every (rule, code). Every (rule, file) it leaves `Unknown` is one where W's clause fails. It
/// reports the coverage of the pack's Python clauses and the answer count beside each time.
///
/// The mutation check (spike 2.17) shows the gate can fail. Moving one kept span's start back one
/// byte, onto the previous line, and re-running the layer over the kept rows (`apply` on the same
/// root) differs from W. So does one rule broken (`broken`).
#[test]
#[ignore = "O37: the lowered B006 clause reports 4 annotated defaults on f010.py that W does not"]
fn the_slice_finds_ws_findings_on_real_files() {
    let policies = policies();
    let dir = temp("s7");
    let (paths, repo) = sample(&dir);
    let sources = paths.iter().map(|p| (p.clone(), std::fs::read_to_string(dir.join(p)).unwrap())).collect::<Vec<_>>();
    let bytes = sources.iter().map(|(_, s)| s.len()).sum::<usize>();
    let (clauses, _) = lowered(&policies).unwrap();
    let names = clauses.iter().map(|c| c.name.clone()).collect::<BTreeSet<_>>();
    let total = python_clauses(&policies);

    // W runs after the first backend's audit, so that audit's footprint is the engine's own
    // (W's run leaves ~260 MB in the process: 40 MB before it, ~300 MB after).
    let footprint = crate::code_rule_dsl::prof::footprint_mb;
    let mut w_run = None;

    // `RERAIL_S7_BACKENDS`: the backends, of `cpu` and (on macOS) `metal`; default `cpu`, which
    // keeps the gate within a minute (Metal adds 15 s and agrees; decision 0017).
    let wanted = std::env::var("RERAIL_S7_BACKENDS").unwrap_or_else(|_| "cpu".to_owned());
    let mut backends = Vec::new();
    // The production bindings: interpreted until the wave's plan is compiled (list step 16).
    if wanted.contains("cpu") {
        backends.push(("cpu", ExecutionBackend::CpuBackground));
    }
    #[cfg(target_os = "macos")]
    if wanted.contains("metal") {
        backends.push(("metal", ExecutionBackend::MetalBackground));
    }
    let mut cpu_store = None;
    for (name, backend) in backends {
        let store = ContentStore::open(&temp(&format!("s7-{name}")).with_extension("sqlite")).unwrap();
        let mut rules = Rules::lower(&policies, backend).unwrap();
        let (cpu0, wall0) = (cpu(), std::time::Instant::now());
        let root = directory_root(&store, &dir).unwrap();
        let before = footprint();
        let run = audit(&store, WS, root, &mut rules).unwrap();
        let (spent, wall) = (cpu() - cpu0, wall0.elapsed());
        eprintln!("S7 {name} footprint: {before} MB before the audit, {} MB after{}", footprint(), if w_run.is_none() { " (before W's run)" } else { "" });
        #[cfg(target_os = "macos")]
        eprintln!("S7 {name} Metal buffers: peak {} MB", crate::code_rule_dsl::transducer::packed::metal_backend::peak_mb());
        // `RERAIL_S7_HOLD=<path>`: wait here, the audit's state alive, until `path` exists (to
        // inspect the process's heap).
        if let Some(path) = std::env::var_os("RERAIL_S7_HOLD") {
            #[cfg(target_os = "macos")]
            eprintln!(
                "S7 Metal buffers alive: {} MB; the driver's allocation {} MB",
                crate::code_rule_dsl::transducer::packed::metal_backend::reserved_mb(),
                crate::code_rule_dsl::transducer::packed::metal_backend::driver_mb()
            );
            let jemalloc = || {
                let _ = tikv_jemalloc_ctl::epoch::advance();
                format!(
                    "allocated {} MB, active {} MB, resident {} MB",
                    tikv_jemalloc_ctl::stats::allocated::read().unwrap_or(0) >> 20,
                    tikv_jemalloc_ctl::stats::active::read().unwrap_or(0) >> 20,
                    tikv_jemalloc_ctl::stats::resident::read().unwrap_or(0) >> 20
                )
            };
            eprintln!("S7 jemalloc: {}", jemalloc());
            crate::code_rule_dsl::prof::purge_allocator_pages();
            eprintln!("S7 after purge: jemalloc {}; footprint {} MB", jemalloc(), footprint());
            eprintln!("S7 holding, pid {}", std::process::id());
            while !std::path::Path::new(&path).exists() {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        crate::code_rule_dsl::prof::report(name);
        // Decision 0026: no stored relation holds a row per node (at most 0.25 a node each).
        let nodes = store.query_rows("SELECT sum(nodes) FROM prefilter_sets", [], |r| r.get::<_, i64>(0)).unwrap()[0];
        let tables = store.query_rows("SELECT name FROM sqlite_schema WHERE type='table' AND sql NOT LIKE 'CREATE VIRTUAL%'", [], |r| r.get::<_, String>(0)).unwrap();
        for table in &tables {
            let rows = store.query_rows(&format!("SELECT count(*) FROM \"{table}\""), [], |r| r.get::<_, i64>(0)).unwrap()[0];
            assert!(rows as f64 <= 0.25 * nodes as f64, "{table}: {rows} rows for {nodes} nodes");
        }
        let (ws, failed, w_time) = w_run.get_or_insert_with(|| {
            let (cpu0, wall0) = (cpu(), std::time::Instant::now());
            let (ws, failed) = ws_keys(&policies, &names, &sources);
            (ws, failed, (cpu() - cpu0, wall0.elapsed()))
        });
        let (ws, failed, w_time) = (&*ws, &*failed, *w_time);
        let difference = Difference::of(&run, ws, failed);
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
        for key in ours.symmetric_difference(ws).take(20) {
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
    let (ws, failed, _) = w_run.expect("W ran after the first backend");
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
/// levels of the same construct. W finds no yoda condition in any of these. The sweep's marks
/// that only added kept a verdict read from an unsettled complement, on the CPU and on Metal alike.
#[test]
fn a_recursive_label_settles_at_every_depth() {
    let policies = policies();
    let dir = temp("o29");
    std::fs::create_dir_all(&dir).unwrap();
    let source = "X == 4 * 365 + 1\nX == [[1]]\nX == 1 + 2 + 3\nX == (4 + 1)\n4 * 365 + 1 == x\n";
    write(&dir, "a.py", source);
    let (clauses, _) = lowered(&policies).unwrap();
    let names = clauses.iter().map(|c| c.name.clone()).collect::<BTreeSet<_>>();
    let (ws, failed) = ws_keys(&policies, &names, &[("a.py".to_owned(), source.to_owned())]);
    let run = cold(&dir, &policies);
    let difference = Difference::of(&run, &ws, &failed);
    assert!(difference.clauses.is_empty(), "{:?}", difference.clauses);
}

/// The napi entry point finds what the slice's own `audit` finds, and a run over the same
/// snapshot afterwards is warm: nothing evaluated, nothing re-run.
#[test]
fn the_napi_slice_audits_and_applies_as_the_slice_does() {
    let dir = temp("napi-dir");
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir, "a.py", "assert x\nimport os\nos.chmod('f', 0o777)\n");
    write(&dir, "b.py", "def f():\n    password = 'hunter2'\n    return password\n");
    let mut documents = Vec::new();
    files(&pack().join("rules"), "json", &mut documents);
    files(&pack().join("candidates/rules"), "json", &mut documents);
    documents.sort();
    let sources = documents
        .iter()
        .map(|path| napi::SlicePolicy {
            name: path.file_stem().unwrap().to_string_lossy().into_owned(),
            source: std::fs::read_to_string(path).unwrap(),
            format: "json".to_owned(),
        })
        .filter(|policy| CodeRuleDslRuntime::compile(policy.source.clone(), policy.format.clone(), CAPABILITIES.to_owned()).is_ok())
        .collect::<Vec<_>>();
    let store = temp("napi-store").with_extension("sqlite");
    let mut slice = napi::CodeRuleSlice::new(store.to_string_lossy().into_owned(), sources, CAPABILITIES.to_owned(), "cpu".to_owned()).unwrap();
    assert!(slice.clauses() > 0);
    let found = |json: &str| -> (Keys, usize, u64) {
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        let keys = value["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| (k["rule"].as_str().unwrap().to_owned(), k["code"].as_str().unwrap().to_owned(), k["path"].as_str().unwrap().to_owned(), k["line"].as_u64().unwrap() as usize))
            .collect();
        (keys, value["evaluated"].as_array().unwrap().len(), value["rerun"].as_u64().unwrap())
    };
    let (keys, evaluated, _) = found(&slice.audit(WS.to_owned(), dir.to_string_lossy().into_owned(), None, None).unwrap());
    assert_eq!(keys, self::keys(&cold(&dir, &policies())));
    assert!(!keys.is_empty() && evaluated == 2);
    let (warm, evaluated, rerun) = found(&slice.apply(WS.to_owned(), dir.to_string_lossy().into_owned(), None, None).unwrap());
    assert_eq!((warm, evaluated, rerun), (keys.clone(), 0, 0));
    // The Git observer: a clean checkout's `HEAD` is the directory's snapshot, so it is warm too.
    let git = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .args(["-c", "user.name=rerail", "-c", "user.email=rerail@example.invalid", "-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(status.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&status.stderr));
    };
    git(&["init", "-q"]);
    git(&["add", "a.py", "b.py"]);
    git(&["commit", "-q", "-m", "fixture"]);
    let (head, evaluated, rerun) = found(&slice.apply(WS.to_owned(), dir.to_string_lossy().into_owned(), Some("head".to_owned()), None).unwrap());
    assert_eq!((head, evaluated, rerun), (keys.clone(), 0, 0));
    assert!(slice.apply(WS.to_owned(), dir.to_string_lossy().into_owned(), Some("nope".to_owned()), None).is_err());
    // The buffers observer: an unsaved edit of `b.py` and an unsaved new file in a new directory
    // are found as if written, and nothing on disk changes.
    let buffers = || {
        vec![
            napi::SliceBuffer { path: "b.py".to_owned(), source: "def f():\n    assert x\n    password = 'hunter2'\n    return password\n".to_owned() },
            napi::SliceBuffer { path: "pkg/c.py".to_owned(), source: "import os\nos.chmod('g', 0o777)\n".to_owned() },
        ]
    };
    assert!(slice.apply(WS.to_owned(), dir.to_string_lossy().into_owned(), Some("head".to_owned()), Some(buffers())).is_err());
    let (buffered, evaluated, _) = found(&slice.apply(WS.to_owned(), dir.to_string_lossy().into_owned(), None, Some(buffers())).unwrap());
    assert_eq!(evaluated, 2);
    let written = temp("napi-written");
    std::fs::create_dir_all(written.join("pkg")).unwrap();
    std::fs::copy(dir.join("a.py"), written.join("a.py")).unwrap();
    for buffer in buffers() {
        std::fs::write(written.join(&buffer.path), &buffer.source).unwrap();
    }
    assert_eq!(buffered, self::keys(&cold(&written, &policies())));
    assert_ne!(buffered, keys);
    assert!(!dir.join("pkg").exists());
}

/// S8 (06): the first measurements against 03's targets, on S7's sample. Each pass runs, per
/// backend (`RERAIL_S8_BACKENDS`, default `cpu,metal`) and in a fresh store: a cold audit, a warm
/// one-file edit and a warm whitespace-only edit, each with its CPU time (user + sys), wall time,
/// load, documents evaluated, derivations re-run and footprint. The backends interleave within a
/// pass; the first pass is discarded (`RERAIL_S8_PASSES`, default 3, counts it). Build with
/// `--profile profiling`.
#[test]
#[ignore = "S8's measurements; run with --ignored --nocapture"]
fn s8_measures_the_targets_on_s7s_sample() {
    let dir = temp("s8");
    let (paths, repo) = sample(&dir);
    let first = dir.join(paths.iter().find(|p| p.starts_with('r')).unwrap_or(&paths[0]));
    let source = std::fs::read_to_string(&first).unwrap();
    let policies = policies();
    let backends = std::env::var("RERAIL_S8_BACKENDS").unwrap_or_else(|_| "cpu,metal".to_owned());
    let passes = std::env::var("RERAIL_S8_PASSES").ok().and_then(|n| n.parse().ok()).unwrap_or(3_usize);
    let load = || {
        let mut loads = [0.0_f64; 3];
        unsafe { libc::getloadavg(loads.as_mut_ptr(), 3) };
        loads[0]
    };
    eprintln!("S8: {} files ({} from {repo}); edited {}", paths.len(), paths.iter().filter(|p| p.starts_with('r')).count(), first.display());
    for pass in 0..passes {
        for backend in backends.split(',') {
            let execution = match backend {
                #[cfg(target_os = "macos")]
                "metal" => ExecutionBackend::MetalJit,
                _ => ExecutionBackend::CpuJit,
            };
            std::fs::write(&first, &source).unwrap();
            let store = ContentStore::open(&temp("s8-store").with_extension("sqlite")).unwrap();
            let mut rules = Rules::lower(&policies, execution).unwrap();
            let mut arm = |what: &str, cold: bool| {
                let (load, cpu0, wall0) = (load(), cpu(), std::time::Instant::now());
                let root = directory_root(&store, &dir).unwrap();
                let run = if cold { audit(&store, WS, root, &mut rules) } else { apply(&store, WS, root, &mut rules) }.unwrap();
                let (cpu, wall) = (cpu() - cpu0, wall0.elapsed());
                let kept = if pass == 0 { " (discarded)" } else { "" };
                eprintln!(
                    "S8 pass {pass} {backend} {what}{kept}: {} keys, {} evaluated, {} re-run; cpu {:.3} s, wall {:.3} s, load {load:.1}; footprint {} MB",
                    run.findings.keys.len(),
                    run.evaluated.len(),
                    run.ingested.push.work.rerun,
                    cpu.as_secs_f64(),
                    wall.as_secs_f64(),
                    crate::code_rule_dsl::prof::footprint_mb(),
                );
                crate::code_rule_dsl::prof::report(what);
            };
            arm("cold", true);
            std::fs::write(&first, format!("{source}\ng = lambda: 1\n")).unwrap();
            arm("one-file edit", false);
            std::fs::write(&first, format!("{source}\ng = lambda: 1 \n")).unwrap();
            arm("whitespace edit", false);
        }
    }
    eprintln!("S8 peak footprint: {} MB", crate::code_rule_dsl::prof::max_footprint_mb());
}

/// A push that crashed after observing its root (names rebound, nothing derived, the push marker
/// set) is finished by the next run (O1): the edited file's derivations re-run, it is evaluated,
/// and the findings equal a cold audit's of the edited snapshot.
#[test]
fn a_run_after_a_crashed_push_recovers_it() {
    let dir = temp("crash-dir");
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir, "a.py", "import os\nos.chmod('f', 0o777)\n");
    write(&dir, "b.py", "def f():\n    return 1\n");
    let policies = policies();
    let store = ContentStore::open(&temp("crash").with_extension("sqlite")).unwrap();
    let mut rules = Rules::lower(&policies, ExecutionBackend::CpuJit).unwrap();
    let before = keys(&audit(&store, WS, directory_root(&store, &dir).unwrap(), &mut rules).unwrap());
    write(&dir, "b.py", "def f():\n    assert x\n    return 1\n");
    let root = directory_root(&store, &dir).unwrap();
    // The crash: the push began and observed the root, and stopped there.
    store.begin_push().unwrap();
    crate::code_rule_dsl::facts::observe::observe_root(&store, WS, root).unwrap();
    assert!(!store.push_clean().unwrap() && !store.invalid_derivations().unwrap().is_empty());
    let run = apply(&store, WS, root, &mut rules).unwrap();
    assert!(store.push_clean().unwrap() && store.invalid_derivations().unwrap().is_empty());
    assert_eq!(run.evaluated, ["b.py"]);
    let after = keys(&run);
    assert_ne!(after, before, "the edit adds a finding");
    assert_eq!(after, keys(&cold(&dir, &policies)));
}

/// The content memo (05 Phase 1's facts memo row): a second scope over the same files, a rename,
/// and an edit back to content evaluated before take the rows from the memo, evaluate nothing
/// for those documents, and find what a cold audit of the same snapshot in a fresh store finds.
#[test]
fn documents_seen_before_take_their_rows_from_the_content_memo() {
    let one = temp("memo-one");
    std::fs::create_dir_all(&one).unwrap();
    write(&one, "a.py", "import os\nos.chmod('f', 0o777)\n");
    write(&one, "b.py", "def f():\n    assert x\n    return 1\n");
    let policies = policies();
    let store = ContentStore::open(&temp("memo").with_extension("sqlite")).unwrap();
    let mut rules = Rules::lower(&policies, ExecutionBackend::CpuJit).unwrap();
    let first = audit(&store, WS, directory_root(&store, &one).unwrap(), &mut rules).unwrap();
    assert_eq!((first.evaluated.len(), first.reused.len()), (2, 0));
    // Another scope over the same files: every document from the memo.
    let other = audit(&store, "/other", directory_root(&store, &one).unwrap(), &mut rules).unwrap();
    assert_eq!((other.evaluated.len(), other.reused.len()), (0, 2));
    assert_eq!(keys(&other), keys(&first));
    // A rename keeps the content: from the memo, found at the new path.
    let two = temp("memo-two");
    std::fs::create_dir_all(&two).unwrap();
    std::fs::copy(one.join("a.py"), two.join("a.py")).unwrap();
    std::fs::copy(one.join("b.py"), two.join("renamed.py")).unwrap();
    let renamed = audit(&store, "/two", directory_root(&store, &two).unwrap(), &mut rules).unwrap();
    assert_eq!((renamed.evaluated.len(), renamed.reused.len()), (0, 2));
    assert_eq!(keys(&renamed), keys(&cold(&two, &policies)));
    // An edit, then back: the edit is evaluated, the return to the first content is not.
    write(&one, "b.py", "def f():\n    return 1\n");
    let edited = apply(&store, WS, directory_root(&store, &one).unwrap(), &mut rules).unwrap();
    assert_eq!((edited.evaluated.as_slice(), edited.reused.len()), (&["b.py".to_owned()][..], 0));
    write(&one, "b.py", "def f():\n    assert x\n    return 1\n");
    let back = apply(&store, WS, directory_root(&store, &one).unwrap(), &mut rules).unwrap();
    assert_eq!((back.evaluated.len(), back.reused.as_slice()), (0, &["b.py".to_owned()][..]));
    assert_eq!(keys(&back), keys(&first));
}

#[test]
fn the_statistics_loop_runs_on_the_slices_path() {
    let one = temp("stats-one");
    std::fs::create_dir_all(&one).unwrap();
    write(&one, "a.py", "import os\nos.chmod('f', 0o777)\n");
    write(&one, "b.py", "def f():\n    assert x\n    return 1\n");
    let policies = policies();
    let store = ContentStore::open(&temp("stats").with_extension("sqlite")).unwrap();
    let mut rules = Rules::lower(&policies, ExecutionBackend::CpuJit).unwrap();
    let count = |sql: &str| store.query_rows(sql, [], |row| row.get::<_, i64>(0)).unwrap()[0];
    let first = audit(&store, WS, directory_root(&store, &one).unwrap(), &mut rules).unwrap();
    assert_eq!(keys(&first), keys(&cold(&one, &policies)), "gated sets keep every finding");
    // The gates are compiled plans with the statistics they read, and each batch's selectivity
    // is fed back.
    let plans = count("SELECT count(*) FROM compiled_plans");
    assert!(plans > 0);
    assert!(count("SELECT count(*) FROM compiled_plan_reads") > 0);
    assert!(count("SELECT count(*) FROM plan_feedback") > 0);
    // The kept rows' column statistics, and SQLite's from them.
    let matches = count("SELECT count(*) FROM slice_match");
    assert_eq!(count("SELECT rows FROM fact_stats WHERE tbl = 'slice_match' AND col = 'c0'"), matches);
    assert_eq!(count("SELECT count(*) FROM sqlite_stat1 WHERE tbl IN ('slice_match', 'slice_span')"), 2);
    // An edit: the statistics move by the edited document, and match a cold build's.
    write(&one, "b.py", "def f():\n    return 1\n");
    let edited = apply(&store, WS, directory_root(&store, &one).unwrap(), &mut rules).unwrap();
    assert_eq!(keys(&edited), keys(&cold(&one, &policies)));
    let matches = count("SELECT count(*) FROM slice_match");
    assert_eq!(count("SELECT rows FROM fact_stats WHERE tbl = 'slice_match' AND col = 'c0'"), matches);
    let distinct = count("SELECT count(DISTINCT c0) FROM slice_match");
    assert_eq!(count("SELECT ndv FROM fact_stats WHERE tbl = 'slice_match' AND col = 'c0'"), distinct);
    // Unchanged statistics keep their compiled gates.
    assert_eq!(count("SELECT count(*) FROM compiled_plans"), plans);
    assert!(count("SELECT max(samples) FROM plan_feedback") >= 2);
}

mod data_layer;

/// RER-131 probe (scratch): one pack rule on one small file, interpreter only.
#[test]
#[ignore = "RER-131 scratch probe"]
fn rer131_one_rule_probe() {
    let rule = std::env::var("RERAIL_PROBE_RULE").unwrap_or("ruff-e722-bare-except".into());
    let source = std::env::var("RERAIL_PROBE_SOURCE").unwrap_or("try:\n    x()\nexcept:\n    pass\n".into());
    let policies: Vec<_> = policies().into_iter().filter(|(name, _)| *name == rule).collect();
    assert_eq!(policies.len(), 1);
    let dir = temp("probe");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), source).unwrap();
    let store = ContentStore::open(&temp("probe-store").with_extension("sqlite")).unwrap();
    let mut rules = Rules::lower(&policies, ExecutionBackend::CpuInterpreter).unwrap();
    let root = directory_root(&store, &dir).unwrap();
    eprintln!("PROBE keys {:?}", keys(&audit(&store, WS, root, &mut rules).unwrap()));
}

/// RER-131 oracle (ignored): one pack's rules over a file list, interpreter only, keys to `RERAIL_DUMP_KEYS`.
/// `RERAIL_PACK` (a directory under `packs/`), `RERAIL_FILES` (a list of paths), `RERAIL_MAX` (default 200).
#[test]
#[ignore = "RER-131 oracle: needs RERAIL_PACK and RERAIL_FILES"]
fn rer131_pack_keys() {
    let pack = Path::new(env!("CARGO_MANIFEST_DIR")).join("../packs").join(std::env::var("RERAIL_PACK").unwrap());
    let mut documents = Vec::new();
    files(&pack.join("rules"), "json", &mut documents);
    files(&pack.join("candidates/rules"), "json", &mut documents);
    documents.sort();
    let policies: Vec<_> = documents
        .into_iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            // A policy that does not compile is left out, and named.
            let compiled = CodeRuleDslRuntime::compile(text, "json".to_owned(), CAPABILITIES.to_owned())
                .map_err(|error| eprintln!("SKIP {}: {}", path.display(), error.reason))
                .ok()?;
            Some((path.file_stem().unwrap().to_string_lossy().into_owned(), compiled))
        })
        .collect();
    let max: usize = std::env::var("RERAIL_MAX").ok().and_then(|m| m.parse().ok()).unwrap_or(200);
    let dir = temp("pack-keys");
    std::fs::create_dir_all(&dir).unwrap();
    let list = std::fs::read_to_string(std::env::var("RERAIL_FILES").unwrap()).unwrap();
    let mut copied = 0;
    for (i, path) in list.lines().filter(|l| !l.is_empty()).enumerate() {
        if copied == max {
            break;
        }
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        let name = Path::new(path).file_name().unwrap().to_string_lossy();
        std::fs::write(dir.join(format!("f{i:05}-{name}")), text).unwrap();
        copied += 1;
    }
    let store = ContentStore::open(&temp("pack-keys-store").with_extension("sqlite")).unwrap();
    let backend = match std::env::var("RERAIL_BACKEND").as_deref() {
        Ok("jit") => ExecutionBackend::CpuJit,
        #[cfg(target_os = "macos")]
        Ok("metal") => ExecutionBackend::MetalInterpreter,
        _ => ExecutionBackend::CpuInterpreter,
    };
    // A policy that does not lower on its own is left out, and named.
    let policies: Vec<_> = policies
        .into_iter()
        .filter(|policy| {
            let lowered = Rules::lower(std::slice::from_ref(policy), backend);
            if let Err(error) = &lowered {
                eprintln!("SKIP {}: {error:?}", policy.0);
            }
            lowered.is_ok()
        })
        .collect();
    let mut rules = Rules::lower(&policies, backend).unwrap();
    let start = cpu();
    let root = directory_root(&store, &dir).unwrap();
    let run = audit(&store, WS, root, &mut rules).inspect_err(|error| eprintln!("AUDIT {error:?}")).unwrap();
    let found = keys(&run);
    eprintln!("PACK {}: {} rules, {} files, {} keys, cpu {:?}", pack.display(), policies.len(), copied, found.len(), cpu() - start);
    // The slice runs only the clauses that lower to its wave; a rule with none finds nothing here and has no
    // coverage, so it is named (the runtime, `rer131_runtime_keys`, evaluates every rule).
    let (clauses, _) = crate::code_rule_dsl::slice::lowered(&policies).unwrap();
    let waved: BTreeSet<&str> = clauses.iter().filter_map(|clause| clause.name.split('/').next()).collect();
    let left: Vec<&str> = policies.iter().map(|(name, _)| name.as_str()).filter(|name| !waved.contains(name)).collect();
    eprintln!("NOT IN THE WAVE: {} of {} rules", left.len(), policies.len());
    for name in left {
        eprintln!("  {name}");
    }
    crate::code_rule_dsl::prof::report("pack");
    if let Ok(path) = std::env::var("RERAIL_DUMP_KEYS") {
        std::fs::write(&path, found.iter().map(|k| format!("{}\t{}\t{}\t{}\n", k.0, k.1, k.2, k.3)).collect::<String>()).unwrap();
    }
}

/// RER-131 oracle (ignored), any language: every policy of `RERAIL_PACK` evaluated by the runtime
/// over `RERAIL_FILES` (first `RERAIL_MAX`, default 150), its violations as keys to `RERAIL_DUMP_KEYS`.
#[test]
#[ignore = "RER-131 oracle: needs RERAIL_PACK and RERAIL_FILES"]
fn rer131_runtime_keys() {
    let pack = Path::new(env!("CARGO_MANIFEST_DIR")).join("../packs").join(std::env::var("RERAIL_PACK").unwrap());
    let mut documents = Vec::new();
    files(&pack.join("rules"), "json", &mut documents);
    files(&pack.join("candidates/rules"), "json", &mut documents);
    documents.sort();
    let max: usize = std::env::var("RERAIL_MAX").ok().and_then(|m| m.parse().ok()).unwrap_or(150);
    let label = |path: &str| match Path::new(path).extension().and_then(|e| e.to_str()) {
        Some("ts" | "mts" | "cts") => "typescript",
        Some("tsx") => "tsx",
        Some("js" | "mjs" | "cjs" | "jsx") => "javascript",
        Some("py") => "python",
        Some("php") => "php",
        Some("rs") => "rust",
        Some("sql") => "sql",
        Some("vue") => "vue",
        _ => "",
    };
    let list = std::fs::read_to_string(std::env::var("RERAIL_FILES").unwrap()).unwrap();
    let inputs: Vec<serde_json::Value> = list
        .lines()
        .filter(|l| !label(l).is_empty())
        .filter_map(|path| Some((path, std::fs::read_to_string(path).ok()?)))
        .take(max)
        .enumerate()
        .map(|(i, (path, source))| serde_json::json!({ "path": format!("f{i:05}-{}", Path::new(path).file_name().unwrap().to_string_lossy()), "language": label(path), "source": source }))
        .collect();
    let snapshot = serde_json::json!({ "facts": [], "format": "rerail.repository-snapshot/v1", "revision": 0, "files": inputs }).to_string();
    let start = cpu();
    let mut out = String::new();
    let (mut rules, mut failed, mut keys) = (0, 0, 0);
    for path in documents {
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let compiled = match CodeRuleDslRuntime::compile(text, "json".to_owned(), CAPABILITIES.to_owned()) {
            Ok(compiled) => compiled,
            // A policy that does not compile is left out, and named.
            Err(error) => {
                eprintln!("SKIP {}: {}", path.display(), error.reason);
                continue;
            }
        };
        rules += 1;
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        let result = match compiled.evaluate(snapshot.clone(), "report".to_owned()) {
            Ok(result) => result,
            Err(error) => {
                eprintln!("FAILURE {stem}: {}", error.reason.chars().take(300).collect::<String>());
                failed += 1;
                continue;
            }
        };
        let result: serde_json::Value = serde_json::from_str(&result).unwrap();
        if let Some(failures) = result["failures"].as_array().filter(|f| !f.is_empty()) {
            eprintln!("FAILURE {stem}: {}", serde_json::Value::from(failures.clone()).to_string().chars().take(300).collect::<String>());
            failed += 1;
        }
        for v in result["violations"].as_array().into_iter().flatten() {
            if keys == 0 && std::env::var_os("RERAIL_SHOW_VIOLATION").is_some() {
                eprintln!("VIOLATION {v}");
            }
            let line = v.pointer("/span/startLine").cloned().unwrap_or_default();
            out.push_str(&format!("{stem}\t{}\t{}\t{line}\n", v["code"].as_str().unwrap_or(""), v["path"].as_str().unwrap_or("")));
            keys += 1;
        }
    }
    eprintln!("RUNTIME {}: {rules} rules ({failed} with failures), {} files, {keys} keys, cpu {:?}", pack.display(), max, cpu() - start);
    if let Ok(path) = std::env::var("RERAIL_DUMP_KEYS") {
        let mut lines: Vec<&str> = out.lines().collect();
        lines.sort_unstable();
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    }
}
