//! Phase 0: the loading of rule files, as a run of `bootstrap.rerail` (t25).
//!
//! The engine knows one rule file by itself, `bootstrap.rerail`, and a few builtin facts:
//! - the library directory, as `env("rerail.lib", dir)`;
//! - each program text (`p0_text`), whether it is in T's shape (`p0_t_shape`), and the names it includes (`p0_ask`);
//! - each program file it was given (`p0_file`).
//!
//! From these, `bootstrap.rerail` asks for each rule file by `load(path, name, "rr")`. This run's loader answers that like
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
pub use crate::code_rule_dsl::phase_clock::{active, in_phase_0};

/// What phase 0 starts from for one program: its text's includes, or its file.
#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum Seed {
    /// A program text: in T's shape, its includes as (offset, name), and the phase programs its own declarations
    /// name (`rr::phases_named`): the run reads their exports, so they are rule files of its too.
    Text { t_shape: bool, includes: Vec<(i64, String)>, phases: Vec<String> },
    /// A program file (canonical).
    File(PathBuf),
}

// bootstrap.rerail splits and joins path symbols with '/'. A Windows verbatim path with mixed
// separators cannot be opened. Convert at both boundaries, including absent candidates.
fn bootstrap_spelling(s: &str) -> std::borrow::Cow<'_, str> {
    #[cfg(windows)]
    if s.contains('\\') { return s.replace('\\', "/").into(); }
    s.into()
}

fn host_spelling(s: &str) -> String {
    #[cfg(windows)]
    if s.contains('/') && std::path::Path::new(s).is_absolute() {
        let spelling = bootstrap_spelling(s);
        if let Ok(path) = std::path::absolute(spelling.as_ref()) {
            return path.to_string_lossy().into_owned();
        }
    }
    s.to_owned()
}

fn candidate_path(dir: &str, name: &str) -> PathBuf {
    PathBuf::from(host_spelling(&format!("{}/{name}", bootstrap_spelling(dir))))
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
    /// For each text or module, the phase programs its declarations name (`p0_phase`): (phase, candidate file). A
    /// candidate in `absent` is not there.
    pub(in crate::code_rule_dsl) phases: BTreeMap<String, Vec<(String, PathBuf)>>,
    /// The rule files a seed reaches through a phase program and not by its includes, each with its digest: the phase
    /// programs, their includes and the phases they name (decision 0246). A run reads their exports, so the
    /// whole-run cutoff names them (`reuse::rule_files`); the program's text is not made of them, so the graph's
    /// digest, which keys the program, leaves them out.
    pub(in crate::code_rule_dsl) rules: BTreeMap<PathBuf, [u8; 32]>,
    /// The plans the run of `bootstrap.rerail` that derived it used (`Report::plans`): the run that reads the graph
    /// names them in its record, so they stay live. A graph kept as an image keeps them (`Images`, Q150), so a run
    /// that reads it names what the run that derived it named. Not part of the graph's digest.
    pub(in crate::code_rule_dsl) plans: Vec<[u8; 32]>,
}

impl Graph {
    /// Whether every module and every absent candidate is as it was: a `stat` each.
    fn holds(&self) -> bool {
        self.digests.iter().chain(&self.rules).all(|(p, d)| super::observe(|o| o.digest(p)).as_ref() == Some(d))
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

    /// `holds`, for the engine's tests of phase 0 (`phase0::tests`, hosted where the languages are registered).
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn holds_now(&self) -> bool {
        self.holds()
    }

    /// Its includes, missing names and absent candidates, for a test's message.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn describe(&self) -> String {
        let digests: Vec<String> = self.digests.iter().map(|(p, d)| format!("{} {}", p.display(), d.iter().take(6).map(|b| format!("{b:02x}")).collect::<String>())).collect();
        format!("includes {:#?}\nmissing {:?}\nabsent {:?}\ndigests {digests:#?}", self.includes, self.missing, self.absent)
    }

    /// Every module, and every candidate found absent, as `rr::LibraryFile`s check them.
    pub(in crate::code_rule_dsl) fn files(&self) -> Vec<(PathBuf, Option<[u8; 32]>)> {
        self.digests.iter().map(|(p, d)| (p.clone(), Some(*d))).chain(self.absent.iter().map(|p| (p.clone(), None))).collect()
    }
}

