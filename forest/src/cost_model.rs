//! The learned cost model (decision 0100 §14): what a job costs, its wall and its host CPU seconds, from the jobs
//! the executor ran. Two regression forests over the same features, one for ln(wall) and one for ln(CPU), trained
//! from the samples the executor keeps (`CostSample`) and asked a batch of jobs at once (`CostModel::predict`).
//!
//! **Features.** A job's lane and launch names (both categorical), its launches, units, whether it was cold, the
//! cores' slowdown under the machine's load, and the pool's threads.
//!
//! **What it knows.** A prediction is given only for a (lane, names) the model trained on at least
//! `CostConfig::min_samples` warm, successful samples of; for anything else it gives `None`, and the caller keeps
//! its priors.
//!
//! **Where it predicts.** A batch of up to `CostConfig::host_batch` jobs is walked on the host (`FlatForest::
//! predict_row`), where a device call's round trip would cost more than the walk; a larger one on the device, its
//! values bucketized there (`DeviceForest::predict_values`), both forests in one dispatch.
//!
//! **Plain data.** A model is its bucket rules, its two flat forests and their target scales, and what it knows:
//! `to_bytes` and `from_bytes` keep it across runs, and the device's copy is made when first needed.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use crate::bucket::CellValue;
use crate::dataset_feature_space::{ColumnData, DatasetFeatureSpace, FeatureType};
use crate::memory_region::Pod;
use crate::randomforest_predict::{BucketRules, BucketizerColumn, DeviceBucketizer, DeviceForest, FlatForest, PredictNode};
use crate::randomforest_procedure::{train_regression_forest, Qos, RandomForestProcedureConfig, TargetScale};

/// A finished job, as the model trains on it.
#[derive(Clone, Debug, PartialEq)]
pub struct CostSample {
    pub lane: String,
    /// Its launches' names, comma-joined in order (one name for a job of one kind of launch).
    pub forms: String,
    pub launches: u32,
    pub units: f64,
    pub cold: bool,
    pub ok: bool,
    /// The cores' slowdown under the machine's load.
    pub load: f64,
    pub threads: u32,
    /// What it cost: wall seconds, and its thread's CPU seconds.
    pub wall: f64,
    pub cpu: f64,
}

/// A job to price.
#[derive(Clone, Copy, Debug)]
pub struct CostQuery<'a> {
    pub lane: &'a str,
    pub forms: &'a str,
    pub launches: u32,
    pub units: f64,
    pub cold: bool,
    pub load: f64,
    pub threads: u32,
}

/// A job's predicted cost, in seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CostPrediction {
    pub wall: f64,
    pub cpu: f64,
}

/// How a model is trained and asked.
#[derive(Clone, Debug)]
pub struct CostConfig {
    pub forest: RandomForestProcedureConfig,
    /// The warm, successful samples a (lane, names) needs before the model prices it.
    pub min_samples: u32,
    /// The largest batch predicted on the host.
    pub host_batch: usize,
}

impl Default for CostConfig {
    /// A row sample per tree (100 trees, each its own bag and every feature), at least 5 samples a leaf, depth 20:
    /// on the engine's samples this was the forest's best (decision 0100 §14's comparison), at about 1.4–2× the
    /// training of upstream's 5 bags of 20 trees. Two threads at the utility quality of service, below the
    /// engine's: background would hold them to the efficiency cores and the GPU's lowest priority, at about 4–10×
    /// the wall and twice the GPU seconds (measured 2026-11-11).
    fn default() -> Self {
        CostConfig {
            forest: RandomForestProcedureConfig {
                feature_vector_samplings: 100,
                feature_samplings: 1,
                feature_sampling_prop: 1.0,
                min_leaf_examples: 5,
                max_threads: 2,
                qos: Qos::Utility,
                ..Default::default()
            },
            min_samples: 8,
            host_batch: 4,
        }
    }
}

/// The features, in the feature space's column order.
pub const FEATURES: [&str; 7] = ["lane", "forms", "launches", "units", "cold", "load", "threads"];

/// The smallest seconds a target is taken as (a job measured as no time at all): its log is finite.
const MIN_SECONDS: f64 = 1e-7;

