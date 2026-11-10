//! The Neural Engine lane (decision 0098 §3): bit products against a **fixed relation**, compiled into a Core ML
//! model as a 1-bit constant.
//!
//! **What it runs.** Only [`Kernel::BitProduct`] whose `b` is fixed (`b.fixed` is set: the same rows across many
//! runs, such as a pack's rule vocabularies). `b` becomes the model's constant, kept compressed (1 bit an entry) and
//! expanded on chip; `a`'s rows are the per-run input, as fp16 0/1 values (16 bits an entry: the Neural Engine takes
//! nothing narrower). The model's template is [`package::Template`]: tiles of [`package::TILE_ROWS`] rows of `a`, the
//! shared dimension cut into pieces so that every partial count is exact in fp16. The host sums the pieces exactly
//! and applies thresholds, so the output equals [`crate::cpu::run`]'s.
//!
//! **Where models come from**, in order: the executable (ahead-of-time models, compiled by the build step from
//! `accel/models/aot.list`), the [`ModelStore`] (models compiled at run time are facts in the engine's content
//! store), then compiling now: the package is written by [`package`], compiled by Core ML, archived and put in the
//! store. A model's key is the digest of its template and its constant's digest ([`model_key`]).
//!
//! **Placement is checked**, never assumed: every compiled model's plan (`MLComputePlan`) must put every operation
//! on the Neural Engine. A model it would run elsewhere is refused with the reason ([`Ane::refusal`]), and the lane
//! does not take that kernel. Nothing moves to the CPU silently.
//!
//! **One submission** runs each model's launches together: their rows are concatenated and cut into tiles, one
//! prediction a tile.
//!
//! **Inputs and outputs live in IOSurfaces.** A tile's fp16 rows are written in place into an IOSurface-backed
//! array, and the engine writes its counts into IOSurfaces given as the outputs' backings, so Core ML copies
//! nothing on the host. The surfaces are kept in a pool by shape and reused by later submissions
//! ([`coreml::Surface`]). A shape past IOSurface's limits ([`coreml::surface_fits`]) cannot be backed by one, and
//! only such a shape takes the older path: plain arrays, one batch call (`predictionsFromBatch:`).
//!
//! **The host's work overlaps the predictions.** On IOSurfaces a submission is a pipeline over [`DEPTH`] sets of
//! surfaces: while tile `k`'s prediction runs on a predictor thread, the lane's thread decodes tile `k - 1`'s
//! counts and expands tile `k + 1` into the other set. Two bounded channels carry the tiles between the threads
//! (at most [`DEPTH`] in flight); the lane's thread blocks only on a prediction it needs, for at most
//! [`PREDICT_WAIT`]. `RERAIL_ANE_OVERLAP=off` runs the tiles one after another on the lane's thread.

pub mod coreml;
pub mod package;

#[cfg(test)]
mod tests;

use crate::exec::{process_cpu, thread_cpu, Backend, Phases};
use crate::kernel::{Bits, Kernel, Output};
use crate::lanes::Lane;
use package::Template;
use rayon::prelude::*;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// The ahead-of-time models: `(constant file, compiled model archive)`, written by `build/ane.rs`.
mod aot {
    include!(concat!(env!("OUT_DIR"), "/ane_aot.rs"));
}

/// The fewest floating-point operations (dense) a model's prediction must do: below about this, Core ML places a
/// model on the CPU whatever the Neural Engine supports (the spike's A1), so the lane does not compile one.
pub const MIN_FLOPS: f64 = 0.25e9;
/// Loaded models kept at once (Core ML holds at most about 119 compiled models per process).
pub const MAX_LOADED: usize = 64;
/// Tiles in one batch call (plain arrays only): bounds the fp16 inputs held at once.
pub const MAX_BATCH: usize = 16;
/// Bytes of IOSurfaces the pool keeps between submissions; past it, returned surfaces are freed.
pub const MAX_POOL_BYTES: usize = 256 << 20;
/// Sets of tile surfaces a submission cycles through when it overlaps the host's work with the predictions: one
/// being predicted while the host decodes and refills the other.
pub const DEPTH: usize = 2;
/// The longest the lane's thread waits for one prediction before it fails the submission.
pub const PREDICT_WAIT: std::time::Duration = std::time::Duration::from_secs(120);

/// Compiled models by key: what the engine keeps as facts in its content store. The value is an archive of the
/// compiled `.mlmodelc` directory ([`package::archive`]).
pub trait ModelStore: Send + Sync {
    fn get(&self, key: &[u8; 32]) -> Option<Arc<[u8]>>;
    fn put(&self, key: [u8; 32], archive: Arc<[u8]>);
}

