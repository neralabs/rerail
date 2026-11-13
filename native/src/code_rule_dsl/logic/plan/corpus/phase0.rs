//! Phase 0: the loading of rule files, as a run of `bootstrap.rr` (t25).
//!
//! The engine knows one rule file by itself, `bootstrap.rr`, and a few builtin facts:
//! - the library directory, as `env("rerail.lib", dir)`;
//! - each program text (`p0_text`), whether it is in T's shape (`p0_t_shape`), and the names it includes (`p0_ask`);
//! - each program file it was given (`p0_file`).
//!
//! From these, `bootstrap.rr` asks for each rule file by `load(path, name, "rr")`. This run's loader answers that like
//! any document's load: the file is parsed into its tape, kept in the content store, and memoised per file. Each
//! loaded file's `#include`s are facts of its tree, and they ask for further loads, until none is new. The run's
//! outputs are the include graph: which file each `#include` names, which names have no file, and which candidates
//! were not there.
//!
//! A program is then assembled from the graph, each module's statements read off its stored tape (`rr::read`). The
//! graph is the program's key (`Graph::digest`): its modules' paths and digests, and its edges.
//!
//! A run of phase 0 is a corpus run like any other. Its record (`reuse`) makes a run whose rule files are all as
//! they were a check of their `stat`s; within a process, the graph is kept while every module and every absent
//! candidate is as it was.

use super::{Error as PlanError, Result};
use crate::code_rule_dsl::logic::front::Error;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

// Whether this thread runs phase 0: the flag is the profiler's (`phase_clock`), which reads it too.
pub use crate::code_rule_dsl::phase_clock::{IN_PHASE_0, active};

/// What phase 0 starts from for one program: its text's includes, or its file.
#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum Seed {
    /// A program text: in T's shape, and its includes as (offset, name).
    Text { t_shape: bool, includes: Vec<(i64, String)> },
    /// A program file (canonical).
    File(PathBuf),
}

/// The include graph phase 0 derived.
#[derive(Clone, Debug, Default)]
pub struct Graph {
    /// For each includer ("text:0", a module's path, or "engine"), its includes in order: (offset, name, file).
    pub(in crate::code_rule_dsl) includes: BTreeMap<String, Vec<(i64, String, PathBuf)>>,
    /// The names with no file: (includer, offset, name).
    pub(in crate::code_rule_dsl) missing: Vec<(String, i64, String)>,
    /// The candidates that were not there.
    pub(in crate::code_rule_dsl) absent: Vec<PathBuf>,
    /// Each module's digest, as phase 0's observer gives it.
    pub(in crate::code_rule_dsl) digests: BTreeMap<PathBuf, [u8; 32]>,
    /// The plans the run of `bootstrap.rr` that derived it used (`Report::plans`): the run that reads the graph
    /// names them in its record, so they stay live. Not part of the graph's digest.
    pub(in crate::code_rule_dsl) plans: Vec<[u8; 32]>,
}

impl Graph {
    /// Whether every module and every absent candidate is as it was: a `stat` each.
    fn holds(&self) -> bool {
        self.digests.iter().all(|(p, d)| super::observe(|o| o.digest(p)).as_ref() == Some(d))
            && self.absent.iter().all(|p| super::observe(|o| o.kind(p)) == crate::content_store::observe::Kind::Missing)
    }

    /// The program's key: its modules (path and digest) and its edges.
    pub fn digest(&self) -> [u8; 32] {
        use sha2::{Digest as _, Sha256};
        let mut h = Sha256::new();
        h.update(b"rerail.module-graph/v1\0");
        for (p, d) in &self.digests {
            h.update(p.as_os_str().as_encoded_bytes());
            h.update([0]);
            h.update(d);
        }
        for (from, to) in &self.includes {
            for (at, name, path) in to {
                h.update(format!("{from}\0{at}\0{name}\0{}\0", path.display()).as_bytes());
            }
        }
        for (from, at, name) in &self.missing {
            h.update(format!("missing\0{from}\0{at}\0{name}\0").as_bytes());
        }
        h.finalize().into()
    }

    /// The modules `from` reaches, each once, includes first (the order a program's statements are read in), added
    /// to `out` after those it holds already.
    pub(in crate::code_rule_dsl) fn order(&self, from: &str, out: &mut Vec<PathBuf>) -> std::result::Result<(), Error> {
        self.visit(from, out, &mut Vec::new())
    }