/// A trained cost model.
pub struct CostModel {
    pub rules: BucketRules,
    pub wall: FlatForest,
    pub cpu: FlatForest,
    pub wall_scale: TargetScale,
    pub cpu_scale: TargetScale,
    /// The warm, successful samples of each (lane, names) it trained on.
    pub known: HashMap<(String, String), u32>,
    pub min_samples: u32,
    pub host_batch: usize,
    /// The samples it trained on.
    pub trained_on: usize,
    /// The priced (lane, names) pairs, looked up by borrowed names: their codes and buckets ([`Index`]).
    index: Index,
    device: OnceLock<Option<Arc<(DeviceForest, DeviceBucketizer)>>>,
}

/// What a query's lane and names give, found once a query with no allocation: the codes of the device's
/// categorical columns (in `lanes` and `forms`, the same every batch) and the host's buckets.
#[derive(Clone, Copy)]
struct Entry {
    lane_code: u32,
    forms_code: u32,
    lane_bucket: u32,
    forms_bucket: u32,
}

#[derive(Default)]
struct Index {
    /// Per lane (a handful: scanned), its names' entries.
    by_lane: Vec<(String, HashMap<String, Entry, Words>)>,
    lanes: Vec<String>,
    forms: Vec<String>,
}

impl Index {
    /// The pairs of `known` with at least `min` samples.
    fn new(known: &HashMap<(String, String), u32>, min: u32, rules: &BucketRules) -> Index {
        let mut ix = Index::default();
        let mut pairs: Vec<_> = known.iter().filter(|(_, n)| **n >= min).map(|(k, _)| k).collect();
        pairs.sort();
        let code = |names: &mut Vec<String>, n: &str| names.iter().position(|x| x == n).unwrap_or_else(|| {
            names.push(n.to_owned());
            names.len() - 1
        }) as u32;
        for (lane, forms) in pairs {
            let e = Entry {
                lane_code: code(&mut ix.lanes, lane),
                forms_code: code(&mut ix.forms, forms),
                lane_bucket: rules.bucket_of_name(0, Some(lane)),
                forms_bucket: rules.bucket_of_name(1, Some(forms)),
            };
            let at = match ix.by_lane.iter().position(|(l, _)| l == lane) {
                Some(at) => at,
                None => {
                    ix.by_lane.push((lane.clone(), HashMap::default()));
                    ix.by_lane.len() - 1
                }
            };
            ix.by_lane[at].1.insert(forms.clone(), e);
        }
        ix
    }

    fn get(&self, lane: &str, forms: &str) -> Option<Entry> {
        self.by_lane.iter().find(|(l, _)| l == lane)?.1.get(forms).copied()
    }
}

/// A hasher of a string eight bytes at a time (a multiply and a rotate a word): the index is asked once a query,
/// on the placement path, and its keys are the program's own names.
#[derive(Clone, Copy, Default)]
struct Words;

impl std::hash::BuildHasher for Words {
    type Hasher = WordHasher;
    fn build_hasher(&self) -> WordHasher {
        WordHasher(0)
    }
}

struct WordHasher(u64);