/// A [`ModelStore`] in memory: the default, for a process that has no content store.
#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<[u8; 32], Arc<[u8]>>>);

impl ModelStore for MemoryStore {
    fn get(&self, key: &[u8; 32]) -> Option<Arc<[u8]>> {
        self.0.lock().unwrap().get(key).cloned()
    }
    fn put(&self, key: [u8; 32], archive: Arc<[u8]>) {
        self.0.lock().unwrap().insert(key, archive);
    }
}

/// A model's key: the digest of its template and of its constant's digest (`Bits::fixed`).
pub fn model_key(t: &Template, constant: &[u8; 32]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(t.encode());
    h.update(constant);
    h.finalize().into()
}

fn hex(k: &[u8; 32]) -> String {
    k.iter().map(|b| format!("{b:02x}")).collect()
}

/// The process's model store, when its host has one (the engine's content store: compiled models are facts there,
/// decision 0098 §3); `None` keeps them in memory.
static STORE: std::sync::RwLock<Option<Arc<dyn ModelStore>>> = std::sync::RwLock::new(None);

/// Makes `store` where this process's Neural Engine models are kept from now on: JIT-compiled models are written
/// there and read back by later runs. AOT models are part of the executable and never stored.
pub fn set_store(store: Arc<dyn ModelStore>) {
    *STORE.write().unwrap() = Some(store);
}

/// The host's store when set, else memory.
#[derive(Default)]
struct HostStore(MemoryStore);

impl ModelStore for HostStore {
    fn get(&self, key: &[u8; 32]) -> Option<Arc<[u8]>> {
        match STORE.read().unwrap().as_ref() {
            Some(s) => s.get(key),
            None => self.0.get(key),
        }
    }
    fn put(&self, key: [u8; 32], archive: Arc<[u8]>) {
        match STORE.read().unwrap().as_ref() {
            Some(s) => s.put(key, archive),
            None => self.0.put(key, archive),
        }
    }
}

/// This lane's backend, when it can run here: with the host's model store (`set_store`, else memory) and a working directory for compiled
/// models under `RERAIL_ANE_DIR` or the system's temporary directory.
pub fn backend() -> Option<Arc<dyn Backend>> {
    // Apple silicon, with Core ML's compute plans (macOS 14.4 and later), which placement checking needs.
    if !cfg!(target_arch = "aarch64") || objc::runtime::Class::get("MLComputePlan").is_none() {
        return None;
    }
    Some(Arc::new(Ane::new(Arc::new(HostStore::default()))))
}

/// Where a model came from, and what getting it cost: for the profile and the timing tests.
#[derive(Clone, Debug, Default)]
pub struct Prepared {
    /// `aot`, `store` or `jit`.
    pub source: &'static str,
    /// Seconds writing the package, compiling it, checking the plan, and loading it (the first load of a model
    /// includes the Neural Engine's own compile, which Core ML caches per process name).
    pub write: f64,
    pub compile: f64,
    pub plan: f64,
    pub load: f64,
    pub pieces: usize,
}

/// Where the last submission's time went, in seconds, summed over its models:
///
/// - `expand`: wall time expanding `a`'s bits to fp16 (the executor's [`Phases::encode`]);
/// - `predict`: wall time of the prediction calls ([`Phases::device`]); with `overlap` they run while the host
///   expands and sums;
/// - `sum`: wall time summing the partial counts, with the outputs' conversion and the thresholds
///   ([`Phases::decode`]);
/// - `wait`: wall time the lane's thread was blocked on a prediction with nothing else to do (not in [`Phases`]);
/// - `expand_cpu`, `sum_cpu`: the lane's thread's CPU (user + sys) over `expand` and `sum` (on IOSurfaces only:
///   the plain-array path expands on the rayon pool and leaves them zero);
/// - `predict_cpu`: Core ML's host work: on IOSurfaces, the process's CPU over the tiles less the lane's thread's
///   (so the predictor thread's and Core ML's own threads': [`Phases::worker_cpu`]); on plain arrays, the process's CPU over the calls;
/// - `surfaces`: whether every model's tiles ran on IOSurfaces; `overlap`: whether any ran pipelined.
#[derive(Clone, Copy, Debug, Default)]
pub struct Split {
    pub expand: f64,
    pub predict: f64,
    pub predict_cpu: f64,
    pub sum: f64,
    pub wait: f64,
    pub expand_cpu: f64,
    pub sum_cpu: f64,
    pub calls: usize,
    pub tiles: usize,
    pub surfaces: bool,
    pub overlap: bool,
}

impl Split {
    /// The executor's phases ([`Backend::submit_timed`]).
    pub fn phases(&self) -> Phases {
        Phases { encode: self.expand, device: Some(self.predict), decode: self.sum, worker_cpu: self.predict_cpu }
    }
}