/// A text's key in the graph a seed is given.
pub(in crate::code_rule_dsl) const TEXT: &str = "text";

/// The include graph for `seed`: kept in this process while it holds, else derived by running `bootstrap.rerail`.
pub fn graph(seed: &Seed) -> std::result::Result<Arc<Graph>, Error> {
    Ok(graphs(std::slice::from_ref(seed))?.remove(0))
}

/// The include graphs for `seeds`, each kept in this process while it holds; those not kept are derived by one run
/// of `bootstrap.rerail` over all of them (a pack's programs are one run, not one each).
#[derive(Default)]
struct PhaseZeroGraphs(Mutex<HashMap<(Option<Dirs>, Seed), Arc<Graph>>>);

pub fn graphs(seeds: &[Seed]) -> std::result::Result<Vec<Arc<Graph>>, Error> {
    let dirs = Dirs::here();
    let kept_held = crate::code_rule_dsl::logic::profile::shared::<PhaseZeroGraphs>();
    let kept = &kept_held.0;
    let mut found: Vec<Option<Arc<Graph>>> = {
        let map = kept.lock().expect("phase 0's graphs");
        seeds.iter().map(|s| map.get(&(dirs.clone(), s.clone())).cloned()).collect()
    };
    // A kept graph that no longer holds leaves the process's map too: the images' step below fills a seed from the
    // map, and found the stale graph there again, so a file edited after its graph was kept was read as it was (its
    // include's old digest), though phase 0 derived its graph anew.
    let stale: Vec<&Seed> = seeds.iter().zip(found.iter_mut()).filter(|(_, g)| g.as_ref().is_some_and(|g| !g.holds())).map(|(s, g)| {
        *g = None;
        s
    }).collect();
    if !stale.is_empty() {
        let mut map = kept.lock().expect("phase 0's graphs");
        for s in stale {
            map.remove(&(dirs.clone(), s.clone()));
        }
    }
    let images = Images::here();
    if let Some(images) = &images {
        for (seed, graph) in seeds.iter().zip(&found) {
            if let Some(graph) = graph {
                crate::code_rule_dsl::logic::rr::retain_graph_image(images.store.as_deref(), images.key(seed), || images.encode(graph));
            }
        }
    }
    let wanted: Vec<Seed> = {
        let mut w: Vec<Seed> = seeds.iter().zip(&found).filter(|(_, g)| g.is_none()).map(|(s, _)| s.clone()).collect();
        w.sort();
        w.dedup();
        w
    };
    crate::code_rule_dsl::prof::count("phase0.graph_kept", seeds.len() - found.iter().filter(|g| g.is_none()).count());
    // A seed's graph kept as an image (the shipped bundle, or an earlier process of this build) is taken where it
    // holds: a digest of each file and a look at each absent candidate, not a run of `bootstrap.rerail`.
    let mut wanted = wanted;
    if !wanted.is_empty() && let Some(images) = &images {
        let mut accepted = HashMap::new();
        // A pack's graphs share their modules: each file is looked at once for all of them, not once a graph.
        let mut looked: HashMap<PathBuf, Option<[u8; 32]>> = HashMap::new();
        let mut missing: HashMap<PathBuf, bool> = HashMap::new();
        let mut holds = |g: &Graph| {
            g.digests.iter().chain(&g.rules).all(|(p, d)| looked.entry(p.clone()).or_insert_with(|| super::observe(|o| o.digest(p))).as_ref() == Some(d))
                && g.absent.iter().all(|p| *missing.entry(p.clone()).or_insert_with(|| super::observe(|o| o.kind(p)) == crate::content_store::observe::Kind::Missing))
        };
        let graphs: Vec<Option<Graph>> = wanted.iter().map(|seed| crate::code_rule_dsl::logic::rr::graph_image(images.store.as_deref(), images.key(seed)).and_then(|b| images.decode(&b))).collect();
        let mut graphs = graphs.into_iter();
        let trace = std::env::var_os("RERAIL_PHASE0_TRACE").is_some();
        wanted.retain(|seed| {
            let image = graphs.next().flatten();
            if trace {
                let why = match &image {
                    None => "no image".to_owned(),
                    Some(g) => match g.digests.iter().chain(&g.rules).find(|(p, d)| super::observe(|o| o.digest(p)).as_ref() != Some(*d)) {
                        Some((p, _)) => format!("an image whose {} changed", p.display()),
                        None => match g.absent.iter().find(|p| super::observe(|o| o.kind(p)) != crate::content_store::observe::Kind::Missing) {
                            Some(p) => format!("an image whose absent {} is there", p.display()),
                            None => "an image that holds".to_owned(),
                        },
                    },
                };
                eprintln!("rerail: phase 0 seed {}: {why}", format!("{seed:?}").chars().take(300).collect::<String>());
            }
            let Some(g) = image.filter(|g| holds(g)) else {
                return true;
            };
            crate::code_rule_dsl::prof::count("phase0.graph_image_read", 1);
            accepted.insert(seed.clone(), Arc::new(g));
            false
        });
        for (g, seed) in found.iter_mut().zip(seeds) {
            if g.is_none() {
                *g = accepted.get(seed).cloned();
            }
        }
        kept.lock().expect("phase 0's graphs").extend(accepted.into_iter().map(|(seed, graph)| ((dirs.clone(), seed), graph)));
    }
    if !wanted.is_empty() {
        if std::env::var_os("RERAIL_PHASE0_TRACE").is_some() {
            eprintln!("rerail: phase 0 derives {} seeds (images {})", wanted.len(), if images.is_some() { "on" } else { "off" });
        }
        let whole = match derive(&wanted) {
            Err(PlanError::Cache(_)) if super::tape_cache_path().is_some() => {
                super::with_tape_cache_path(std::path::Path::new(":memory:"), None, || derive(&wanted))
            }
            result => result,
        }.map_err(|e| Error::Syntax(format!("phase 0: {e}")))?;
        let made: HashMap<Seed, Arc<Graph>> = wanted.iter().enumerate().map(|(k, seed)| (seed.clone(), Arc::new(whole.of(seed, k)))).collect();
        // Kept as images (decision 0246) whichever process derived them: an image is the engine build's, not the
        // process's. The image keeps the bootstrap run's plans (Q150), so this run and the next, which reads it, name
        // the same plans (`the_run_after_a_cold_one_writes_nothing`). Clearing them here as well (f7e047997, from
        // before images kept them) made the run that derived a graph name none and the run that read it name them.
        if let Some(images) = &images {
            for (seed, g) in &made {
                crate::code_rule_dsl::logic::rr::keep_graph_image(images.store.as_deref(), images.key(seed), images.encode(g));
            }
        }
        // A test store's template is filled in whichever test process asks first (`test_store`): its graphs are not
        // kept for the process, whose own runs then derived none, and prepared one program fewer than a process that
        // found the template made (the identity test's first child after a rebuild).
        #[cfg(any(test, feature = "test-hooks"))]
        let keep = !super::test_store::filling();
        #[cfg(not(any(test, feature = "test-hooks")))]
        let keep = true;
        if keep {
            let mut map = kept.lock().expect("phase 0's graphs");
            map.extend(made.iter().map(|(s, g)| ((dirs.clone(), s.clone()), Arc::clone(g))));
        }
        for (g, seed) in found.iter_mut().zip(seeds) {
            if g.is_none() {
                *g = made.get(seed).cloned();
            }
        }
    }
    Ok(found.into_iter().map(|g| g.expect("each seed's graph")).collect())
}