impl std::hash::Hasher for WordHasher {
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for c in &mut chunks {
            self.word(u64::from_le_bytes(c.try_into().unwrap()));
        }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut w = [0u8; 8];
            w[..rest.len()].copy_from_slice(rest);
            self.word(u64::from_le_bytes(w));
        }
    }
    fn write_u8(&mut self, b: u8) {
        self.word(b as u64);
    }
    fn write_usize(&mut self, v: usize) {
        self.word(v as u64);
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

impl WordHasher {
    fn word(&mut self, w: u64) {
        self.0 = (self.0.rotate_left(5) ^ w).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

impl std::fmt::Debug for CostModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CostModel {{ trained on {}, {} known, {} + {} nodes }}", self.trained_on, self.known.len(), self.wall.nodes.len(), self.cpu.nodes.len())
    }
}

impl CostModel {
    /// Trains on the successful samples (cold ones too: coldness is a feature).
    pub fn train(samples: &[CostSample], config: &CostConfig) -> Result<CostModel, String> {
        let used: Vec<&CostSample> = samples.iter().filter(|s| s.ok && s.wall.is_finite() && s.cpu.is_finite() && s.units.is_finite()).collect();
        if used.is_empty() {
            return Err("no successful samples to train on".into());
        }
        let columns = cells(used.iter().map(|s| query_of(s)));
        let named: Vec<(String, Vec<CellValue>)> = FEATURES.iter().map(|n| n.to_string()).zip(columns).collect();
        let fs = Arc::new(DatasetFeatureSpace::from_columns(&named, FeatureType::Real)?);
        let ln = |v: f64| v.max(MIN_SECONDS).ln() as f32;
        let wall_targets: Vec<f32> = used.iter().map(|s| ln(s.wall)).collect();
        let cpu_targets: Vec<f32> = used.iter().map(|s| ln(s.cpu)).collect();
        let wall = train_regression_forest(fs.clone(), &wall_targets, None, None, &config.forest)?;
        let cpu = train_regression_forest(fs.clone(), &cpu_targets, None, None, &config.forest)?;
        let mut known: HashMap<(String, String), u32> = HashMap::new();
        for s in used.iter().filter(|s| !s.cold) {
            *known.entry((s.lane.clone(), s.forms.clone())).or_default() += 1;
        }
        let rules = BucketRules::new(&fs)?;
        Ok(CostModel {
            index: Index::new(&known, config.min_samples, &rules),
            rules,
            wall_scale: wall.target_scale.ok_or("no target scale")?,
            cpu_scale: cpu.target_scale.ok_or("no target scale")?,
            wall: FlatForest::new(&wall)?,
            cpu: FlatForest::new(&cpu)?,
            known,
            min_samples: config.min_samples,
            host_batch: config.host_batch,
            trained_on: used.len(),
            device: OnceLock::new(),
        })
    }

    /// Whether the model prices a job of these names on this lane.
    pub fn knows(&self, lane: &str, forms: &str) -> bool {
        self.index.get(lane, forms).is_some()
    }

    /// Each job's predicted cost, `None` for a job the model does not know (`knows`).
    pub fn predict(&self, queries: &[CostQuery]) -> Result<Vec<Option<CostPrediction>>, String> {
        let asked: Vec<(usize, Entry)> = queries.iter().enumerate().filter_map(|(i, q)| Some((i, self.index.get(q.lane, q.forms)?))).collect();
        let mut out = vec![None; queries.len()];
        if asked.is_empty() {
            return Ok(out);
        }
        let device = if asked.len() > self.host_batch { self.device() } else { None };
        let raw: Vec<([f32; 2], [f32; 2])> = match device {
            Some(d) => {
                let (forests, bucketizer) = &*d;
                let columns = self.columns(queries, &asked);
                let p = forests.predict_values(bucketizer, &columns)?;
                p.chunks(2).map(|c| (c[0], c[1])).collect()
            }
            None => asked.iter().map(|&(i, e)| {
                let b = self.buckets(&queries[i], e);
                Ok((self.wall.predict_row(&b)?, self.cpu.predict_row(&b)?))
            }).collect::<Result<_, String>>()?,
        };
        for (&(i, _), (w, c)) in asked.iter().zip(raw) {
            out[i] = Some(CostPrediction { wall: self.wall_scale.from_unit(w[1]).exp(), cpu: self.cpu_scale.from_unit(c[1]).exp() });
        }
        Ok(out)
    }

    /// The query's bucket in each feature (the host's bucketizing, as the device's).
    fn buckets(&self, q: &CostQuery, e: Entry) -> [u32; 7] {
        let r = &self.rules;
        [
            e.lane_bucket,
            e.forms_bucket,
            r.bucket_of_number(2, q.launches as f64),
            r.bucket_of_number(3, q.units),
            r.bucket_of_number(4, q.cold as u8 as f64),
            r.bucket_of_number(5, q.load),
            r.bucket_of_number(6, q.threads as f64),
        ]
    }

    /// Makes the device's copy and predicts once on it, so that its kernels are compiled here (a new binary's
    /// first use compiles them, seconds), not on the first placement that asks. Without a device, nothing.
    pub fn warm(&self) -> Result<(), String> {
        let Some((lane, names)) = self.index.by_lane.first() else { return Ok(()) };
        let Some(forms) = names.keys().next() else { return Ok(()) };
        if self.device().is_none() {
            return Ok(());
        }
        let q = CostQuery { lane, forms, launches: 1, units: 1.0, cold: false, load: 1.0, threads: 1 };
        self.predict(&vec![q; self.host_batch + 1]).map(|_| ())
    }

    /// The asked jobs as the device's typed columns: the names by their codes in the index.
    fn columns(&self, queries: &[CostQuery], asked: &[(usize, Entry)]) -> Vec<ColumnData> {
        let num = |f: &dyn Fn(&CostQuery) -> f64| ColumnData::Numeric(asked.iter().map(|&(i, _)| f(&queries[i])).collect());
        vec![
            ColumnData::Categorical { codes: asked.iter().map(|(_, e)| e.lane_code).collect(), names: self.index.lanes.clone() },
            ColumnData::Categorical { codes: asked.iter().map(|(_, e)| e.forms_code).collect(), names: self.index.forms.clone() },
            num(&|q| q.launches as f64),
            num(&|q| q.units),
            num(&|q| q.cold as u8 as f64),
            num(&|q| q.load),
            num(&|q| q.threads as f64),
        ]
    }

    /// The device's copy (both forests resident together, and the bucket rules), made once; `None` without a device.
    fn device(&self) -> Option<Arc<(DeviceForest, DeviceBucketizer)>> {
        self.device
            .get_or_init(|| {
                let forests = DeviceForest::new_many(&[&self.wall, &self.cpu]).ok()?;
                let bucketizer = DeviceBucketizer::from_rules(&self.rules).ok()?;
                Some(Arc::new((forests, bucketizer)))
            })
            .clone()
    }

    /// The model as bytes (`from_bytes` reads them back).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer(Vec::new());
        w.bytes(MAGIC);
        w.u32(FORMAT);
        w.u32(self.rules.columns.len() as u32);
        for c in &self.rules.columns {
            for v in [c.bits, c.split_start, c.num_splits, c.numeric_offset, c.null_bucket, c.unseen, c.strings.len() as u32] {
                w.u32(v);
            }
            let mut strings: Vec<(&String, &u32)> = c.strings.iter().collect();
            strings.sort();
            for (s, &b) in strings {
                w.str(s);
                w.u32(b);
            }
        }
        w.pods(&self.rules.split_keys);
        for (f, scale) in [(&self.wall, &self.wall_scale), (&self.cpu, &self.cpu_scale)] {
            w.pods(&f.nodes);
            w.pods(&f.roots);
            w.pods(&f.weights);
            w.pods(&f.leaf_preds);
            w.pods(&f.subsets);
            w.u32(f.max_depth);
            w.u32(f.num_features as u32);
            w.pods(&f.feature_bits);
            w.pods(&[scale.lo, scale.hi]);
        }
        let mut known: Vec<(&(String, String), &u32)> = self.known.iter().collect();
        known.sort();
        w.u32(known.len() as u32);
        for ((lane, forms), &n) in known {
            w.str(lane);
            w.str(forms);
            w.u32(n);
        }
        w.u32(self.min_samples);
        w.u32(self.host_batch as u32);
        w.u64(self.trained_on as u64);
        w.0
    }

    /// A model from `to_bytes`'s bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<CostModel, String> {
        let mut r = Reader(bytes);
        if r.take(MAGIC.len())? != MAGIC {
            return Err("not a cost model".into());
        }
        let format = r.u32()?;
        if format != FORMAT {
            return Err(format!("cost model format {format}, not {FORMAT}"));
        }
        let mut rules = BucketRules::default();
        for _ in 0..r.u32()? {
            let mut c = BucketizerColumn { bits: r.u32()?, split_start: r.u32()?, num_splits: r.u32()?, numeric_offset: r.u32()?, null_bucket: r.u32()?, unseen: r.u32()?, strings: HashMap::new() };
            for _ in 0..r.u32()? {
                let s = r.str()?;
                c.strings.insert(s, r.u32()?);
            }
            rules.columns.push(c);
        }
        rules.split_keys = r.pods()?;
        let mut forest = || -> Result<(FlatForest, TargetScale), String> {
            let f = FlatForest { nodes: r.pods::<PredictNode>()?, roots: r.pods()?, weights: r.pods()?, leaf_preds: r.pods()?, subsets: r.pods()?, max_depth: r.u32()?, num_features: r.u32()? as usize, feature_bits: r.pods()? };
            let s: Vec<f64> = r.pods()?;
            if s.len() != 2 {
                return Err("a target scale of the wrong length".into());
            }
            Ok((f, TargetScale { lo: s[0], hi: s[1] }))
        };
        let (wall, wall_scale) = forest()?;
        let (cpu, cpu_scale) = forest()?;
        let mut known = HashMap::new();
        for _ in 0..r.u32()? {
            let (lane, forms) = (r.str()?, r.str()?);
            known.insert((lane, forms), r.u32()?);
        }
        let (min_samples, host_batch, trained_on) = (r.u32()?, r.u32()? as usize, r.u64()? as usize);
        if !r.0.is_empty() {
            return Err(format!("{} bytes past the cost model", r.0.len()));
        }
        // A forest must walk within its arrays: checked here, so a damaged model is refused, not walked.
        for f in [&wall, &cpu] {
            check(f, &rules)?;
        }
        let index = Index::new(&known, min_samples, &rules);
        Ok(CostModel { rules, wall, cpu, wall_scale, cpu_scale, known, min_samples, host_batch, trained_on, index, device: OnceLock::new() })
    }
}