struct Model {
    model: coreml::Obj,
    template: Template,
    outputs: Vec<String>,
}

#[derive(Default)]
struct State {
    /// Per constant digest: its template and model key, or why it has none.
    plans: HashMap<[u8; 32], Result<(Template, [u8; 32]), String>>,
    loaded: HashMap<[u8; 32], Arc<Model>>,
    order: VecDeque<[u8; 32]>,
    refused: HashMap<[u8; 32], String>,
    prepared: HashMap<[u8; 32], Prepared>,
    /// The ahead-of-time models by key, filled on first use.
    aot: Option<HashMap<[u8; 32], &'static [u8]>>,
}

/// The Neural Engine lane.
pub struct Ane {
    store: Arc<dyn ModelStore>,
    dir: PathBuf,
    cfg: coreml::Obj,
    state: Mutex<State>,
    last: Mutex<Split>,
    extra_aot: Vec<(Vec<u8>, Arc<[u8]>)>,
    /// IOSurfaces by `(rows, cols)`, reused across submissions.
    pool: Mutex<HashMap<(usize, usize), Vec<coreml::Surface>>>,
    /// Tests only: every shape on plain arrays, so the path for shapes no IOSurface holds is checked too.
    arrays_only: std::sync::atomic::AtomicBool,
    /// Whether the tiles on IOSurfaces are pipelined ([`DEPTH`] sets); off runs them one after another.
    overlap: std::sync::atomic::AtomicBool,
}

static INSTANCE: AtomicUsize = AtomicUsize::new(0);

impl Ane {
    pub fn new(store: Arc<dyn ModelStore>) -> Self {
        Ane::with_aot(store, Vec::new())
    }