/// The include graphs of program files `paths`, derived by one run of `bootstrap.rerail` and kept: files read one by one
/// after it find theirs (`rr::read_file`, `parse_file`), as a pack's programs do in a run (`settings::program_texts`).
/// Read one by one with none kept, each file is a run of its own: a test over a pack's programs was hundreds of corpus
/// runs, most of its time spent waiting on the pool. A file that does not canonicalize, or a run that fails, is left
/// to its read, which derives its graph again and reports the error.
pub fn keep_files<'p>(paths: impl IntoIterator<Item = &'p std::path::PathBuf>) {
    let seeds: Vec<Seed> = paths.into_iter().filter_map(|p| p.canonicalize().ok()).map(Seed::File).collect();
    let _ = graphs(&seeds);
}

/// Keeps the graph of a text made from a program file (`tools::program_texts`), taken from the file's graph rather
/// than from another run of `bootstrap.rerail`: each name the text includes is a library module the file's graph holds,
/// found as `bootstrap.rerail` finds a text's names (the library alone). The graph is the one that run would give, so
/// its digest is too. False when a name is not such a module (the text then gets its own run when asked).
pub(in crate::code_rule_dsl) fn keep_text(seed: &Seed, file: &Graph, dirs: &Dirs) -> bool {
    let Seed::Text { t_shape, includes, phases } = seed else {
        return false;
    };
    let lib = &dirs.lib;
    let mut asks: Vec<(i64, String, PathBuf)> = Vec::with_capacity(includes.len() + 1);
    let names = includes.iter().cloned().chain((!t_shape).then(|| (-1, "core.rerail".to_owned())));
    for (at, name) in names {
        // `bootstrap.rerail`'s candidate: `cat(lib, "/", name)`, as written, not made canonical.
        let path = candidate_path(&lib.to_string_lossy(), &name);
        if !file.digests.contains_key(&path) {
            return false;
        }
        asks.push((at, name, path));
    }
    asks.sort();
    // The phases the text names are the file's (its declarations are the text's): their candidates, as
    // `bootstrap.rerail` makes them, each one the file's graph holds an answer for.
    let phase_lib = &dirs.phases;
    let mut named: Vec<(String, PathBuf)> = Vec::with_capacity(phases.len());
    for p in phases {
        let path = candidate_path(&phase_lib.to_string_lossy(), &format!("{p}.rerail"));
        if !file.digests.contains_key(&path) && !file.absent.contains(&path) {
            return false;
        }
        named.push((p.clone(), path));
    }
    let mut whole = file.clone();
    whole.includes.insert(text_key(0), asks);
    if !named.is_empty() {
        whole.phases.insert(text_key(0), named);
    }
    let graph = whole.of(seed, 0);
    crate::code_rule_dsl::logic::profile::shared::<PhaseZeroGraphs>().0.lock().expect("phase 0's graphs").insert((Some(dirs.clone()), seed.clone()), Arc::new(graph));
    true
}