    fn visit(&self, from: &str, out: &mut Vec<PathBuf>, visiting: &mut Vec<String>) -> std::result::Result<(), Error> {
        if let Some((_, _, name)) = self.missing.iter().find(|(f, _, _)| f == from) {
            return Err(Error::Syntax(format!("{from}: #include {name:?}: no such file")));
        }
        for (_, _, path) in self.includes.get(from).into_iter().flatten() {
            let key = path.to_string_lossy().into_owned();
            if out.contains(path) || visiting.contains(&key) {
                continue;
            }
            visiting.push(key.clone());
            self.visit(&key, out, visiting)?;
            visiting.pop();
            out.push(path.clone());
        }
        Ok(())
    }

    /// Every module, and every candidate found absent, as `rr::LibraryFile`s check them.
    pub(in crate::code_rule_dsl) fn files(&self) -> Vec<(PathBuf, Option<[u8; 32]>)> {
        self.digests.iter().map(|(p, d)| (p.clone(), Some(*d))).chain(self.absent.iter().map(|p| (p.clone(), None))).collect()
    }
}

/// A text's key in the graph a seed is given.
pub(in crate::code_rule_dsl) const TEXT: &str = "text";

/// The include graph for `seed`: kept in this process while it holds, else derived by running `bootstrap.rr`.
pub fn graph(seed: &Seed) -> std::result::Result<Arc<Graph>, Error> {
    Ok(graphs(std::slice::from_ref(seed))?.remove(0))
}

/// The include graphs for `seeds`, each kept in this process while it holds; those not kept are derived by one run
/// of `bootstrap.rr` over all of them (a pack's programs are one run, not one each).
static KEPT: OnceLock<Mutex<HashMap<Seed, Arc<Graph>>>> = OnceLock::new();

pub fn graphs(seeds: &[Seed]) -> std::result::Result<Vec<Arc<Graph>>, Error> {
    let kept = KEPT.get_or_init(Default::default);
    let mut found: Vec<Option<Arc<Graph>>> = {
        let map = kept.lock().expect("phase 0's graphs");
        seeds.iter().map(|s| map.get(s).cloned()).collect()
    };
    for g in found.iter_mut() {
        if g.as_ref().is_some_and(|g| !g.holds()) {
            *g = None;
        }
    }
    let wanted: Vec<Seed> = {
        let mut w: Vec<Seed> = seeds.iter().zip(&found).filter(|(_, g)| g.is_none()).map(|(s, _)| s.clone()).collect();
        w.sort();
        w.dedup();
        w
    };
    crate::code_rule_dsl::prof::count("phase0.graph_kept", seeds.len() - found.iter().filter(|g| g.is_none()).count());
    if !wanted.is_empty() {
        let whole = derive(&wanted).map_err(|e| Error::Syntax(format!("phase 0: {e}")))?;
        let mut map = kept.lock().expect("phase 0's graphs");
        for (k, seed) in wanted.iter().enumerate() {
            map.insert(seed.clone(), Arc::new(whole.of(seed, k)));
        }
        for (g, seed) in found.iter_mut().zip(seeds) {
            if g.is_none() {
                *g = map.get(seed).cloned();
            }
        }
    }
    Ok(found.into_iter().map(|g| g.expect("each seed's graph")).collect())
}

/// Keeps the graph of a text made from a program file (`tools::program_texts`), taken from the file's graph rather
/// than from another run of `bootstrap.rr`: each name the text includes is a library module the file's graph holds,
/// found as `bootstrap.rr` finds a text's names (the library alone). The graph is the one that run would give, so
/// its digest is too. False when a name is not such a module (the text then gets its own run when asked).
pub(in crate::code_rule_dsl) fn keep_text(seed: &Seed, file: &Graph) -> bool {
    let Seed::Text { t_shape, includes } = seed else {
        return false;
    };
    let Ok(lib) = std::fs::canonicalize(crate::code_rule_dsl::logic::rr::library()) else {
        return false;
    };
    let mut asks: Vec<(i64, String, PathBuf)> = Vec::with_capacity(includes.len() + 1);
    let names = includes.iter().cloned().chain((!t_shape).then(|| (-1, "core.rr".to_owned())));
    for (at, name) in names {
        // `bootstrap.rr`'s candidate: `cat(lib, "/", name)`, as written, not made canonical.
        let path = PathBuf::from(format!("{}/{name}", lib.display()));
        if !file.digests.contains_key(&path) {
            return false;
        }
        asks.push((at, name, path));
    }
    asks.sort();
    let mut whole = file.clone();
    whole.includes.insert(text_key(0), asks);
    let graph = whole.of(seed, 0);
    KEPT.get_or_init(Default::default).lock().expect("phase 0's graphs").insert(seed.clone(), Arc::new(graph));
    true
}