    /// With more ahead-of-time models than the executable's: `(constant file, archive)` pairs (tests).
    pub fn with_aot(store: Arc<dyn ModelStore>, extra_aot: Vec<(Vec<u8>, Arc<[u8]>)>) -> Self {
        let base = std::env::var_os("RERAIL_ANE_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let dir = base.join(format!("rerail-ane-{}-{}", std::process::id(), INSTANCE.fetch_add(1, Ordering::Relaxed)));
        Ane { store, dir, cfg: coreml::config(), state: Mutex::new(State::default()), last: Mutex::default(), extra_aot, pool: Mutex::default(), arrays_only: Default::default(), overlap: (std::env::var("RERAIL_ANE_OVERLAP").as_deref() != Ok("off")).into() }
    }

    /// Whether later submissions overlap the host's work with the predictions (on unless `RERAIL_ANE_OVERLAP=off`).
    pub fn set_overlap(&self, on: bool) {
        self.overlap.store(on, Ordering::Relaxed);
    }

    /// Where the last submission's time went.
    pub fn last_split(&self) -> Split {
        *self.last.lock().unwrap()
    }

    /// What getting the model for `b` cost, once it is prepared.
    pub fn prepared(&self, b: &Bits) -> Option<Prepared> {
        let st = self.state.lock().unwrap();
        let (_, key) = st.plans.get(&b.fixed?)?.as_ref().ok()?;
        st.prepared.get(key).cloned()
    }

    /// Why this lane does not run `k`, or `None` when it does. A fixed constant seen for the first time is prepared
    /// here: its model compiled (or found) and its placement checked, so a launch placed here cannot fail on it.
    pub fn refusal(&self, k: &Kernel) -> Option<String> {
        let Kernel::BitProduct { a, b, min } = k else {
            return Some(format!("{}: the Neural Engine lane runs only bit products", k.form().name()));
        };
        if b.fixed.is_none() {
            return Some("bit product: b is not a fixed relation, so it cannot be a model's constant".into());
        }
        if a.words != b.words || min.as_ref().is_some_and(|m| m.len() != b.rows) {
            return Some("bit product: malformed (widths or thresholds)".into());
        }
        if b.rows == 0 || b.words == 0 {
            return Some("bit product: an empty constant".into());
        }
        self.model(b).err()
    }

    /// The template and key for `b`.
    fn plan(&self, st: &mut State, b: &Bits) -> Result<(Template, [u8; 32]), String> {
        let digest = b.fixed.expect("a fixed constant");
        st.plans
            .entry(digest)
            .or_insert_with(|| {
                let t = package::plan(b.rows, b.words, &b.data, package::TILE_ROWS)?;
                if t.flops() < MIN_FLOPS {
                    return Err(format!(
                        "bit product: {:.2e} operations a prediction, under the {MIN_FLOPS:.0e} below which Core ML runs a model on the CPU",
                        t.flops()
                    ));
                }
                let key = model_key(&t, &digest);
                Ok((t, key))
            })
            .clone()
    }

    fn aot_lookup(&self, st: &mut State, key: &[u8; 32]) -> Option<Arc<[u8]>> {
        if st.aot.is_none() {
            let mut m = HashMap::new();
            for &(c, a) in aot::AOT {
                if let Some(k) = aot_key(c) {
                    m.insert(k, a);
                }
            }
            st.aot = Some(m);
        }
        if let Some(a) = st.aot.as_ref().unwrap().get(key) {
            return Some(Arc::from(*a));
        }
        self.extra_aot.iter().find(|(c, _)| aot_key(c).as_ref() == Some(key)).map(|(_, a)| a.clone())
    }

    /// The loaded model for `b`: loaded already, or from the executable, the store, or compiled now.
    fn model(&self, b: &Bits) -> Result<Arc<Model>, String> {
        let mut st = self.state.lock().unwrap();
        let (t, key) = self.plan(&mut st, b)?;
        if let Some(m) = st.loaded.get(&key) {
            return Ok(m.clone());
        }
        if let Some(r) = st.refused.get(&key) {
            return Err(r.clone());
        }
        let mut prep = Prepared { pieces: t.pieces.len(), ..Default::default() };
        let r = self.build(&mut st, &t, &key, b, &mut prep);
        match r {
            Ok(m) => {
                let m = Arc::new(m);
                if st.order.len() >= MAX_LOADED {
                    if let Some(old) = st.order.pop_front() {
                        st.loaded.remove(&old);
                        let _ = std::fs::remove_dir_all(self.dir.join(format!("{}.mlmodelc", hex(&old))));
                    }
                }
                st.loaded.insert(key, m.clone());
                st.order.push_back(key);
                st.prepared.insert(key, prep);
                Ok(m)
            }
            Err(e) => {
                st.refused.insert(key, e.clone());
                Err(e)
            }
        }
    }

    fn build(&self, st: &mut State, t: &Template, key: &[u8; 32], b: &Bits, prep: &mut Prepared) -> Result<Model, String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("the lane's directory {}: {e}", self.dir.display()))?;
        let name = hex(key);
        let archive = if let Some(a) = self.aot_lookup(st, key) {
            prep.source = "aot";
            a
        } else if let Some(a) = self.store.get(key) {
            prep.source = "store";
            a
        } else {
            prep.source = "jit";
            let pkg = self.dir.join(format!("{name}.mlpackage"));
            let t0 = Instant::now();
            package::write_package(&pkg, t, b.words, &b.data).map_err(|e| format!("writing the model package: {e}"))?;
            prep.write = t0.elapsed().as_secs_f64();
            let t0 = Instant::now();
            let compiled = coreml::compile(&pkg);
            let _ = std::fs::remove_dir_all(&pkg);
            let compiled = compiled?;
            prep.compile = t0.elapsed().as_secs_f64();
            let a = package::archive(&compiled).map_err(|e| format!("archiving the compiled model: {e}"));
            let _ = std::fs::remove_dir_all(&compiled);
            let a: Arc<[u8]> = a?.into();
            self.store.put(*key, a.clone());
            a
        };
        let dir = self.dir.join(format!("{name}.mlmodelc"));
        package::unarchive(&archive, &dir)?;
        let t0 = Instant::now();
        let placed = coreml::check_placement(&dir, &self.cfg);
        prep.plan = t0.elapsed().as_secs_f64();
        if let Err(e) = placed {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e);
        }
        let t0 = Instant::now();
        let model = coreml::load(&dir, &self.cfg)?;
        prep.load = t0.elapsed().as_secs_f64();
        let outputs = (0..t.pieces.len()).map(|p| format!("y{p}")).collect();
        Ok(Model { model, template: t.clone(), outputs })
    }

    /// Runs every launch against one model: their rows concatenated, tiled, predicted; partial counts summed per
    /// piece; thresholds applied.
    fn run_model(&self, m: &Model, ks: &[&Kernel], split: &mut Split) -> Result<Vec<Output>, String> {
        let t = &m.template;
        // Every row of every launch, in order: (launch, row).
        let parts: Vec<(&Bits, &Bits, Option<&[u32]>)> = ks
            .iter()
            .map(|k| match k {
                Kernel::BitProduct { a, b, min } => (a, b, min.as_deref()),
                _ => unreachable!("grouped as bit products"),
            })
            .collect();
        let starts: Vec<usize> = parts.iter().scan(0, |s, p| { let x = *s; *s += p.0.rows; Some(x) }).collect();
        let total: usize = parts.iter().map(|p| p.0.rows).sum();
        let row_of = |g: usize| -> (&Bits, usize) {
            let l = starts.partition_point(|&s| s <= g) - 1;
            (parts[l].0, g - starts[l])
        };
        let n = t.n;
        let mut counts = vec![0u32; total * n];
        let tiles = total.div_ceil(t.m);
        let surfaces = !self.arrays_only.load(Ordering::Relaxed)
            && (0..t.pieces.len()).all(|p| coreml::surface_fits(t.m, t.k(p))) && coreml::surface_fits(t.m, n);
        split.tiles += tiles;
        split.surfaces &= surfaces;
        if surfaces {
            self.counts_on_surfaces(m, total, &row_of, &mut counts, tiles, split)?;
        } else {
            self.counts_on_arrays(m, total, &row_of, &mut counts, tiles, split)?;
        }
        // Each launch's share, as the reference gives it.
        if let [(a, b, None)] = parts[..] {
            // One launch's counts: the summed counts themselves, not a copy.
            return Ok(vec![Output::Counts { rows: a.rows, cols: b.rows, data: counts }]);
        }
        let (t0, c0) = (Instant::now(), thread_cpu());
        let outs: Vec<Output> = parts
            .iter()
            .zip(&starts)
            .map(|(&(a, b, min), &s)| {
                let c = &counts[s * n..(s + a.rows) * n];
                match min {
                    None => Output::Counts { rows: a.rows, cols: b.rows, data: c.to_vec() },
                    Some(min) => Output::Pairs(
                        c.chunks(n.max(1))
                            .enumerate()
                            .flat_map(|(i, row)| row.iter().enumerate().filter(|&(j, &x)| x >= min[j]).map(move |(j, _)| (i as u32, j as u32)))
                            .collect(),
                    ),
                }
            })
            .collect();
        split.sum += t0.elapsed().as_secs_f64();
        split.sum_cpu += thread_cpu() - c0;
        Ok(outs)
    }
}