/// The rule library and the engine's phase programs' directory, canonical, as `bootstrap.rerail` is handed them: found
/// once by a caller that keeps many texts (`keep_text`).
#[derive(Clone, Eq, PartialEq, Hash)]
pub(in crate::code_rule_dsl) struct Dirs {
    pub(in crate::code_rule_dsl) lib: PathBuf,
    pub(in crate::code_rule_dsl) phases: PathBuf,
}

impl Dirs {
    pub(in crate::code_rule_dsl) fn here() -> Option<Dirs> {
        Some(Dirs { lib: std::fs::canonicalize(crate::code_rule_dsl::logic::rr::library()).ok()?, phases: phase_lib()? })
    }
}

/// The engine's phase programs' directory (`phases::locate`'s builtin), canonical, as `bootstrap.rerail` is handed it.
pub(in crate::code_rule_dsl) fn phase_lib() -> Option<PathBuf> {
    std::fs::canonicalize(super::phases::builtin_dir()).ok()
}

/// Graphs as images (`rr::graph_image`): keyed by the engine's build and the seed, their paths written relative to the
/// library and the phase programs' directory, so a bundle made where those stand elsewhere is read here too (each file
/// is then checked by its digest, `Graph::holds`). The bootstrap run's plans are kept with it: a run that reads the
/// graph names them in its manifest, as the run that derived it did, so a warm run's manifest is the cold run's and
/// what the graph came from stays live while it is used (Q150).
struct Images {
    lib: String,
    phases: String,
    /// The engine build's image store (`rr::image_store`), opened once for the call's seeds.
    store: Option<Arc<crate::content_store::ContentStore>>,
}

impl Images {
    fn here() -> Option<Images> {
        let lib = std::fs::canonicalize(crate::code_rule_dsl::logic::rr::library()).ok()?;
        Some(Images { lib: format!("{}/", bootstrap_spelling(&lib.to_string_lossy())), phases: format!("{}/", bootstrap_spelling(&phase_lib()?.to_string_lossy())), store: crate::code_rule_dsl::logic::rr::image_store() })
    }

