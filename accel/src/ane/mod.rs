//! The Neural Engine lane (decision 0096 §3): bit products against a **fixed relation**, compiled into a Core ML
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
//! **One submission** runs each model's launches together: their rows are concatenated, cut into tiles, and the
//! tiles go to Core ML as one batch (`predictionsFromBatch:`).

pub mod coreml;
pub mod package;

#[cfg(test)]
mod tests;

use crate::exec::Backend;
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
/// Tiles in one batch call: bounds the fp16 inputs held at once.
pub const MAX_BATCH: usize = 16;

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
/// decision 0096 §3); `None` keeps them in memory.
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

/// Where the last submission's wall time went, in seconds: expanding `a`'s bits to fp16, the prediction calls, and
/// summing the partial counts (with the outputs' conversion and the thresholds).
#[derive(Clone, Copy, Debug, Default)]
pub struct Split {
    pub expand: f64,
    pub predict: f64,
    pub sum: f64,
    pub calls: usize,
    pub tiles: usize,
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
        Ane { store, dir, cfg: coreml::config(), state: Mutex::new(State::default()), last: Mutex::default(), extra_aot }
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

    /// Runs every launch against one model: their rows concatenated, tiled, predicted in batches; partial counts
    /// summed per piece; thresholds applied.
    fn run_model(&self, m: &Model, ks: &[&Kernel]) -> Result<Vec<Output>, String> {
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
        let mut split = Split { tiles, ..Default::default() };
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
            let call = coreml::predict(&m.model, &mut items, &m.outputs, |item, _piece, data, rows, cols, stride| {
                if rows != t.m || cols != n {
                    return Err(format!("output of {rows} x {cols}, expected {} x {n}", t.m));
                }
                let tile = first + item;
                let lo = tile * t.m;
                let hi = (lo + t.m).min(total);
                counts[lo * n..hi * n].par_chunks_mut(n).enumerate().for_each(|(r, out)| {
                    for (o, &h) in out.iter_mut().zip(&data[r * stride..r * stride + n]) {
                        match half_count(h) {
                            Some(c) => *o += c,
                            None => bad.store(true, Ordering::Relaxed),
                        }
                    }
                });
                Ok(())
            })?;
            split.predict += call;
            split.sum += t0.elapsed().as_secs_f64() - call;
            split.calls += 1;
            if bad.load(Ordering::Relaxed) {
                return Err("an output was not an exact count".into());
            }
        }
        // Each launch's share, as the reference gives it.
        let t0 = Instant::now();
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
        *self.last.lock().unwrap() = split;
        Ok(outs)
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
pub fn half_count(h: u16) -> Option<u32> {
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
    (v <= package::EXACT).then_some(v)
}

impl Backend for Ane {
    fn lane(&self) -> Lane {
        Lane::Ane
    }

    fn can(&self, k: &Kernel) -> bool {
        self.refusal(k).is_none()
    }

    fn submit(&self, ks: &[&Kernel]) -> Result<Vec<Output>, String> {
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
        for (_, m, ids) in &groups {
            let sub: Vec<&Kernel> = ids.iter().map(|&i| ks[i]).collect();
            for (&i, o) in ids.iter().zip(self.run_model(m, &sub)?) {
                outs[i] = Some(o);
            }
        }
        Ok(outs.into_iter().map(|o| o.expect("every launch ran")).collect())
    }
}

impl Drop for Ane {
    fn drop(&mut self) {
        let st = self.state.get_mut().unwrap();
        st.loaded.clear();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