impl Ane {
    /// A surface of `rows` × `cols` from the pool, or a new one.
    fn take(&self, rows: usize, cols: usize) -> Result<coreml::Surface, String> {
        if let Some(s) = self.pool.lock().unwrap().get_mut(&(rows, cols)).and_then(|v| v.pop()) {
            return Ok(s);
        }
        coreml::Surface::new(rows, cols)
    }

    /// Returns surfaces to the pool, freeing those past [`MAX_POOL_BYTES`].
    fn give(&self, surfaces: Vec<coreml::Surface>) {
        let mut pool = self.pool.lock().unwrap();
        let mut held: usize = pool.values().flatten().map(|s| s.bytes()).sum();
        for s in surfaces {
            if held + s.bytes() <= MAX_POOL_BYTES {
                held += s.bytes();
                pool.entry((s.rows, s.cols)).or_default().push(s);
            }
        }
    }

    /// The partial counts, tile by tile, on IOSurfaces: each tile's rows expanded in place into the pieces' input
    /// surfaces, one prediction, and the outputs read from the surfaces Core ML wrote them into.
    ///
    /// With `overlap` (and more than one tile) the tiles are a pipeline over [`DEPTH`] sets of surfaces: a predictor
    /// thread runs the predictions in order while this thread expands the next tile into a free set and decodes the
    /// last one predicted. Without it, the three steps run one after another here.
    ///
    /// The host's part runs on the lane's own thread, not the rayon pool: it is a few ms at the spikes' sizes, and
    /// the pool's workers spinning between a submission's short parallel steps cost several times that in CPU (W1:
    /// 4 ms of CPU a submission serial, 30 ms on the pool). The lane is there to free the CPU.
    fn counts_on_surfaces<'b>(
        &self,
        m: &Model,
        total: usize,
        row_of: &(dyn Fn(usize) -> (&'b Bits, usize) + Sync),
        counts: &mut [u32],
        tiles: usize,
        split: &mut Split,
    ) -> Result<(), String> {
        let t = &m.template;
        let n = t.n;
        let np = t.pieces.len();
        let depth = if self.overlap.load(Ordering::Relaxed) { DEPTH.min(tiles) } else { 1 };
        let mut held = Vec::with_capacity(depth * 2 * np);
        let r = (|| {
            for _ in 0..depth {
                for p in 0..np {
                    held.push(self.take(t.m, t.k(p))?);
                }
                for _ in 0..np {
                    held.push(self.take(t.m, n)?);
                }
            }
            let in_names: Vec<String> = (0..np).map(|p| format!("x{p}")).collect();
            let slots: Vec<Slot> = held
                .chunks(2 * np)
                .map(|set| {
                    let (ins, outs) = set.split_at(np);
                    Slot {
                        ins,
                        outs,
                        inputs: in_names.iter().map(String::as_str).zip(ins).collect(),
                        outputs: m.outputs.iter().map(String::as_str).zip(outs).collect(),
                    }
                })
                .collect();
            let tiler = Tiler { t, total, row_of };
            let (c0, host0) = (process_cpu(), split.expand_cpu + split.sum_cpu);
            let r = if depth == 1 {
                (|| {
                    for tile in 0..tiles {
                        tiler.fill(&slots[0], tile, split)?;
                        let (call, _) = coreml::predict_surfaces(&m.model, &slots[0].inputs, &slots[0].outputs)?;
                        split.predict += call;
                        split.calls += 1;
                        tiler.drain(&slots[0], tile, counts, split)?;
                    }
                    Ok(())
                })()
            } else {
                split.overlap = true;
                tiler.pipeline(&m.model, &slots, tiles, counts, split)
            };
            split.predict_cpu += (process_cpu() - c0 - (split.expand_cpu + split.sum_cpu - host0)).max(0.0);
            r
        })();
        self.give(held);
        r
    }

    /// The partial counts on plain arrays, for a shape no IOSurface can hold: per batch of tiles, the rows expanded
    /// into host buffers, which Core ML copies at each prediction, and one batch call.
    fn counts_on_arrays<'b>(
        &self,
        m: &Model,
        total: usize,
        row_of: &(dyn Fn(usize) -> (&'b Bits, usize) + Sync),
        counts: &mut [u32],
        tiles: usize,
        split: &mut Split,
    ) -> Result<(), String> {
        let t = &m.template;
        let n = t.n;
        for first in (0..tiles).step_by(MAX_BATCH) {
            let t0 = Instant::now();
            let last = (first + MAX_BATCH).min(tiles);
            // The inputs: per tile, per piece, `m` rows of fp16 0/1 (rows past the end are zero).
            let mut bufs: Vec<Vec<Vec<u16>>> = (first..last)
                .map(|tile| {
                    (0..t.pieces.len())
                        .map(|p| {
                            let (w0, w1) = t.pieces[p];
                            let k = t.k(p);
                            let mut buf = vec![0u16; t.m * k];
                            buf.par_chunks_mut(k).enumerate().for_each(|(r, out)| {
                                let g = tile * t.m + r;
                                if g < total {
                                    let (a, i) = row_of(g);
                                    expand(&a.row(i)[w0..w1], out);
                                }
                            });
                            buf
                        })
                        .collect()
                })
                .collect();
            let mut items: Vec<Vec<coreml::Input>> = bufs
                .iter_mut()
                .map(|pieces| {
                    pieces
                        .iter_mut()
                        .enumerate()
                        .map(|(p, data)| coreml::Input { name: format!("x{p}"), cols: t.k(p), rows: t.m, data })
                        .collect()
                })
                .collect();
            let bad = std::sync::atomic::AtomicBool::new(false);
            split.expand += t0.elapsed().as_secs_f64();
            let t0 = Instant::now();
            let (call, cpu) = coreml::predict(&m.model, &mut items, &m.outputs, |item, _piece, data, rows, cols, stride| {
                if rows != t.m || cols != n {
                    return Err(format!("output of {rows} x {cols}, expected {} x {n}", t.m));
                }
                let tile = first + item;
                let lo = tile * t.m;
                let hi = (lo + t.m).min(total);
                counts[lo * n..hi * n].par_chunks_mut(n).enumerate().for_each(|(r, out)| {
                    if !add_counts(out, &data[r * stride..r * stride + n]) {
                        bad.store(true, Ordering::Relaxed);
                    }
                });
                Ok(())
            })?;
            split.predict += call;
            split.predict_cpu += cpu;
            split.sum += t0.elapsed().as_secs_f64() - call;
            split.calls += 1;
            if bad.load(Ordering::Relaxed) {
                return Err("an output was not an exact count".to_string());
            }
        }
        Ok(())
    }
}

/// One set of a tile's surfaces: the pieces' inputs and outputs, and the same by feature name.
struct Slot<'s> {
    ins: &'s [coreml::Surface],
    outs: &'s [coreml::Surface],
    inputs: Vec<(&'s str, &'s coreml::Surface)>,
    outputs: Vec<(&'s str, &'s coreml::Surface)>,
}

/// A submission's rows as tiles of a model's template: what expands a tile into a slot and decodes its counts.
struct Tiler<'a, 'b> {
    t: &'a Template,
    total: usize,
    row_of: &'a (dyn Fn(usize) -> (&'b Bits, usize) + Sync),
}