    /// A path or an includer, relative where it is under one of the two directories.
    fn rel(&self, s: &str) -> String {
        let s = bootstrap_spelling(s);
        if let Some(r) = s.strip_prefix(&self.lib) {
            format!("lib:{r}")
        } else if let Some(r) = s.strip_prefix(&self.phases) {
            format!("phase:{r}")
        } else {
            format!("abs:{s}")
        }
    }

    fn abs(&self, s: &str) -> Option<String> {
        if let Some(r) = s.strip_prefix("lib:") {
            Some(host_spelling(&format!("{}{r}", self.lib)))
        } else if let Some(r) = s.strip_prefix("phase:") {
            Some(host_spelling(&format!("{}{r}", self.phases)))
        } else {
            s.strip_prefix("abs:").map(host_spelling)
        }
    }

    fn key(&self, seed: &Seed) -> crate::content_store::HashKey {
        let seed = match seed {
            Seed::Text { t_shape, includes, phases } => {
                let includes: Vec<String> = includes.iter().map(|(at, n)| format!("{at}\u{1}{n}")).collect();
                format!("text\0{t_shape}\0{}\0{}", includes.join("\u{2}"), phases.join("\u{2}"))
            }
            Seed::File(p) => format!("file\0{}", self.rel(&p.to_string_lossy())),
        };
        crate::content_store::HashKey::sha256(format!("rerail phase 0 graph image 1\0{}\0{seed}", crate::code_rule_dsl::build_ids::engine()).as_bytes())
    }

    fn encode(&self, g: &Graph) -> Vec<u8> {
        let p = |p: &PathBuf| self.rel(&p.to_string_lossy());
        let hex = |d: &[u8; 32]| d.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let includes: Vec<String> = g.includes.iter().flat_map(|(from, to)| to.iter().flat_map(move |(at, n, path)| [self.rel(from), at.to_string(), n.clone(), p(path)])).collect();
        let missing: Vec<String> = g.missing.iter().flat_map(|(from, at, n)| [self.rel(from), at.to_string(), n.clone()]).collect();
        let absent: Vec<String> = g.absent.iter().map(p).collect();
        let digests: Vec<String> = g.digests.iter().flat_map(|(path, d)| [p(path), hex(d)]).collect();
        let phases: Vec<String> = g.phases.iter().flat_map(|(from, to)| to.iter().flat_map(move |(ph, path)| [self.rel(from), ph.clone(), p(path)])).collect();
        let rules: Vec<String> = g.rules.iter().flat_map(|(path, d)| [p(path), hex(d)]).collect();
        let plans: Vec<String> = g.plans.iter().map(hex).collect();
        crate::content_store::observe::encode_lists(&[&includes[..], &missing[..], &absent[..], &digests[..], &phases[..], &rules[..], &plans[..]])
    }

