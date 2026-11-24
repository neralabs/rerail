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

use rerail_blocks::vector::Vector;
use rerail_store::Canonical as _;
use super::{Error as PlanError, Result};
use crate::code_rule_dsl::logic::front::Error;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
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

/// The directory of the file a text's `#include` names from library `lib` (canonical), where that file is outside
/// the library: a pack's own library, named by its path from the library (`settings::module_text`).
fn outside_library(lib: &Path, name: &str) -> Option<PathBuf> {
    if !name.starts_with("..") {
        return None;
    }
    let file = dunce::canonicalize(candidate_path(&lib.to_string_lossy(), name)).ok()?;
    (!file.starts_with(lib)).then(|| file.parent().map(Path::to_path_buf)).flatten()
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
    pub digests: BTreeMap<PathBuf, [u8; 32]>,
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
    pub plans: Vector<[u8; 32]>,
    /// The module plan (decision 0639 §1, `bootstrap.rerail`'s `p0_module_*`): each module the seed's includes reach,
    /// by its name (its variant and its members' digests, `core:<d>+<d>`), with its rank, its key and its scope. Made
    /// of the graph's digests and edges, so not part of its digest.
    pub plan: BTreeMap<String, PlanModule>,
    /// Each planned module's image, by its key (`p0_module_lowered`, decision 0639 §3): the digest of the image the
    /// image store keeps under the key. A graph holds only while its modules' images are there (`Looks::holds`).
    pub lowered: BTreeMap<String, String>,
    /// Each planned module's tail, by its tail key (`p0_module_tailed`, decision 0639 §6a): the digest of the image the
    /// image store keeps under the tail key. A graph holds only while those images are there too.
    pub tailed: BTreeMap<String, String>,
}

/// A module of the plan (`Graph::plan`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlanModule {
    /// The files of its closure (`p0_module_rank`): it ranks above every module in its scope.
    pub rank: i64,
    /// Its key (`p0_module_key`), hex: the engine's build, its name, and its closure's digests and edges.
    pub key: String,
    /// The modules it is lowered against (`p0_scope`).
    pub scope: std::collections::BTreeSet<String>,
    /// Its tail key (`p0_tail_key`), hex: its key and its scope's files' digests.
    pub tail: String,
}

impl PlanModule {
    /// The digests of a module's files, hex, from its name.
    pub fn members(name: &str) -> impl Iterator<Item = &str> {
        name.split_once(':').map_or("", |(_, ds)| ds).split('+').filter(|d| !d.is_empty())
    }
}