impl Tiler<'_, '_> {
    /// Expands tile `tile`'s rows in place into `slot`'s input surfaces (rows past the end are zero).
    fn fill(&self, slot: &Slot, tile: usize, split: &mut Split) -> Result<(), String> {
        let (t0, c0) = (Instant::now(), thread_cpu());
        let t = self.t;
        for (p, s) in slot.ins.iter().enumerate() {
            let (w0, w1) = t.pieces[p];
            let (k, stride) = (t.k(p), s.stride);
            s.write(|buf| {
                buf.chunks_mut(stride).enumerate().for_each(|(r, row)| {
                    let g = tile * t.m + r;
                    if g < self.total {
                        let (a, i) = (self.row_of)(g);
                        expand(&a.row(i)[w0..w1], &mut row[..k]);
                    } else {
                        row[..k].fill(0);
                    }
                })
            })?;
        }
        split.expand += t0.elapsed().as_secs_f64();
        split.expand_cpu += thread_cpu() - c0;
        Ok(())
    }

    /// Adds tile `tile`'s partial counts, read from `slot`'s output surfaces, into `counts`.
    fn drain(&self, slot: &Slot, tile: usize, counts: &mut [u32], split: &mut Split) -> Result<(), String> {
        let (t0, c0) = (Instant::now(), thread_cpu());
        let n = self.t.n;
        let lo = tile * self.t.m;
        let hi = (lo + self.t.m).min(self.total);
        let outs = slot.outs;
        // Every piece's partial counts into a row while the row is in cache.
        let good = coreml::Surface::read_all(outs, |data| {
            let mut good = true;
            for (r, out) in counts[lo * n..hi * n].chunks_mut(n).enumerate() {
                for (s, d) in outs.iter().zip(data) {
                    good &= add_counts(out, &d[r * s.stride..r * s.stride + n]);
                }
            }
            good
        })?;
        split.sum += t0.elapsed().as_secs_f64();
        split.sum_cpu += thread_cpu() - c0;
        if good { Ok(()) } else { Err("an output was not an exact count".to_string()) }
    }