/// A text seed's key in phase 0's facts, by its place among the run's seeds.
fn text_key(k: usize) -> String {
    format!("text:{k}")
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

impl Graph {
    /// The part of a run's graph one seed reaches (its `k`, its place among the run's seeds), a text's key named
    /// [`TEXT`].
    pub fn of(&self, seed: &Seed, k: usize) -> Graph {
        let start = match seed {
            Seed::Text { .. } => text_key(k),
            Seed::File(p) => p.to_string_lossy().into_owned(),
        };
        let mut g = Graph { absent: self.absent.clone(), plans: self.plans.clone(), ..Graph::default() };
        let mut pending = vec![start.clone(), "engine".to_owned()];
        let mut seen = std::collections::BTreeSet::new();
        while let Some(from) = pending.pop() {
            if !seen.insert(from.clone()) {
                continue;
            }
            let name = if from == start && matches!(seed, Seed::Text { .. }) { TEXT.to_owned() } else { from.clone() };
            if let Some(to) = self.includes.get(&from) {
                for (_, _, path) in to {
                    pending.push(path.to_string_lossy().into_owned());
                    if let Some(d) = self.digests.get(path) {
                        g.digests.insert(path.clone(), *d);
                    }
                }
                g.includes.insert(name.clone(), to.clone());
            }
            g.missing.extend(self.missing.iter().filter(|(f, _, _)| *f == from).map(|(_, at, n)| (name.clone(), *at, n.clone())));
        }
        if let Seed::File(p) = seed
            && let Some(d) = self.digests.get(p)
        {
            g.digests.insert(p.clone(), *d);
        }
        g
    }
}

/// Runs `bootstrap.rr` over the seeds' facts, as a corpus run with no corpus: every document it reads it loads.
pub fn derive(seeds: &[Seed]) -> Result<Graph> {
    let lib = crate::code_rule_dsl::logic::rr::library();
    let lib = std::fs::canonicalize(&lib).map_err(|e| PlanError::Plan(format!("{}: {e}", lib.display())))?;
    let bootstrap = lib.join("bootstrap.rr");
    // The builtin read: the one rule file the engine knows by itself.
    let (bytes, _) = super::observe(|o| o.read(&bootstrap)).ok_or_else(|| PlanError::Plan(format!("{}: cannot read the file", bootstrap.display())))?;
    let mut text = String::from_utf8(bytes).map_err(|e| PlanError::Plan(format!("{}: {e}", bootstrap.display())))?;
    text.push_str("\n// The engine's builtin facts.\n");
    text.push_str(&format!("env(\"rerail.lib\", {}).\n", quote(&lib.to_string_lossy())));
    // The library's parent is a root too: packs include their tables beside the library (`#include
    // "../packs/python/….names.rr"`), which the resolver before phase 0 read, and the loader confines reads to roots.
    if let Some(parent) = lib.parent() {
        text.push_str(&format!("env(\"rerail.library_parent\", {}).\n", quote(&parent.to_string_lossy())));
    }
    let mut dirs = std::collections::BTreeSet::new();
    for (k, seed) in seeds.iter().enumerate() {
        match seed {
            Seed::Text { t_shape, includes } => {
                let t = quote(&text_key(k));
                text.push_str(&format!("p0_text({t}).\n"));
                if *t_shape {
                    text.push_str(&format!("p0_t_shape({t}).\n"));
                }
                for (at, name) in includes {
                    text.push_str(&format!("p0_ask({t}, {at}, {}).\n", quote(name)));
                }
            }
            Seed::File(path) => {
                text.push_str(&format!("p0_file({}).\n", quote(&path.to_string_lossy())));
                // The file's own directory is a root the loader may read: its includes beside it are files there.
                if let Some(dir) = path.parent() {
                    dirs.insert(dir.to_path_buf());
                }
            }
        }
    }
    for dir in dirs {
        text.push_str(&format!("env(\"rerail.program_dir\", {}).\n", quote(&dir.to_string_lossy())));
    }
    // No corpus: an empty root, and the library a root the loader may read (`env`).
    let root = std::env::temp_dir().join("rerail-phase-0-root");
    std::fs::create_dir_all(&root).map_err(|e| PlanError::Plan(format!("{}: {e}", root.display())))?;
    static RUNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let out = std::env::temp_dir().join(format!("rerail-phase-0-{}-{}", std::process::id(), RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    let options = serde_json::json!({ "reuse": true });
    IN_PHASE_0.with(|c| c.set(true));
    let run = super::run_with_options(&[&text], &root, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18, &super::Typing::default(), &options);
    IN_PHASE_0.with(|c| c.set(false));
    let read = |rel: &str| -> Vec<Vec<String>> {
        std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap_or_default().lines().map(|l| l.split('\t').map(str::to_owned).collect()).collect()
    };
    let result = run.map(|report| {
        let mut g = Graph { plans: report.plans, ..Graph::default() };
        for r in read("p0_include") {
            if let [from, at, name, path] = r.as_slice() {
                g.includes.entry(from.clone()).or_default().push((at.parse().unwrap_or(0), name.clone(), PathBuf::from(path)));
            }
        }
        for v in g.includes.values_mut() {
            v.sort();
        }
        for r in read("p0_missing") {
            if let [from, at, name] = r.as_slice() {
                g.missing.push((from.clone(), at.parse().unwrap_or(0), name.clone()));
            }
        }
        g.missing.sort();
        g.absent = read("p0_absent").into_iter().filter_map(|r| r.first().map(PathBuf::from)).collect();
        let files = seeds.iter().filter_map(|s| match s {
            Seed::File(p) => Some(p.clone()),
            Seed::Text { .. } => None,
        });
        let modules: Vec<PathBuf> = g.includes.values().flatten().map(|(_, _, p)| p.clone()).chain(files).collect();
        for p in modules {
            if let Some(d) = super::observe(|o| o.digest(&p)) {
                g.digests.insert(p, d);
            }
        }
        g
    });
    let _ = std::fs::remove_dir_all(&out);
    result
}

// Its tests run in the engine's test binary (its facade of this module mounts `tests.rs`): they read what the
// engine registers as it loads (the languages, their `.rr` libraries).


#[cfg(test)]
pub mod benchmarks {
    /// Phase 0 over every `.rr` file of `RERAIL_P0_RULES` (a pack's directory), as a pack's run reads them
    /// (`tools::program_texts`): one run of `bootstrap.rr` for all. Run it twice for the warm figure.
    #[test]
    #[ignore = "a benchmark: RERAIL_P0_RULES=<a pack's directory>"]
    fn benchmark_phase_0_rules() {
        let dir = std::path::PathBuf::from(std::env::var("RERAIL_P0_RULES").expect("RERAIL_P0_RULES"));
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "rr")).collect();
        files.sort();
        let before = super::super::observe(|o| {
            o.begin();
            o.counts
        });
        let start = super::super::thread_usage();
        let refs: Vec<&std::path::Path> = files.iter().map(|p| p.as_path()).collect();
        let texts = crate::code_rule_dsl::logic::settings::program_texts(&refs).unwrap();
        let resolved = super::super::thread_usage();
        let seeds: Vec<super::Seed> = texts.iter().filter_map(|t| crate::code_rule_dsl::logic::rr::text_seed(t).unwrap()).collect();
        let graphs = super::graphs(&seeds).unwrap();
        let keyed = super::super::thread_usage();
        let did = super::super::observe(|o| {
            o.flush();
            o.counts.since(&before)
        });
        let modules: std::collections::BTreeSet<&std::path::PathBuf> = graphs.iter().flat_map(|g| g.digests.keys()).collect();
        let cpu = |a: (f64, f64), b: (f64, f64)| (b.0 - a.0) + (b.1 - a.1);
        println!(
            "phase 0 over {} files of {}: {} texts with includes, {} modules; files {:.2} s, texts' graphs {:.2} s CPU (thread); peak RSS {:.0} MB\n  {did}",
            files.len(),
            dir.display(),
            seeds.len(),
            modules.len(),
            cpu(start, resolved),
            cpu(resolved, keyed),
            super::super::usage().2
        );
    }
}