impl Graph {
    /// Whether every module and every absent candidate is as it was (`Looks::holds`, for this graph alone).
    #[cfg(any(test, feature = "test-hooks"))]
    fn holds(&self) -> bool {
        Looks::default().holds(self)
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

    /// Where it differs from `other`, for a check's message (`RERAIL_PHASE0_COMPOSE=check`, the tests of `Graph::of`):
    /// each part's entries only one of the two holds.
    pub fn differences(&self, other: &Graph) -> String {
        fn diff<T: Ord + Clone + std::fmt::Debug>(what: &str, a: impl IntoIterator<Item = T>, b: impl IntoIterator<Item = T>) -> String {
            let (a, b): (std::collections::BTreeSet<T>, std::collections::BTreeSet<T>) = (a.into_iter().collect(), b.into_iter().collect());
            let (only_a, only_b): (Vec<&T>, Vec<&T>) = (a.difference(&b).collect(), b.difference(&a).collect());
            if only_a.is_empty() && only_b.is_empty() { String::new() } else { format!("\n{what}: this one's only {only_a:?}; the other's only {only_b:?}") }
        }
        let edges = |g: &Graph| g.includes.iter().flat_map(|(f, t)| t.iter().map(move |e| (f.clone(), e.clone()))).collect::<Vec<_>>();
        [
            diff("includes", edges(self), edges(other)),
            diff("missing", self.missing.clone(), other.missing.clone()),
            diff("absent", self.absent.clone(), other.absent.clone()),
            diff("digests", self.digests.iter().map(|(p, d)| (p.clone(), *d)), other.digests.iter().map(|(p, d)| (p.clone(), *d))),
            diff("rules", self.rules.iter().map(|(p, d)| (p.clone(), *d)), other.rules.iter().map(|(p, d)| (p.clone(), *d))),
            diff("phases", self.phases.iter().map(|(k, v)| (k.clone(), v.clone())), other.phases.iter().map(|(k, v)| (k.clone(), v.clone()))),
            diff("lowered", self.lowered.clone(), other.lowered.clone()),
            diff("tailed", self.tailed.clone(), other.tailed.clone()),
            diff("plan", self.plan.iter().map(|(k, v)| (k.clone(), v.rank, v.key.clone(), v.scope.clone(), v.tail.clone())), other.plan.iter().map(|(k, v)| (k.clone(), v.rank, v.key.clone(), v.scope.clone(), v.tail.clone()))),
        ]
        .concat()
    }

    /// Every module, and every candidate found absent, as `rr::LibraryFile`s check them.
    pub(in crate::code_rule_dsl) fn files(&self) -> Vec<(PathBuf, Option<[u8; 32]>)> {
        self.digests.iter().map(|(p, d)| (p.clone(), Some(*d))).chain(self.absent.iter().map(|p| (p.clone(), None))).collect()
    }
}

/// The files a call of `graphs` looked at, each once for all its graphs: a pack's graphs share their modules (the
/// library's), and asked graph by graph the observer answered each module once a graph (316 graphs, 0.42 G of a warm
/// ep2 run's 1.9 G, checking the graphs the same process had checked a moment before).
#[derive(Default)]
struct Looks {
    digests: HashMap<PathBuf, Option<[u8; 32]>>,
    missing: HashMap<PathBuf, bool>,
    images: HashMap<String, bool>,
}

impl Looks {
    /// Whether every module of `g` and every absent candidate is as it was: a look at each file this call has not
    /// looked at yet.
    fn holds(&mut self, g: &Graph) -> bool {
        g.digests.iter().chain(&g.rules).all(|(p, d)| self.digests.entry(p.clone()).or_insert_with(|| super::observe(|o| o.digest(p))).as_ref() == Some(d))
            && g.absent.iter().all(|p| *self.missing.entry(p.clone()).or_insert_with(|| super::observe(|o| o.kind(p)) == crate::content_store::observe::Kind::Missing))
            // Each module's image in the store the run reads: one another store holds is lowered again here.
            && g.lowered.keys().all(|k| *self.images.entry(k.clone()).or_insert_with(|| crate::code_rule_dsl::logic::rr::planned_module_held(k)))
            && g.tailed.keys().all(|t| *self.images.entry(format!("tail {t}")).or_insert_with(|| crate::code_rule_dsl::logic::rr::planned_tail_held(t)))
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

/// The graphs' images this process retained, by the image store's path (`rr::image_store_path`) and the image's key: a
/// graph held in the process is retained once in each store a run reads (the store that evaluates a working tree's
/// policies keeps them), not at every lookup, which opened the image store and asked it for the image each time
/// (1,263 lookups, 0.85 s of ep2's edit).
#[derive(Default)]
struct PhaseZeroRetained(Mutex<std::collections::HashSet<(Option<PathBuf>, crate::content_store::HashKey)>>);

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
    // Each file is looked at once in this call, for the kept graphs and the images alike.
    let mut looks = Looks::default();
    let stale: Vec<&Seed> = seeds.iter().zip(found.iter_mut()).filter(|(_, g)| g.as_ref().is_some_and(|g| !looks.holds(g))).map(|(s, g)| {
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
        let at = crate::code_rule_dsl::logic::rr::image_store_path();
        let retained = crate::code_rule_dsl::logic::profile::shared::<PhaseZeroRetained>();
        // Retained together: one read of the keys kept, their leases renewed together, and those not kept written
        // as one record (`keep_graph_images`), so they lie in one segment.
        let fresh: Vec<(crate::content_store::HashKey, &Graph)> = {
            let mut held = retained.0.lock().expect("phase 0's retained images");
            seeds.iter().zip(&found).filter_map(|(seed, graph)| Some((images.key(seed), graph.as_deref()?))).filter(|(key, _)| held.insert((at.clone(), *key))).collect()
        };
        let fresh: Vec<(crate::content_store::HashKey, &Graph)> = fresh.into_iter().filter(|(key, _)| !crate::code_rule_dsl::logic::rr::graph_shipped(*key)).collect();
        crate::code_rule_dsl::logic::rr::retain_graph_facts(images.store(), fresh.iter().map(|(key, _)| images.store_key(key)).collect(), |i| images.facts(fresh[i].1));
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
        // The images in one read of the store, not one a seed.
        let keys: Vec<crate::content_store::HashKey> = wanted.iter().map(|seed| images.key(seed)).collect();
        // The engine's shipped graphs first (images), then the store's (kept facts, read together).
        let mut graphs: Vec<Option<Graph>> = crate::code_rule_dsl::logic::rr::shipped_graph_images(&keys, |b| images.decode(b));
        let rest: Vec<usize> = (0..keys.len()).filter(|&k| graphs[k].is_none()).collect();
        if !rest.is_empty() {
            let store_keys: Vec<crate::content_store::HashKey> = rest.iter().map(|&k| images.store_key(&keys[k])).collect();
            let kept = crate::code_rule_dsl::logic::rr::graph_facts(images.store(), &store_keys);
            // The graphs share the library's modules: each path and digest read once for all of them.
            let _s = crate::code_rule_dsl::logic::profile::span(|| "the graphs, of their facts".into());
            let mut read = Read::default();
            for (k, facts) in rest.into_iter().zip(kept) {
                graphs[k] = facts.and_then(|f| images.graph_of(&f, &mut read));
            }
        }
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
            let Some(g) = image.filter(|g| looks.holds(g)) else {
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
    // A text whose names are each a library module, and whose phases each a phase program or no file, has the graph
    // of its modules' and phase programs' own (`composed`): no run of `bootstrap.rerail`. A test's program is such a
    // text, a new one in each test's process, and each derived its graph in a run of its own (on m3, 9.6 k of the
    // corpus suite's 125 k busy samples under `derive`).
    let compose = std::env::var("RERAIL_PHASE0_COMPOSE").unwrap_or_default();
    // Only where every seed wanted is a text or a program file of the run's own: the library's modules wanted with them
    // (a store's template, filled) are one run, which their texts share.
    let dirs_here = Dirs::here();
    let own_seed = |s: &Seed| match s {
        Seed::Text { .. } => true,
        Seed::File(p) => dirs_here.as_ref().is_some_and(|d| !p.starts_with(&d.lib) && !p.starts_with(&d.phases)),
    };
    if !wanted.is_empty() && compose != "off" && wanted.iter().all(own_seed) && let Some(dirs) = dirs_here.clone() {
        let mut made = HashMap::new();
        for seed in &wanted {
            let Some(g) = composed(seed, &dirs)? else { continue };
            // `RERAIL_PHASE0_COMPOSE=check`: the composed graph against the one its own run derives.
            if compose == "check" {
                let derived = derive(std::slice::from_ref(seed)).map_err(|e| Error::Syntax(format!("phase 0: {e}")))?.of(seed, 0);
                assert!(same_graph(&g, &derived), "{seed:?}: the composed graph is the derived one: {}", g.differences(&derived));
            }
            crate::code_rule_dsl::prof::count("phase0.graph_composed", 1);
            made.insert(seed.clone(), Arc::new(g));
        }
        wanted.retain(|s| !made.contains_key(s));
        for (g, seed) in found.iter_mut().zip(seeds) {
            if g.is_none() {
                *g = made.get(seed).cloned();
            }
        }
        kept.lock().expect("phase 0's graphs").extend(made.into_iter().map(|(seed, graph)| ((Some(dirs.clone()), seed), graph)));
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
        let lib = dunce::canonicalize(crate::code_rule_dsl::logic::rr::library()).ok();
        let made: HashMap<Seed, Arc<Graph>> = wanted.iter().enumerate().map(|(k, seed)| (seed.clone(), Arc::new(whole.of_in(seed, k, lib.as_deref())))).collect();
        // Kept as images (decision 0246) whichever process derived them: an image is the engine build's, not the
        // process's. The image keeps the bootstrap run's plans (Q150), so this run and the next, which reads it, name
        // the same plans (`the_run_after_a_cold_one_writes_nothing`). Clearing them here as well (f7e047997, from
        // before images kept them) made the run that derived a graph name none and the run that read it name them.
        if let Some(images) = &images {
            crate::code_rule_dsl::logic::rr::export_graph_images(&made.iter().map(|(seed, g)| (images.key(seed), images.encode(g))).collect::<Vec<_>>());
            crate::code_rule_dsl::logic::rr::keep_graph_facts(images.store(), made.iter().map(|(seed, g)| (images.store_key(&images.key(seed)), images.facts(g))).collect());
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
    let seeds: Vec<Seed> = paths.into_iter().filter_map(|p| p.canonical().ok()).map(Seed::File).collect();
    let _ = graphs(&seeds);
}

/// Keeps the graph of a text made from a program file (`tools::program_texts`), taken from the file's graph rather
/// than from another run of `bootstrap.rerail`: each name the text includes is a library module the file's graph holds,
/// found as `bootstrap.rerail` finds a text's names (the library alone). The graph is the one that run would give, so
/// its digest is too. False when a name is not such a module (the text then gets its own run when asked).
pub(in crate::code_rule_dsl) fn keep_text(seed: &Seed, file: &Graph, dirs: &Dirs) -> bool {
    // A text in T's shape plans its modules without `core.rerail` (the plan's `bare` variant), which a file's graph does
    // not hold: its own run derives them.
    let Seed::Text { t_shape: false, includes, phases } = seed else {
        return false;
    };
    let lib = &dirs.lib;
    let mut asks: Vec<(i64, String, PathBuf)> = Vec::with_capacity(includes.len() + 1);
    let names = includes.iter().cloned().chain([(-1, "core.rerail".to_owned())]);
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
    let graph = whole.of_in(seed, 0, Some(lib));
    crate::code_rule_dsl::logic::profile::shared::<PhaseZeroGraphs>().0.lock().expect("phase 0's graphs").insert((Some(dirs.clone()), seed.clone()), Arc::new(graph));
    true
}

/// Text `seed`'s graph made of its modules' own (`Seed::File`, each found as `graphs` finds it: kept, or an image), as
/// a run of `bootstrap.rerail` over the text alone gives it (`keep_text`, from one file's graph): each name it includes
/// (and `core.rerail`, outside T's shape) is a file of the library, and each phase it names is a phase program, whose
/// graph is taken too, or no file (an absent candidate). `None` where a name is no file of the library: its run
/// derives it, and reports a missing one. The modules' graphs each hold what their modules reach through their
/// phases (`rules`) and the candidates their includes found absent, which the text's graph holds as well.
fn composed(seed: &Seed, dirs: &Dirs) -> std::result::Result<Option<Graph>, Error> {
    use crate::content_store::observe::Kind;
    // The seed's node in the graph, its names (offset, name; `core.rerail` at -1 outside T's shape) and phases, and,
    // for a file, its digest.
    let (start, names, phases, own): (String, Vec<(i64, String)>, Vec<String>, Option<(PathBuf, [u8; 32])>) = match seed {
        Seed::Text { t_shape, includes, phases } => {
            (text_key(0), includes.iter().cloned().chain((!t_shape).then(|| (-1, "core.rerail".to_owned()))).collect(), phases.clone(), None)
        }
        // A program file of the run's own, beside no module of the library or the phase programs (a policy, a
        // test's rule file): those are seeds of the template's run, which `graphs` derives in one run for all.
        Seed::File(path) => {
            if path.starts_with(&dirs.lib) || path.starts_with(&dirs.phases) {
                return Ok(None);
            }
            let (Some((bytes, _)), Some(digest)) = (super::observe(|o| o.read(path)), super::observe(|o| o.digest(path))) else { return Ok(None) };
            let Ok(text) = String::from_utf8(bytes) else { return Ok(None) };
            let Ok((includes, t_shape, phases)) = crate::code_rule_dsl::logic::rr::includes_and_phases(&text) else { return Ok(None) };
            let names = includes.into_iter().chain((!t_shape).then(|| (-1, "core.rerail".to_owned()))).collect();
            (path.to_string_lossy().into_owned(), names, phases, Some((path.clone(), digest)))
        }
    };
    // A program in T's shape reads its files without `core.rerail`: its modules are the plan's `bare` variant, which
    // the library's own graphs (each a file seed, read with it) do not hold. Its run derives them.
    if !names.iter().any(|(at, n)| *at == -1 && n == "core.rerail") {
        return Ok(None);
    }
    let lib = dirs.lib.to_string_lossy();
    let mut asks: Vec<(i64, String, PathBuf)> = Vec::with_capacity(names.len());
    let mut files: Vec<Seed> = Vec::new();
    let mut absent: Vec<PathBuf> = Vec::new();
    for (at, name) in names {
        // A name outside the library (a pack's own library) is a file of no library graph: the run's to resolve.
        if outside_library(&dirs.lib, &name).is_some() {
            return Ok(None);
        }
        let path = candidate_path(&lib, &name);
        if super::observe(|o| o.kind(&path)) != Kind::File {
            return Ok(None);
        }
        // Each name a file has (its own `core.rerail` too) has its candidate beside the file, held absent
        // (`p0_absent`: every candidate not there); one there is the run's to resolve.
        if let Some((own_path, _)) = &own {
            let Some((dir, _)) = bootstrap_spelling(&own_path.to_string_lossy()).rsplit_once('/').map(|(d, f)| (d.to_owned(), f.to_owned())) else { return Ok(None) };
            let beside = candidate_path(&dir, &name);
            if super::observe(|o| o.kind(&beside)) != Kind::Missing {
                return Ok(None);
            }
            absent.push(beside);
        }
        files.push(Seed::File(path.clone()));
        asks.push((at, name, path));
    }
    asks.sort();
    let phase_lib = dirs.phases.to_string_lossy();
    let mut named: Vec<(String, PathBuf)> = Vec::with_capacity(phases.len());
    for p in &phases {
        let path = candidate_path(&phase_lib, &format!("{p}.rerail"));
        match super::observe(|o| o.kind(&path)) {
            Kind::File => files.push(Seed::File(path.clone())),
            Kind::Missing => absent.push(path.clone()),
            _ => return Ok(None),
        }
        named.push((p.clone(), path));
    }
    named.sort();
    named.dedup();
    files.sort();
    files.dedup();
    let parts = graphs(&files)?;
    let mut whole = Graph::default();
    for g in &parts {
        for (from, to) in &g.includes {
            whole.includes.entry(from.clone()).or_insert_with(|| to.clone());
        }
        whole.missing.extend(g.missing.iter().cloned());
        whole.absent.extend(g.absent.iter().cloned());
        whole.digests.extend(g.digests.iter().map(|(p, d)| (p.clone(), *d)));
        whole.rules.extend(g.rules.iter().map(|(p, d)| (p.clone(), *d)));
        whole.plans.extend(g.plans.iter().copied());
        whole.plan.extend(g.plan.iter().map(|(m, row)| (m.clone(), row.clone())));
        whole.lowered.extend(g.lowered.iter().map(|(k, d)| (k.clone(), d.clone())));
        whole.tailed.extend(g.tailed.iter().map(|(k, d)| (k.clone(), d.clone())));
    }
    whole.absent.extend(absent);
    whole.includes.insert(start.clone(), asks);
    if !named.is_empty() {
        whole.phases.insert(start, named);
    }
    if let Some((path, digest)) = own {
        whole.digests.insert(path, digest);
    }
    let mut g = whole.of(seed, 0);
    // What the modules reach through their phases, and the candidates they found absent: their graphs' (`of` reads a
    // module's phases only from a run's graph, which theirs hold as `rules`, with the candidates beside what those
    // reach). But one: a module taken as a file seed names `core.rerail` itself (`<its dir>/core.rerail`, absent), which
    // a module a text includes does not, unless a module beside it names `core.rerail` too.
    for (part, seed) in parts.iter().zip(&files) {
        g.rules.extend(part.rules.iter().filter(|(p, _)| !g.digests.contains_key(*p)).map(|(p, d)| (p.clone(), *d)));
        let Seed::File(start) = seed else { continue };
        let start_key = start.to_string_lossy().into_owned();
        let dir = |from: &str| bootstrap_spelling(from).rsplit_once('/').map(|(d, _)| d.to_owned());
        let own_dir = dir(&start_key);
        let named = part.includes.iter().any(|(from, to)| {
            to.iter().any(|(at, n, _)| n == "core.rerail" && !(*from == start_key && *at == -1) && dir(from) == own_dir)
        });
        let implicit = own_dir.as_deref().map(|d| candidate_path(d, "core.rerail"));
        g.absent.extend(part.absent.iter().filter(|p| named || Some(*p) != implicit.as_ref()).cloned());
    }
    // The phase programs the text names, which `of` reached as `rules`: a phase program that is one of the text's
    // modules too is a module.
    g.rules.retain(|p, _| !g.digests.contains_key(p));
    g.absent.sort();
    g.absent.dedup();
    g.missing.sort();
    g.missing.dedup();
    g.plans.sort();
    g.plans.dedup();
    Ok(Some(g))
}

/// `composed`, with this checkout's directories, for the engine's tests of phase 0 (`phase0::tests`).
#[cfg(any(test, feature = "test-hooks"))]
pub fn composed_here(seed: &Seed) -> std::result::Result<Option<Graph>, Error> {
    composed(seed, &Dirs::here().expect("the library and the phase programs' directory"))
}

/// Whether two graphs of a seed are one: their digest (modules and edges), what they reach through phases, and the
/// candidates found absent (`RERAIL_PHASE0_COMPOSE=check`). The plans are the runs' that made them, not the graph.
pub fn same_graph(a: &Graph, b: &Graph) -> bool {
    let sorted = |v: &[PathBuf]| {
        let mut v = v.to_vec();
        v.sort();
        v.dedup();
        v
    };
    a.digest() == b.digest() && a.rules == b.rules && sorted(&a.absent) == sorted(&b.absent) && a.phases == b.phases && a.plan == b.plan && a.lowered == b.lowered && a.tailed == b.tailed
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
        Some(Dirs { lib: dunce::canonicalize(crate::code_rule_dsl::logic::rr::library()).ok()?, phases: phase_lib()? })
    }
}

/// The engine's phase programs' directory (`phases::locate`'s builtin), canonical, as `bootstrap.rerail` is handed it.
pub(in crate::code_rule_dsl) fn phase_lib() -> Option<PathBuf> {
    dunce::canonicalize(super::phases::builtin_dir()).ok()
}

/// Graphs as images (`rr::graph_images`): keyed by the engine's build and the seed, their paths written relative to the
/// library and the phase programs' directory, so a bundle made where those stand elsewhere is read here too (each file
/// is then checked by its digest, `Graph::holds`). The bootstrap run's plans are kept with it: a run that reads the
/// graph names them in its manifest, as the run that derived it did, so a warm run's manifest is the cold run's and
/// what the graph came from stays live while it is used (Q150).
struct Images {
    lib: String,
    phases: String,
    /// The engine build's image store (`rr::image_store`), opened once for the call's seeds when one is asked of it.
    store: OnceLock<Option<Arc<crate::content_store::ContentStore>>>,
}

impl Images {
    fn here() -> Option<Images> {
        let lib = dunce::canonicalize(crate::code_rule_dsl::logic::rr::library()).ok()?;
        Some(Images { lib: format!("{}/", bootstrap_spelling(&lib.to_string_lossy())), phases: format!("{}/", bootstrap_spelling(&phase_lib()?.to_string_lossy())), store: OnceLock::new() })
    }

    /// The image store, opened on the first ask.
    fn store(&self) -> Option<&crate::content_store::ContentStore> {
        self.store.get_or_init(crate::code_rule_dsl::logic::rr::image_store).as_deref()
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
        crate::content_store::HashKey::sha256(format!("rerail phase 0 graph image 4\0{}\0{seed}", crate::code_rule_dsl::build_ids::engine()).as_bytes())
    }

    /// The key a store keeps seed key `key`'s graph under, as kept facts (`rr::graph_facts`): the image's key is the
    /// shipped bundle's.
    fn store_key(&self, key: &crate::content_store::HashKey) -> crate::content_store::HashKey {
        crate::content_store::HashKey::sha256(&[b"rerail phase 0 graph facts 2 (the module plan)\0".as_slice(), &key.sha256_bytes().unwrap_or_default()].concat())
    }

    /// The graph as facts (t33 step 2), its paths as the image writes them (`rel`): `p0g_include(from, at, name,
    /// path)`, `p0g_missing(from, at, name)`, `p0g_absent(path)`, `p0g_digest(path, sha256)`, `p0g_phase(from,
    /// phase, path)`, `p0g_rule(path, sha256)`, `p0g_plan(plan)`, and the module plan (decision 0639):
    /// `p0g_module(m, rank, key, scope, tail)`, `p0g_lowered(key, image)`, `p0g_tailed(tail, image)`; each relation's
    /// rows in the graph's order. A graph read back without its modules held no file of a text in a module.
    fn facts(&self, g: &Graph) -> crate::code_rule_dsl::logic::program_facts::Facts {
        use crate::code_rule_dsl::logic::front::Value;
        let s = |x: String| Value::Str(x.into());
        let p = |p: &PathBuf| s(self.rel(&p.to_string_lossy()));
        let hex = |d: &[u8; 32]| s(d.iter().map(|b| format!("{b:02x}")).collect::<String>());
        let mut f = crate::code_rule_dsl::logic::program_facts::Facts::new();
        let mut put = |rel: &str, row: Vec<Value>| f.entry(rel.to_owned()).or_default().push(row.into());
        for (from, to) in &g.includes {
            for (at, n, path) in to {
                put("p0g_include", vec![s(self.rel(from)), Value::Int(*at), s(n.clone()), p(path)]);
            }
        }
        for (from, at, n) in &g.missing {
            put("p0g_missing", vec![s(self.rel(from)), Value::Int(*at), s(n.clone())]);
        }
        for path in &g.absent {
            put("p0g_absent", vec![p(path)]);
        }
        for (path, d) in &g.digests {
            put("p0g_digest", vec![p(path), hex(d)]);
        }
        for (from, to) in &g.phases {
            for (ph, path) in to {
                put("p0g_phase", vec![s(self.rel(from)), s(ph.clone()), p(path)]);
            }
        }
        for (path, d) in &g.rules {
            put("p0g_rule", vec![p(path), hex(d)]);
        }
        for d in &g.plans {
            put("p0g_plan", vec![hex(d)]);
        }
        for (m, r) in &g.plan {
            put("p0g_module", vec![s(m.clone()), Value::Int(r.rank), s(r.key.clone()), s(r.scope.iter().cloned().collect::<Vec<_>>().join(",")), s(r.tail.clone())]);
        }
        for (k, d) in &g.lowered {
            put("p0g_lowered", vec![s(k.clone()), s(d.clone())]);
        }
        for (t, d) in &g.tailed {
            put("p0g_tailed", vec![s(t.clone()), s(d.clone())]);
        }
        // A graph holds a row at least: every seed's graph names the modules it reached (`p0g_digest`).
        f
    }

    /// The graph `facts` made, or none where a row does not read; `read` holds what earlier graphs of the call read.
    fn graph_of(&self, f: &crate::code_rule_dsl::logic::program_facts::Facts, read: &mut Read) -> Option<Graph> {
        use crate::code_rule_dsl::logic::front::Value;
        let text = |v: &Value| match v {
            Value::Str(x) => Some(x.as_str().to_owned()),
            _ => None,
        };
        let int = |v: &Value| match v {
            Value::Int(i) => Some(*i),
            _ => None,
        };
        let rows = |rel: &str| f.get(rel).into_iter().flatten();
        let mut g = Graph::default();
        for r in rows("p0g_include") {
            let [a, b, c, d] = &r[..] else { return None };
            g.includes.entry(read.from(self, a)?).or_default().push((int(b)?, text(c)?, read.path(self, d)?));
        }
        for r in rows("p0g_missing") {
            let [a, b, c] = &r[..] else { return None };
            g.missing.push((read.from(self, a)?, int(b)?, text(c)?));
        }
        for r in rows("p0g_absent") {
            let [a] = &r[..] else { return None };
            g.absent.push(read.path(self, a)?);
        }
        for r in rows("p0g_digest") {
            let [a, b] = &r[..] else { return None };
            g.digests.insert(read.path(self, a)?, read.digest(b)?);
        }
        for r in rows("p0g_phase") {
            let [a, b, c] = &r[..] else { return None };
            g.phases.entry(read.from(self, a)?).or_default().push((text(b)?, read.path(self, c)?));
        }
        for r in rows("p0g_rule") {
            let [a, b] = &r[..] else { return None };
            g.rules.insert(read.path(self, a)?, read.digest(b)?);
        }
        for r in rows("p0g_plan") {
            let [a] = &r[..] else { return None };
            g.plans.push(read.digest(a)?);
        }
        for r in rows("p0g_module") {
            let [m, rank, key, scope, tail] = &r[..] else { return None };
            let scope = text(scope)?.split(',').filter(|s| !s.is_empty()).map(str::to_owned).collect();
            g.plan.insert(text(m)?, PlanModule { rank: int(rank)?, key: text(key)?, scope, tail: text(tail)? });
        }
        for r in rows("p0g_lowered") {
            let [k, d] = &r[..] else { return None };
            g.lowered.insert(text(k)?, text(d)?);
        }
        for r in rows("p0g_tailed") {
            let [t, d] = &r[..] else { return None };
            g.tailed.insert(text(t)?, text(d)?);
        }
        Some(g)
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
        let plan: Vec<String> = g.plan.iter().flat_map(|(m, r)| [m.clone(), r.rank.to_string(), r.key.clone(), r.scope.iter().cloned().collect::<Vec<_>>().join(","), r.tail.clone()]).collect();
        let lowered: Vec<String> = g.lowered.iter().flat_map(|(k, d)| [k.clone(), d.clone()]).collect();
        let tailed: Vec<String> = g.tailed.iter().flat_map(|(k, d)| [k.clone(), d.clone()]).collect();
        crate::content_store::observe::encode_lists(&[&includes[..], &missing[..], &absent[..], &digests[..], &phases[..], &rules[..], &plans[..], &plan[..], &lowered[..], &tailed[..]])
    }

    fn decode(&self, bytes: &[u8]) -> Option<Graph> {
        let [includes, missing, absent, digests, phases, rules, plans, plan, lowered, tailed]: [Vec<String>; 10] = crate::content_store::observe::decode_lists(bytes)?.try_into().ok()?;
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
        for r in plan.chunks(5) {
            let [m, rank, key, scope, tail] = r else { return None };
            let scope = scope.split(',').filter(|s| !s.is_empty()).map(str::to_owned).collect();
            g.plan.insert(m.clone(), PlanModule { rank: rank.parse().ok()?, key: key.clone(), scope, tail: tail.clone() });
        }
        for r in lowered.chunks(2) {
            let [k, d] = r else { return None };
            g.lowered.insert(k.clone(), d.clone());
        }
        for r in tailed.chunks(2) {
            let [k, d] = r else { return None };
            g.tailed.insert(k.clone(), d.clone());
        }
        Some(g)
    }
}

/// What a call's graphs read of their facts (`Images::graph_of`), each text once: the paths, the includers and the
/// digests, which the graphs of one library share.
#[derive(Default)]
struct Read {
    paths: HashMap<String, Option<PathBuf>>,
    froms: HashMap<String, String>,
    digests: HashMap<String, Option<[u8; 32]>>,
}

impl Read {
    fn text(v: &crate::code_rule_dsl::logic::front::Value) -> Option<&str> {
        match v {
            crate::code_rule_dsl::logic::front::Value::Str(x) => Some(x.as_str()),
            _ => None,
        }
    }

    fn path(&mut self, images: &Images, v: &crate::code_rule_dsl::logic::front::Value) -> Option<PathBuf> {
        let s = Self::text(v)?;
        if let Some(p) = self.paths.get(s) {
            return p.clone();
        }
        let p = images.abs(s).map(PathBuf::from);
        self.paths.insert(s.to_owned(), p.clone());
        p
    }

    /// An includer: a path, or one that is no file ("text", "engine") as written.
    fn from(&mut self, images: &Images, v: &crate::code_rule_dsl::logic::front::Value) -> Option<String> {
        let s = Self::text(v)?;
        if let Some(f) = self.froms.get(s) {
            return Some(f.clone());
        }
        let f = images.abs(s).unwrap_or_else(|| s.to_owned());
        self.froms.insert(s.to_owned(), f.clone());
        Some(f)
    }

    fn digest(&mut self, v: &crate::code_rule_dsl::logic::front::Value) -> Option<[u8; 32]> {
        let s = Self::text(v)?;
        if let Some(d) = self.digests.get(s) {
            return *d;
        }
        let unhex = |s: &str| -> Option<[u8; 32]> {
            let mut out = [0u8; 32];
            for (i, b) in out.iter_mut().enumerate() {
                *b = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
            }
            Some(out)
        };
        let d = unhex(s);
        self.digests.insert(s.to_owned(), d);
        d
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
        self.of_in(seed, k, dunce::canonicalize(crate::code_rule_dsl::logic::rr::library()).ok().as_deref())
    }

    /// `of`, the rule library `lib` (canonical, `Dirs::lib`) found once by a caller that makes many: found per graph,
    /// each was a walk of the library's path (317 a warm run).
    pub(in crate::code_rule_dsl) fn of_in(&self, seed: &Seed, k: usize, lib: Option<&std::path::Path>) -> Graph {
        let start = match seed {
            Seed::Text { .. } => text_key(k),
            Seed::File(p) => p.to_string_lossy().into_owned(),
        };
        let lib = lib.map(|l| l.to_string_lossy().into_owned());
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
            // A phase program's includes, past the seed's own (which `includes` holds): those its text names, not the
            // `core.rerail` another seed of the run named as a file seed (`own`), which a run over this seed alone
            // has not (a phase program's module, `<its dir>/core.rerail`, was an absent candidate of every seed whose
            // phases reached it, in the template's run of the whole library).
            if seen_rules.contains(&from) {
                for (_, n, path) in self.includes.get(&from).into_iter().flatten().filter(|(at, n, _)| own(&from, *at, n)) {
                    candidate(&from, n);
                    if let Some(d) = self.digests.get(path)
                        && !g.digests.contains_key(path) {
                            g.rules.insert(path.clone(), *d);
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
        // The plan's modules of the seed's own variant (a seed outside T's shape names `core.rerail` at -1) whose
        // files the seed reaches: those a run over the seed alone plans.
        // `core.rerail` as a seed of its own is planned as the programs that read it have it (`p0_seed_variant`).
        let is_core = |p: &PathBuf| lib.as_deref().is_some_and(|l| *p == candidate_path(l, "core.rerail"));
        let core_seed = matches!(seed, Seed::File(p) if is_core(p));
        let variant = if core_seed || g.includes.get(&if matches!(seed, Seed::Text { .. }) { TEXT.to_owned() } else { start.clone() }).is_some_and(|to| to.iter().any(|(at, n, _)| *at == -1 && n == "core.rerail")) { "core:" } else { "bare:" };
        let hex = |d: &[u8; 32]| d.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let reached: std::collections::BTreeSet<String> = g.digests.values().map(hex).collect();
        // A library file or a phase program as a seed is a module of the programs that include it
        // (`p0_seed_reaches(f, f)`): its graph holds its own module, which a text including it is composed with.
        g.plan = self.plan.iter().filter(|(m, _)| m.starts_with(variant) && PlanModule::members(m).all(|d| reached.contains(d))).map(|(m, r)| (m.clone(), r.clone())).collect();
        g.lowered = g.plan.values().filter_map(|r| self.lowered.get(&r.key).map(|d| (r.key.clone(), d.clone()))).collect();
        g.tailed = g.plan.values().filter_map(|r| self.tailed.get(&r.tail).map(|d| (r.tail.clone(), d.clone()))).collect();
        g
    }
}

/// Runs `bootstrap.rerail` over the seeds' facts, as a corpus run with no corpus: every document it reads it loads.
pub fn derive(seeds: &[Seed]) -> Result<Graph> {
    let lib = crate::code_rule_dsl::logic::rr::library();
    let lib = dunce::canonicalize(&lib).map_err(|e| PlanError::Plan(format!("{}: {e}", lib.display())))?;
    let bootstrap = lib.join("bootstrap.rerail");
    // The builtin read: the one rule file the engine knows by itself.
    let (bytes, _) = super::observe(|o| o.read(&bootstrap)).ok_or_else(|| PlanError::Plan(format!("{}: cannot read the file", bootstrap.display())))?;
    let mut text = String::from_utf8(bytes).map_err(|e| PlanError::Plan(format!("{}: {e}", bootstrap.display())))?;
    text.push_str("\n// The engine's builtin facts.\n");
    #[cfg(windows)]
    text.push_str("env(\"rerail.windows_paths\", \"1\").\n");
    text.push_str(&format!("env(\"rerail.lib\", {}).\n", quote(&bootstrap_spelling(&lib.to_string_lossy()))));
    // The engine's build, which a module's key names (`p0_module_key`).
    text.push_str(&format!("env(\"rerail.build\", {}).\n", quote(crate::code_rule_dsl::build_ids::engine())));
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
                    // A name that leaves the library (a pack's own library, named from it: `settings::module_text`):
                    // its file's directory is a root the loader may read, as a program file's own is.
                    if let Some(dir) = outside_library(&lib, name) {
                        dirs.insert(dir);
                    }
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
        .filter_map(|d| dunce::canonicalize(d).ok())
        .collect();
    for dir in dirs {
        text.push_str(&format!("env(\"rerail.program_dir\", {}).\n", quote(&bootstrap_spelling(&dir.to_string_lossy()))));
    }
    // The library is already a permitted input root. Phase zero only loads the files its seeds request.
    let options = serde_json::json!({ "reuse": true, "inputFiles": [] });
    // The modules the store holds, read before the run: its answers read no store (`rr::read_planned_images`).
    crate::code_rule_dsl::logic::rr::read_planned_images();
    let mut image = Vec::new();
    let phase_0 = in_phase_0();
    let run = super::run_handed(&[&text], None, None, &lib, None, std::path::Path::new(""), "program",
        crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18, &super::Typing::default(), &options, Some(&mut image), Default::default());
    drop(phase_0);
    run.and_then(|report| {
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
        g.absent.retain(|p| p.parent().and_then(|d| dunce::canonicalize(d).ok()).is_none_or(|d| roots.iter().any(|r| d.starts_with(r))));
        for r in outputs.rows("p0_phase") {
            if let (V::Text(from), V::Text(phase), V::Text(path)) = (r.get(0), r.get(1), r.get(2)) {
                g.phases.entry(host_spelling(from)).or_default().push((phase.to_owned(), PathBuf::from(host_spelling(path))));
            }
        }
        for v in g.phases.values_mut() {
            v.sort();
            v.dedup();
        }
        for r in outputs.rows("p0_module_rank") {
            if let (V::Text(m), Some(k)) = (r.get(0), r.get(1).int()) {
                g.plan.entry(m.to_owned()).or_default().rank = k;
            }
        }
        for r in outputs.rows("p0_module_key") {
            if let (V::Text(m), V::Text(key)) = (r.get(0), r.get(1)) {
                g.plan.entry(m.to_owned()).or_default().key = key.to_owned();
            }
        }
        for r in outputs.rows(crate::code_rule_dsl::logic::rr::LOWERED) {
            if let (V::Text(k), V::Text(d)) = (r.get(0), r.get(1)) {
                g.lowered.insert(k.to_owned(), d.to_owned());
            }
        }
        for r in outputs.rows("p0_tail_key") {
            if let (V::Text(m), V::Text(t)) = (r.get(0), r.get(1)) {
                g.plan.entry(m.to_owned()).or_default().tail = t.to_owned();
            }
        }
        for r in outputs.rows(crate::code_rule_dsl::logic::rr::TAILED) {
            if let (V::Text(t), V::Text(d)) = (r.get(0), r.get(1)) {
                g.tailed.insert(t.to_owned(), d.to_owned());
            }
        }
        for r in outputs.rows("p0_scope") {
            if let (V::Text(m), V::Text(s)) = (r.get(0), r.get(1)) {
                g.plan.entry(m.to_owned()).or_default().scope.insert(s.to_owned());
            }
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
    })
}

// Its tests run in the engine's test binary (its facade of this module mounts `tests.rs`): they read what the
// engine registers as it loads (the languages, their `.rerail` libraries). Its benchmark (`tests.rs`'s
// `benchmark_phase_0_rules`) too: a run of phase 0 needs the corpus runner registered.