    /// The tiles as a pipeline over `slots`: a predictor thread takes `(slot, tile)` in order and hands back each
    /// result; this thread fills a free slot, and when none is free, waits for the oldest prediction, decodes it,
    /// and so frees its slot. Both channels hold at most `slots.len()` items, which is all that can be in flight, so
    /// neither thread ever blocks on a send.
    fn pipeline(&self, model: &coreml::Obj, slots: &[Slot], tiles: usize, counts: &mut [u32], split: &mut Split) -> Result<(), String> {
        use std::sync::mpsc::sync_channel;
        type Done = (usize, usize, Result<(f64, f64), String>);
        std::thread::scope(|s| {
            let (jobs, job_rx) = sync_channel::<(usize, usize)>(slots.len());
            let (done_tx, done) = sync_channel::<Done>(slots.len());
            s.spawn(move || {
                while let Ok((i, tile)) = job_rx.recv() {
                    let r = coreml::predict_surfaces(model, &slots[i].inputs, &slots[i].outputs);
                    if done_tx.send((i, tile, r)).is_err() {
                        break;
                    }
                }
            });
            // The oldest prediction: waited for, decoded; its slot is free again.
            let retire = |counts: &mut [u32], split: &mut Split| -> Result<usize, String> {
                let t0 = Instant::now();
                let (i, tile, r) = done
                    .recv_timeout(PREDICT_WAIT)
                    .map_err(|_| format!("a prediction did not finish within {} s", PREDICT_WAIT.as_secs()))?;
                split.wait += t0.elapsed().as_secs_f64();
                let (call, _) = r?;
                split.predict += call;
                split.calls += 1;
                self.drain(&slots[i], tile, counts, split)?;
                Ok(i)
            };
            let r = (|| {
                let mut free: Vec<usize> = (0..slots.len()).rev().collect();
                let mut in_flight = 0;
                for tile in 0..tiles {
                    let i = match free.pop() {
                        Some(i) => i,
                        None => {
                            in_flight -= 1;
                            retire(counts, split)?
                        }
                    };
                    self.fill(&slots[i], tile, split)?;
                    jobs.send((i, tile)).map_err(|_| "the predictor thread stopped".to_string())?;
                    in_flight += 1;
                }
                while in_flight > 0 {
                    in_flight -= 1;
                    retire(counts, split)?;
                }
                Ok(())
            })();
            // The predictor ends once the jobs it was sent are done (at most `slots.len()`); the scope joins it
            // before the slots go back to the pool.
            drop(jobs);
            r
        })
    }
}

/// The digest key of an ahead-of-time model from its constant file.
fn aot_key(constant: &[u8]) -> Option<[u8; 32]> {
    let (rows, words, data) = package::read_constant(constant).ok()?;
    let b = Bits::new(rows, words, data).fixed();
    let t = package::plan(rows, words, &b.data, package::TILE_ROWS).ok()?;
    Some(model_key(&t, &b.fixed.unwrap()))
}