/// Every index a forest's walk follows is within its arrays, and its features within the rules.
fn check(f: &FlatForest, rules: &BucketRules) -> Result<(), String> {
    let leaves = f.leaf_preds.len() / 2;
    let child = |c: i32| if c >= 0 { (c as usize) < f.nodes.len() } else { ((-(c + 1)) as usize) < leaves };
    let ok = f.roots.iter().all(|&r| child(r))
        && f.roots.len() == f.weights.len()
        && f.num_features == rules.columns.len()
        && f.nodes.iter().all(|n| child(n.child_true) && child(n.child_false) && (n.feature as usize) < f.num_features && (n.ordinal != 2 || (n.split_bucket as usize + 1) * 16 <= f.subsets.len()))
        && rules.columns.iter().all(|c| (c.split_start + c.num_splits) as usize <= rules.split_keys.len());
    if ok { Ok(()) } else { Err("a damaged cost model".into()) }
}

const MAGIC: &[u8] = b"rerail-cost-model";
const FORMAT: u32 = 1;

struct Writer(Vec<u8>);

impl Writer {
    fn bytes(&mut self, b: &[u8]) {
        self.0.extend_from_slice(b);
    }
    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.bytes(s.as_bytes());
    }
    fn pods<T: Pod>(&mut self, v: &[T]) {
        self.u64(v.len() as u64);
        self.bytes(unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), std::mem::size_of_val(v)) });
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.0.len() < n {
            return Err("a cost model cut short".into());
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn str(&mut self) -> Result<String, String> {
        let n = self.u32()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|e| e.to_string())
    }
    fn pods<T: Pod + Default + Clone>(&mut self) -> Result<Vec<T>, String> {
        let n = self.u64()? as usize;
        let size = std::mem::size_of::<T>();
        let bytes = self.take(n.checked_mul(size).ok_or("a cost model's length overflows")?)?;
        let mut v = vec![T::default(); n];
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), v.as_mut_ptr().cast::<u8>(), bytes.len()) };
        Ok(v)
    }
}