    fn decode(&self, bytes: &[u8]) -> Option<Graph> {
        let [includes, missing, absent, digests, phases, rules, plans]: [Vec<String>; 7] = crate::content_store::observe::decode_lists(bytes)?.try_into().ok()?;
        let unhex = |s: &str| -> Option<[u8; 32]> {
            let mut out = [0u8; 32];
            for (i, b) in out.iter_mut().enumerate() {
                *b = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
            }
            Some(out)
        };
        // An includer that is no file ("text", "engine") is kept as written.
        let from = |s: &str| self.abs(s).unwrap_or_else(|| s.to_owned());
        let mut g = Graph::default();
        for r in includes.chunks(4) {
            let [f, at, n, path] = r else { return None };
            g.includes.entry(from(f)).or_default().push((at.parse().ok()?, n.clone(), PathBuf::from(self.abs(path)?)));
        }
        for r in missing.chunks(3) {
            let [f, at, n] = r else { return None };
            g.missing.push((from(f), at.parse().ok()?, n.clone()));
        }
        g.absent = absent.iter().map(|p| self.abs(p).map(PathBuf::from)).collect::<Option<_>>()?;
        for r in digests.chunks(2) {
            let [path, d] = r else { return None };
            g.digests.insert(PathBuf::from(self.abs(path)?), unhex(d)?);
        }
        for r in phases.chunks(3) {
            let [f, ph, path] = r else { return None };
            g.phases.entry(from(f)).or_default().push((ph.clone(), PathBuf::from(self.abs(path)?)));
        }
        for r in rules.chunks(2) {
            let [path, d] = r else { return None };
            g.rules.insert(PathBuf::from(self.abs(path)?), unhex(d)?);
        }
        g.plans = plans.iter().map(|d| unhex(d)).collect::<Option<_>>()?;
        Some(g)
    }
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
    /// [`TEXT`]: the graph a run of `bootstrap.rerail` over that seed alone gives, whatever seeds shared the run. A module
    /// names `core.rerail` only as a file seed (`p0_from(f, -1, "core.rerail") :- p0_file(f), …`), so another seed's module
    /// does not here; and `p0_absent` names no includer, so the seed's absent candidates are those of the names its
    /// modules ask (`p0_cand`). Taken whole, a text's graph held every seed's absent files and the implicit includes
    /// of the modules that were seeds too, and its digest depended on the run it was derived in.
    pub fn of(&self, seed: &Seed, k: usize) -> Graph {
        let start = match seed {
            Seed::Text { .. } => text_key(k),
            Seed::File(p) => p.to_string_lossy().into_owned(),
        };
        let lib = std::fs::canonicalize(crate::code_rule_dsl::logic::rr::library()).ok().map(|l| l.to_string_lossy().into_owned());
        let mut candidates: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();
        let mut candidate = |from: &str, name: &str| {
            if let Some(l) = &lib {
                candidates.insert(candidate_path(l, name));
            }
            if !from.starts_with("text:") && from != "engine" && let Some((dir, _)) = bootstrap_spelling(from).rsplit_once('/') {
                candidates.insert(candidate_path(dir, name));
            }
        };
        let own = |from: &str, at: i64, name: &str| from == start || !(at == -1 && name == "core.rerail");
        let mut g = Graph { plans: self.plans.clone(), ..Graph::default() };
        let mut pending = vec![start.clone(), "engine".to_owned()];
        let mut seen = std::collections::BTreeSet::new();
        while let Some(from) = pending.pop() {
            if !seen.insert(from.clone()) {
                continue;
            }
            let name = if from == start && matches!(seed, Seed::Text { .. }) { TEXT.to_owned() } else { from.clone() };
            if let Some(to) = self.includes.get(&from) {
                let to: Vec<(i64, String, PathBuf)> = to.iter().filter(|(at, n, _)| own(&from, *at, n)).cloned().collect();
                for (_, n, path) in &to {
                    candidate(&from, n);
                    pending.push(path.to_string_lossy().into_owned());
                    if let Some(d) = self.digests.get(path) {
                        g.digests.insert(path.clone(), *d);
                    }
                }
                if !to.is_empty() {
                    g.includes.insert(name.clone(), to);
                }
            }
            for (_, at, n) in self.missing.iter().filter(|(f, at, n)| *f == from && own(&from, *at, n)) {
                candidate(&from, n);
                g.missing.push((name.clone(), *at, n.clone()));
            }
        }
        // The rule files reached through a phase program (`rules`): from every node the includes reached, its
        // phases' candidates (absent ones kept, as a file appearing there changes the list), and from each phase
        // program found, its includes and its phases in turn.
        let mut pending: Vec<String> = seen.iter().cloned().collect();
        let mut seen_rules = std::collections::BTreeSet::new();
        let mut phase_candidates: Vec<PathBuf> = Vec::new();
        while let Some(from) = pending.pop() {
            for (_, path) in self.phases.get(&from).into_iter().flatten() {
                phase_candidates.push(path.clone());
                if let Some(d) = self.digests.get(path) {
                    if !g.digests.contains_key(path) {
                        g.rules.insert(path.clone(), *d);
                    }
                    let next = path.to_string_lossy().into_owned();
                    if seen_rules.insert(next.clone()) && !seen.contains(&next) {
                        pending.push(next);
                    }
                }
            }
            // A phase program's includes, past the seed's own (which `includes` holds).
            if seen_rules.contains(&from) {
                for (_, n, path) in self.includes.get(&from).into_iter().flatten() {
                    candidate(&from, n);
                    if let Some(d) = self.digests.get(path) {
                        if !g.digests.contains_key(path) {
                            g.rules.insert(path.clone(), *d);
                        }
                    }
                    let next = path.to_string_lossy().into_owned();
                    if seen_rules.insert(next.clone()) && !seen.contains(&next) {
                        pending.push(next);
                    }
                }
            }
        }
        candidates.extend(phase_candidates);
        if let Some(t) = self.phases.get(&start).filter(|_| matches!(seed, Seed::Text { .. })) {
            g.phases.insert(TEXT.to_owned(), t.clone());
        }
        g.absent = self.absent.iter().filter(|p| candidates.contains(*p)).cloned().collect();
        if let Seed::File(p) = seed
            && let Some(d) = self.digests.get(p)
        {
            g.digests.insert(p.clone(), *d);
        }
        g
    }
}