/// fp16 1.0.
const ONE: u16 = 0x3c00;

/// Each byte's 8 bits as fp16 0/1, least significant first.
static EXPAND: [[u16; 8]; 256] = {
    let mut t = [[0u16; 8]; 256];
    let mut b = 0;
    while b < 256 {
        let mut i = 0;
        while i < 8 {
            if b >> i & 1 == 1 {
                t[b][i] = ONE;
            }
            i += 1;
        }
        b += 1;
    }
    t
};

/// Bits as fp16 0/1 values, one per bit.
fn expand(words: &[u64], out: &mut [u16]) {
    for (w, o) in words.iter().zip(out.chunks_exact_mut(64)) {
        for (byte, o8) in w.to_le_bytes().iter().zip(o.chunks_exact_mut(8)) {
            o8.copy_from_slice(&EXPAND[*byte as usize]);
        }
    }
}

/// An fp16 count as an integer; `None` when it is not a whole number in `0..=EXACT`.
pub const fn half_count(h: u16) -> Option<u32> {
    if h == 0 {
        return Some(0);
    }
    let (sign, e, f) = (h >> 15, (h >> 10 & 0x1f) as i32, (h & 0x3ff) as u32);
    if sign != 0 || e == 31 || e < 15 {
        return None;
    }
    let m = 1024 + f;
    let v = if e >= 25 {
        m << (e - 25)
    } else {
        let s = 25 - e;
        if m & ((1 << s) - 1) != 0 {
            return None;
        }
        m >> s
    };
    if v <= package::EXACT { Some(v) } else { None }
}

/// [`half_count`] of every fp16, `NOT_A_COUNT` where it is `None`: the outputs' conversion is one load an element.
static HALF_COUNTS: [u16; 1 << 16] = {
    let mut t = [NOT_A_COUNT; 1 << 16];
    let mut h = 0;
    while h < 1 << 16 {
        if let Some(c) = half_count(h as u16) {
            t[h] = c as u16;
        }
        h += 1;
    }
    t
};
const NOT_A_COUNT: u16 = u16::MAX;

/// Adds the fp16 counts `src` to `out`; `false` when one of them is not an exact count.
fn add_counts(out: &mut [u32], src: &[u16]) -> bool {
    let mut bad = false;
    for (o, &h) in out.iter_mut().zip(src) {
        let c = HALF_COUNTS[h as usize];
        bad |= c == NOT_A_COUNT;
        *o += c as u32;
    }
    !bad
}

impl Backend for Ane {
    fn lane(&self) -> Lane {
        Lane::Ane
    }

    fn can(&self, k: &Kernel) -> bool {
        self.refusal(k).is_none()
    }

    fn submit(&self, ks: &[&Kernel]) -> Result<Vec<Output>, String> {
        self.submit_timed(ks).map(|(o, _)| o)
    }

    fn submit_timed(&self, ks: &[&Kernel]) -> Result<(Vec<Output>, Phases), String> {
        // Group the launches by model, keeping each one's place.
        let mut groups: Vec<([u8; 32], Arc<Model>, Vec<usize>)> = Vec::new();
        let mut outs: Vec<Option<Output>> = vec![None; ks.len()];
        for (i, k) in ks.iter().enumerate() {
            if let Some(r) = self.refusal(k) {
                return Err(format!("launch {i}: {r}"));
            }
            let Kernel::BitProduct { a, b, min } = k else { unreachable!() };
            if a.rows == 0 {
                outs[i] = Some(match min {
                    None => Output::Counts { rows: 0, cols: b.rows, data: Vec::new() },
                    Some(_) => Output::Pairs(Vec::new()),
                });
                continue;
            }
            let m = self.model(b)?;
            let key = { self.state.lock().unwrap().plans[&b.fixed.unwrap()].as_ref().unwrap().1 };
            match groups.iter_mut().find(|g| g.0 == key) {
                Some(g) => g.2.push(i),
                None => groups.push((key, m, vec![i])),
            }
        }
        let mut split = Split { surfaces: true, ..Default::default() };
        for (_, m, ids) in &groups {
            let sub: Vec<&Kernel> = ids.iter().map(|&i| ks[i]).collect();
            for (&i, o) in ids.iter().zip(self.run_model(m, &sub, &mut split)?) {
                outs[i] = Some(o);
            }
        }
        *self.last.lock().unwrap() = split;
        Ok((outs.into_iter().map(|o| o.expect("every launch ran")).collect(), split.phases()))
    }
}

impl Drop for Ane {
    fn drop(&mut self) {
        let st = self.state.get_mut().unwrap();
        st.loaded.clear();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