/// A sample's job, as a query.
fn query_of(s: &CostSample) -> CostQuery<'_> {
    CostQuery { lane: &s.lane, forms: &s.forms, launches: s.launches, units: s.units, cold: s.cold, load: s.load, threads: s.threads }
}

/// The jobs as the feature space's columns of cells (training).
fn cells<'a>(qs: impl Iterator<Item = CostQuery<'a>>) -> Vec<Vec<CellValue>> {
    let mut c: Vec<Vec<CellValue>> = vec![Vec::new(); FEATURES.len()];
    for q in qs {
        c[0].push(CellValue::String(q.lane.to_owned()));
        c[1].push(CellValue::String(q.forms.to_owned()));
        c[2].push(CellValue::Integer(q.launches as i64));
        c[3].push(CellValue::Float(q.units));
        c[4].push(CellValue::Integer(q.cold as i64));
        c[5].push(CellValue::Float(q.load));
        c[6].push(CellValue::Integer(q.threads as i64));
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Jobs whose cost follows their lane, names and units, with noise.
    fn samples(n: usize) -> Vec<CostSample> {
        let lanes = ["cpu", "gpu", "sme"];
        let forms = ["closure", "histogram", "bit product", "count table"];
        (0..n).map(|i| {
            let (l, f) = (i % 3, (i / 3) % 4);
            let units = 10f64.powi((i % 7) as i32 + 2);
            let rate = [5e-10, 2e-11, 8e-11][l] * (1.0 + f as f64);
            let noise = 1.0 + ((i * 7919) % 13) as f64 * 0.01;
            let wall = (2e-5 + rate * units) * noise;
            CostSample { lane: lanes[l].into(), forms: forms[f].into(), launches: 1, units, cold: i % 29 == 0, ok: true, load: 1.0 + (i % 5) as f64 * 0.2, threads: 18, wall, cpu: wall * if l == 0 { 1.0 } else { 0.3 } }
        }).collect()
    }

    fn query(s: &CostSample) -> CostQuery<'_> {
        query_of(s)
    }

    /// Batch prediction on the host and on the device, CPU and wall a call, at each batch size: where the host's
    /// walk stops paying (`CostConfig::host_batch`).
    #[test]
    #[ignore = "benchmark: --ignored --nocapture"]
    fn host_or_device_by_batch() {
        if crate::gpu::shared::get().is_none() {
            return;
        }
        let model = CostModel::train(&samples(12000), &CostConfig::default()).unwrap();
        let test = samples(12000);
        for b in [1usize, 4, 16, 64, 256, 1024] {
            let qs: Vec<CostQuery> = test[..b].iter().map(query).collect();
            for (what, hb) in [("host", usize::MAX), ("device", 0)] {
                let m = CostModel { host_batch: hb, ..CostModel::from_bytes(&model.to_bytes()).unwrap() };
                m.predict(&qs).unwrap();
                let reps = 50;
                let (w, c) = (std::time::Instant::now(), crate::process_cpu());
                for _ in 0..reps {
                    m.predict(&qs).unwrap();
                }
                eprintln!("BATCH {b:5} {what:6}: {:8.1} us wall {:8.1} us CPU a call", w.elapsed().as_secs_f64() / reps as f64 * 1e6, (crate::process_cpu() - c) / reps as f64 * 1e6);
            }
        }
        eprintln!("{model:?}");
    }

    /// Where training's and a large batch's time goes: each phase's wall, process CPU and GPU seconds; training at
    /// each quality of service.
    #[test]
    #[ignore = "benchmark: --ignored --nocapture"]
    fn phases_of_training_and_prediction() {
        if crate::gpu::shared::get().is_none() {
            return;
        }
        fn t<R>(what: &str, f: impl FnOnce() -> R) -> R {
            let (g, c, w) = (crate::compute_kernel_metal::gpu_seconds_awaited(), crate::process_cpu(), std::time::Instant::now());
            let r = f();
            eprintln!("PHASE {what:44}: {:9.2} ms wall {:9.2} ms CPU {:9.2} ms GPU", w.elapsed().as_secs_f64() * 1e3, (crate::process_cpu() - c) * 1e3, (crate::compute_kernel_metal::gpu_seconds_awaited() - g) * 1e3);
            r
        }
        let train = samples(12000);
        let used: Vec<&CostSample> = train.iter().collect();
        let named: Vec<(String, Vec<CellValue>)> = FEATURES.iter().map(|n| n.to_string()).zip(cells(used.iter().map(|s| query_of(s)))).collect();
        let fs = t("feature space", || Arc::new(DatasetFeatureSpace::from_columns(&named, FeatureType::Real).unwrap()));
        let targets: Vec<f32> = used.iter().map(|s| s.wall.max(MIN_SECONDS).ln() as f32).collect();
        for qos in [Qos::Background, Qos::Utility, Qos::Inherit] {
            for threads in [2, 0] {
                let config = RandomForestProcedureConfig { qos, max_threads: threads, ..CostConfig::default().forest };
                t(&format!("one forest, {qos:?}, threads {threads}"), || train_regression_forest(fs.clone(), &targets, None, None, &config).unwrap());
            }
        }
        let model = t("CostModel::train (2 forests, default)", || CostModel::train(&train, &CostConfig::default()).unwrap());
        let m = CostModel { host_batch: 0, ..CostModel::from_bytes(&model.to_bytes()).unwrap() };
        let test = samples(1024);
        let qs: Vec<CostQuery> = test.iter().map(query).collect();
        t("warm (the first device prediction)", || m.warm().unwrap());
        t("predict 1024, the first after warm", || m.predict(&qs).unwrap());
        for _ in 0..3 {
            t("predict 1024, all", || m.predict(&qs).unwrap());
            let asked: Vec<(usize, Entry)> = t("  index", || qs.iter().enumerate().filter_map(|(i, q)| Some((i, m.index.get(q.lane, q.forms)?))).collect());
            let columns = t("  columns", || m.columns(&qs, &asked));
            let d = m.device().unwrap();
            let (_, gpu) = t("  predict_values_timed", || d.0.predict_values_timed(&d.1, &columns).unwrap());
            eprintln!("    (its own GPU seconds: {:.3} ms)", gpu * 1e3);
        }
    }

    /// The model learns the costs (within a few percent on samples like those it trained on), the host's and the
    /// device's predictions agree, an unknown job is not priced, and the bytes give the same model back.
    #[test]
    fn learns_costs_predicts_on_both_and_round_trips() {
        if crate::gpu::shared::get().is_none() {
            return;
        }
        let train = samples(3000);
        let test: Vec<CostSample> = samples(3300)[3000..].iter().filter(|s| !s.cold).cloned().collect();
        let config = CostConfig { forest: RandomForestProcedureConfig { feature_vector_samplings: 20, ..CostConfig::default().forest }, ..Default::default() };
        let model = CostModel::train(&train, &config).unwrap();
        let qs: Vec<CostQuery> = test.iter().map(query).collect();
        let on_host = CostModel { host_batch: usize::MAX, ..CostModel::from_bytes(&model.to_bytes()).unwrap() }.predict(&qs).unwrap();
        let on_device = CostModel { host_batch: 0, ..CostModel::from_bytes(&model.to_bytes()).unwrap() }.predict(&qs).unwrap();
        let mut worst: f64 = 0.0;
        for ((s, h), d) in test.iter().zip(&on_host).zip(&on_device) {
            let (h, d) = (h.unwrap(), d.unwrap());
            assert!((h.wall / d.wall - 1.0).abs() < 1e-4 && (h.cpu / d.cpu - 1.0).abs() < 1e-4, "{h:?} {d:?}");
            worst = worst.max((h.wall / s.wall).ln().abs());
        }
        assert!(worst < 0.25, "worst log error {worst}");
        let unknown = CostQuery { lane: "ane", ..qs[0] };
        assert_eq!(model.predict(&[unknown, qs[0]]).unwrap()[0], None);
        let back = CostModel::from_bytes(&model.to_bytes()).unwrap();
        assert_eq!((back.rules == model.rules, back.wall == model.wall, back.cpu == model.cpu, back.known == model.known), (true, true, true, true));
        assert_eq!(back.to_bytes(), model.to_bytes());
        let mut damaged = model.to_bytes();
        damaged.truncate(damaged.len() - 3);
        assert!(CostModel::from_bytes(&damaged).is_err());
    }
}