/// Runs `bootstrap.rerail` over the seeds' facts, as a corpus run with no corpus: every document it reads it loads.
pub fn derive(seeds: &[Seed]) -> Result<Graph> {
    let lib = crate::code_rule_dsl::logic::rr::library();
    let lib = std::fs::canonicalize(&lib).map_err(|e| PlanError::Plan(format!("{}: {e}", lib.display())))?;
    let bootstrap = lib.join("bootstrap.rerail");
    // The builtin read: the one rule file the engine knows by itself.
    let (bytes, _) = super::observe(|o| o.read(&bootstrap)).ok_or_else(|| PlanError::Plan(format!("{}: cannot read the file", bootstrap.display())))?;
    let mut text = String::from_utf8(bytes).map_err(|e| PlanError::Plan(format!("{}: {e}", bootstrap.display())))?;
    text.push_str("\n// The engine's builtin facts.\n");
    #[cfg(windows)]
    text.push_str("env(\"rerail.windows_paths\", \"1\").\n");
    text.push_str(&format!("env(\"rerail.lib\", {}).\n", quote(&bootstrap_spelling(&lib.to_string_lossy()))));
    // The library's parent is a root too: packs include their tables beside the library (`#include
    // "../packs/python/….names.rerail"`), which the resolver before phase 0 read, and the loader confines reads to roots.
    if let Some(parent) = lib.parent() {
        text.push_str(&format!("env(\"rerail.library_parent\", {}).\n", quote(&bootstrap_spelling(&parent.to_string_lossy()))));
    }
    // The phase programs' directory, where a declaration's phase is found (`p0_phase`).
    if let Some(dir) = phase_lib() {
        text.push_str(&format!("env(\"rerail.phase_lib\", {}).\n", quote(&bootstrap_spelling(&dir.to_string_lossy()))));
    }
    let mut dirs = std::collections::BTreeSet::new();
    for (k, seed) in seeds.iter().enumerate() {
        match seed {
            Seed::Text { t_shape, includes, phases } => {
                let t = quote(&text_key(k));
                text.push_str(&format!("p0_text({t}).\n"));
                for p in phases {
                    text.push_str(&format!("p0_text_phase({t}, {}).\n", quote(p)));
                }
                if *t_shape {
                    text.push_str(&format!("p0_t_shape({t}).\n"));
                }
                for (at, name) in includes {
                    text.push_str(&format!("p0_ask({t}, {at}, {}).\n", quote(name)));
                }
            }
            Seed::File(path) => {
                text.push_str(&format!("p0_file({}).\n", quote(&bootstrap_spelling(&path.to_string_lossy()))));
                // The file's own directory is a root the loader may read: its includes beside it are files there.
                if let Some(dir) = path.parent() {
                    dirs.insert(dir.to_path_buf());
                }
            }
        }
    }
    // The roots the loader may read (`world::World::new`: each setting that is a directory), canonical.
    let roots: Vec<PathBuf> = [Some(lib.clone()), lib.parent().map(std::path::Path::to_path_buf), phase_lib()]
        .into_iter()
        .flatten()
        .chain(dirs.iter().cloned())
        .filter_map(|d| std::fs::canonicalize(d).ok())
        .collect();
    for dir in dirs {
        text.push_str(&format!("env(\"rerail.program_dir\", {}).\n", quote(&bootstrap_spelling(&dir.to_string_lossy()))));
    }
    // The library is already a permitted input root. Phase zero only loads the files its seeds request.
    let options = serde_json::json!({ "reuse": true, "inputFiles": [] });
    let mut image = Vec::new();
    let phase_0 = in_phase_0();
    let run = super::run_handed(&[&text], None, None, &lib, None, std::path::Path::new(""), "program",
        crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18, &super::Typing::default(), &options, Some(&mut image), Default::default());
    drop(phase_0);
    let result = run.and_then(|report| {
        use crate::code_rule_dsl::logic::plan::output::Value as V;
        let mut g = Graph { plans: report.plans, ..Graph::default() };
        // The outputs' image, read where it is: each value typed, nothing parsed.
        let outputs = crate::code_rule_dsl::logic::plan::output::Outputs::of(&image).map_err(PlanError::Plan)?;
        for r in outputs.rows("p0_include") {
            if let (V::Text(from), Some(at), V::Text(name), V::Text(path)) = (r.get(0), r.get(1).int(), r.get(2), r.get(3)) {
                g.includes.entry(host_spelling(from)).or_default().push((at, name.to_owned(), PathBuf::from(host_spelling(path))));
            }
        }
        for v in g.includes.values_mut() {
            v.sort();
        }
        for r in outputs.rows("p0_missing") {
            if let (V::Text(from), Some(at), V::Text(name)) = (r.get(0), r.get(1).int(), r.get(2)) {
                g.missing.push((host_spelling(from), at, name.to_owned()));
            }
        }
        g.missing.sort();
        g.absent = outputs.rows("p0_absent").iter().filter_map(|r| r.get(0).text().map(host_spelling).map(PathBuf::from)).collect();
        // A candidate whose directory lies outside every root (`rules/../packs/…`, beside a rule file outside the
        // library) was refused, not looked at: its listing came back empty. It is no absent file the graph depends
        // on, and `holds`, which stats it, found it whenever it was there: such a graph never held, and phase 0 ran
        // again on every run, warm ones too (its first wave read every frame of the store, 250 MB).
        g.absent.retain(|p| p.parent().and_then(|d| std::fs::canonicalize(d).ok()).is_none_or(|d| roots.iter().any(|r| d.starts_with(r))));
        for r in outputs.rows("p0_phase") {
            if let (V::Text(from), V::Text(phase), V::Text(path)) = (r.get(0), r.get(1), r.get(2)) {
                g.phases.entry(host_spelling(from)).or_default().push((phase.to_owned(), PathBuf::from(host_spelling(path))));
            }
        }
        for v in g.phases.values_mut() {
            v.sort();
            v.dedup();
        }
        drop(outputs);
        let files = seeds.iter().filter_map(|s| match s {
            Seed::File(p) => Some(p.clone()),
            Seed::Text { .. } => None,
        });
        let found: Vec<PathBuf> = g.phases.values().flatten().map(|(_, p)| p.clone()).filter(|p| !g.absent.contains(p)).collect();
        let modules: Vec<PathBuf> = g.includes.values().flatten().map(|(_, _, p)| p.clone()).chain(files).chain(found).collect();
        for p in modules {
            if let Some(d) = super::observe(|o| o.digest(&p)) {
                g.digests.insert(p, d);
            }
        }
        Ok(g)
    });
    result
}

// Its tests run in the engine's test binary (its facade of this module mounts `tests.rs`): they read what the
// engine registers as it loads (the languages, their `.rerail` libraries).


#[cfg(test)]
pub mod benchmarks {
    /// Phase 0 over every `.rerail` file of `RERAIL_P0_RULES` (a pack's directory), as a pack's run reads them
    /// (`tools::program_texts`): one run of `bootstrap.rerail` for all. Run it twice for the warm figure.
    #[test]
    #[ignore = "a benchmark: RERAIL_P0_RULES=<a pack's directory>"]
    fn benchmark_phase_0_rules() {
        let dir = std::path::PathBuf::from(std::env::var("RERAIL_P0_RULES").expect("RERAIL_P0_RULES"));
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "rerail")).collect();
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

