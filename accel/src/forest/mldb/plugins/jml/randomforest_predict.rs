//! Prediction with a trained random forest (rerail's addition; upstream predicted through the committee the
//! procedure saved, one example at a time, on the host).
//!
//! The rows to predict come as one bucket column per feature (`RowBuckets`: each column's values in the training's
//! buckets, `ColumnInfo::bucketize`), and every row goes the way training sent rows with its buckets
//! (`DecisionTree::predict_buckets`). Two forms, which give the same predictions:
//! - the committee of decision trees on the host (`Forest::committee`, `predict_host`), the reference;
//! - the forest flattened (`FlatForest`: nodes, roots, weights and the leaves' distributions) and predicted on the
//!   device, a batch of rows at once (`predictForestKernel`, a UKL kernel in randomforest_kernels_common.h, one
//!   thread per row over every tree; `predict_device`).
//!
//! A prediction is the two labels' weighted leaf distributions (label 1's is the probability the row is true).

use crate::forest::bucket::{BucketList, CellValue};
use crate::forest::compute_kernel_grid::ComputeKernelArgument;
use crate::forest::compute_kernel_metal::{alloc_uninitialized_array_sync, transfer_to_device_immutable, transfer_to_host_sync, MetalComputeQueue};
use crate::forest::dataset_feature_space::DatasetFeatureSpace;
use crate::forest::jml::committee::Committee;
use crate::forest::jml::decision_tree::DecisionTree;
use crate::forest::jml::split::Op;
use crate::forest::jml::tree::{Ptr, Tree};
use crate::forest::memory_region::Pod;
use crate::forest::randomforest_kernels_grid::kernel;
use crate::forest::randomforest_procedure::Forest;

/// The rows to predict: per feature (in the feature space's column order), the rows' buckets.
pub struct RowBuckets {
    pub columns: Vec<BucketList>,
    pub num_rows: usize,
}

impl RowBuckets {
    /// The rows given as values, per column in the feature space's column order, each bucketed as in training.
    pub fn from_values(fs: &DatasetFeatureSpace, columns: &[Vec<CellValue>]) -> Result<Self, String> {
        let num_rows = columns.first().map_or(0, |c| c.len());
        if columns.len() != fs.column_info.len() {
            return Err(format!("{} columns for a feature space of {}", columns.len(), fs.column_info.len()));
        }
        let mut out = Vec::with_capacity(columns.len());
        for (i, values) in columns.iter().enumerate() {
            if values.len() != num_rows {
                return Err("the columns differ in length".into());
            }
            out.push(fs.column(i).ok_or_else(|| format!("no column {i}"))?.bucketize(values));
        }
        Ok(RowBuckets { columns: out, num_rows })
    }
}

/// The column index of a feature.
fn column_index(fs: &DatasetFeatureSpace, feature: &crate::forest::jml::feature::Feature) -> Result<usize, String> {
    let hash = fs.get_hash(*feature)?;
    Ok(fs.column_info.get(&hash).ok_or("a split's feature is not in the feature space")?.index as usize)
}

impl Forest {
    /// The forest as a committee of decision trees (two labels), each tree with its weight.
    pub fn committee(&self) -> Committee {
        let mut c = Committee::new(2);
        for (t, &w) in self.trees.iter().zip(&self.weights) {
            c.add(DecisionTree::new(t.clone(), 2), w);
        }
        c
    }

    /// The predictions on the host: the committee, each tree routing the row by its buckets.
    pub fn predict_host(&self, rows: &RowBuckets) -> Result<Vec<[f32; 2]>, String> {
        HostForest::new(self).predict(rows)
    }
}

/// The committee on the host, built once (its trees and the features' columns), predicting batch after batch.
pub struct HostForest {
    committee: Committee,
    index: std::collections::HashMap<crate::forest::jml::feature::Feature, usize>,
}

impl HostForest {
    pub fn new(forest: &Forest) -> Self {
        let mut index = std::collections::HashMap::new();
        for (hash, c) in &forest.fs.column_info {
            index.insert(forest.fs.get_feature(*hash), c.index as usize);
        }
        HostForest { committee: forest.committee(), index }
    }

    /// The predictions for a batch of rows (in parallel from 256 rows).
    pub fn predict(&self, rows: &RowBuckets) -> Result<Vec<[f32; 2]>, String> {
        use rayon::prelude::*;
        let one = |r: usize| {
            let p = self.committee.predict(&|tree| tree.predict_buckets(&|f| rows.columns[self.index[f]].get(r as u32)));
            [p[0], p[1]]
        };
        Ok(if rows.num_rows < 256 { (0..rows.num_rows).map(one).collect() } else { (0..rows.num_rows).into_par_iter().map(one).collect() })
    }
}

/// One node of a flattened forest (`PredictNode` in randomforest_kernels_common.h).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PredictNode {
    pub feature: u32,
    pub split_bucket: u32,
    pub ordinal: u32,
    /// A node's index when at least 0; leaf -(child + 1) when below 0.
    pub child_true: i32,
    pub child_false: i32,
}
unsafe impl Pod for PredictNode {}

/// A forest flattened for the device: every tree's nodes in one array, each tree's root, the trees' weights and
/// the leaves' distributions (two per leaf).
#[derive(Clone, Debug, Default)]
pub struct FlatForest {
    pub nodes: Vec<PredictNode>,
    pub roots: Vec<i32>,
    pub weights: Vec<f32>,
    pub leaf_preds: Vec<f32>,
    /// The deepest walk (nodes from a root to a leaf).
    pub max_depth: u32,
    pub num_features: usize,
}

impl FlatForest {
    pub fn new(forest: &Forest) -> Result<Self, String> {
        let mut f = FlatForest { num_features: forest.fs.column_info.len(), ..Default::default() };
        for (t, &w) in forest.trees.iter().zip(&forest.weights) {
            let root = f.flatten(&forest.fs, t, t.root, 0)?;
            f.roots.push(root);
            f.weights.push(w);
        }
        Ok(f)
    }

    fn flatten(&mut self, fs: &DatasetFeatureSpace, tree: &Tree, p: Ptr, depth: u32) -> Result<i32, String> {
        match p {
            Ptr::Null => Err(format!("a tree with no node or leaf where one is needed (depth {depth}, {} nodes so far)", self.nodes.len())),
            Ptr::Leaf(_) => {
                let pred = tree.pred(p);
                if pred.len() != 2 {
                    return Err(format!("a leaf with {} labels (2 expected)", pred.len()));
                }
                let leaf = (self.leaf_preds.len() / 2) as i32;
                self.leaf_preds.extend_from_slice(pred);
                self.max_depth = self.max_depth.max(depth);
                Ok(-(leaf + 1))
            }
            Ptr::Node(_) => {
                let node = tree.node(p).unwrap();
                if node.split_bucket < 0 {
                    return Err("a node with no split bucket (not from the random forest trainer)".into());
                }
                let at = self.nodes.len();
                self.nodes.push(PredictNode {
                    feature: column_index(fs, node.split.feature())? as u32,
                    split_bucket: node.split_bucket as u32,
                    ordinal: (node.split.op() == Op::Less) as u32,
                    child_true: 0,
                    child_false: 0,
                });
                let t = self.flatten(fs, tree, node.child_true, depth + 1)?;
                let f = self.flatten(fs, tree, node.child_false, depth + 1)?;
                self.nodes[at].child_true = t;
                self.nodes[at].child_false = f;
                Ok(at as i32)
            }
        }
    }

    /// The predictions on the device, every row at once (the forest uploaded for this call; `DeviceForest` keeps it
    /// there).
    pub fn predict_device(&self, rows: &RowBuckets) -> Result<Vec<[f32; 2]>, String> {
        DeviceForest::new(self)?.predict(rows)
    }
}

/// A flattened forest resident on the device: uploaded once, predicting batch after batch.
pub struct DeviceForest {
    nodes: metal::Buffer,
    roots: metal::Buffer,
    weights: metal::Buffer,
    leaves: metal::Buffer,
    num_trees: u32,
    max_depth: u32,
    num_features: usize,
}

impl DeviceForest {
    pub fn new(flat: &FlatForest) -> Result<Self, String> {
        let shared = crate::gpu::shared::get().ok_or("no Metal device")?;
        let device = &shared.device;
        let one = [PredictNode::default()];
        Ok(DeviceForest {
            nodes: transfer_to_device_immutable(device, if flat.nodes.is_empty() { &one[..] } else { &flat.nodes }),
            roots: transfer_to_device_immutable(device, &flat.roots),
            weights: transfer_to_device_immutable(device, &flat.weights),
            leaves: transfer_to_device_immutable(device, &flat.leaf_preds),
            num_trees: flat.roots.len() as u32,
            max_depth: flat.max_depth,
            num_features: flat.num_features,
        })
    }

    /// The predictions for a batch of rows: their buckets uploaded, one dispatch, one wait.
    pub fn predict(&self, rows: &RowBuckets) -> Result<Vec<[f32; 2]>, String> {
        Ok(self.predict_timed(rows)?.0)
    }

    /// `predict`, with the seconds the GPU spent on the dispatch.
    pub fn predict_timed(&self, rows: &RowBuckets) -> Result<(Vec<[f32; 2]>, f64), String> {
        use ComputeKernelArgument::{Buffer as B, Int};
        if rows.columns.len() != self.num_features {
            return Err(format!("{} columns for a forest over {}", rows.columns.len(), self.num_features));
        }
        if rows.num_rows == 0 {
            return Ok((Vec::new(), 0.0));
        }
        let shared = crate::gpu::shared::get().ok_or("no Metal device")?;
        let device = &shared.device;
        let (mut data, mut offsets, mut bits) = (Vec::new(), Vec::new(), Vec::new());
        for c in &rows.columns {
            offsets.push(data.len() as u32);
            bits.push(c.entry_bits as u32);
            data.extend_from_slice(c.storage.data());
        }
        offsets.push(data.len() as u32);
        // The kernel reads a word past an entry that ends a column's last word.
        data.push(0);
        let nr = rows.num_rows as u32;
        let d_data = transfer_to_device_immutable(device, &data);
        let d_offsets = transfer_to_device_immutable(device, &offsets);
        let d_bits = transfer_to_device_immutable(device, &bits);
        let d_out = alloc_uninitialized_array_sync::<f32>(device, rows.num_rows * 2);
        let mut queue = MetalComputeQueue::new(&shared.queue);
        let k = kernel("predictForest")?;
        let mut ctx = queue.new_bind_context(k.function);
        k.enqueue(&[
            ("numRows", Int(nr as i64)),
            ("numTrees", Int(self.num_trees as i64)),
            ("maxDepth", Int(self.max_depth as i64 + 1)),
            ("bucketData", B(&d_data)),
            ("bucketDataOffsets", B(&d_offsets)),
            ("bucketEntryBits", B(&d_bits)),
            ("nodes", B(&self.nodes)),
            ("treeRoots", B(&self.roots)),
            ("treeWeights", B(&self.weights)),
            ("leafPreds", B(&self.leaves)),
            ("predictionsOut", B(&d_out)),
        ], &[nr], &mut ctx)?;
        drop(ctx);
        let event = queue.flush();
        event.await_()?;
        let out = transfer_to_host_sync::<f32>(&d_out, rows.num_rows * 2);
        if out.iter().any(|v| v.is_nan()) {
            return Err("a walk did not reach a leaf".into());
        }
        Ok((out.chunks(2).map(|c| [c[0], c[1]]).collect(), event.gpu_seconds()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forest::dataset_feature_space::FeatureType;
    use crate::forest::randomforest_procedure::{train_forest, training_data, RandomForestProcedureConfig};
    use std::sync::Arc;

    /// Columns of values: real (with more distinct values than 255 buckets), real with nulls, categorical with
    /// nulls, small integers; the label depends on the first three.
    fn columns(n: usize, seed: u64) -> (Vec<Vec<CellValue>>, Vec<f32>) {
        let h = |i: usize, k: u64| ((i as u64 ^ seed).wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(k.wrapping_mul(0xD6E8_FEB8_6659_FD93))).rotate_right(23).wrapping_mul(0x2C1B_3C6D_297A_2D39) >> 33;
        let cats = ["red", "green", "blue", "grey"];
        let mut cols = vec![Vec::new(); 4];
        let mut labels = Vec::new();
        for i in 0..n {
            let a = (h(i, 1) % 100_000) as f64 / 1000.0;
            let b = if h(i, 2) % 10 == 0 { CellValue::Empty } else { CellValue::Float((h(i, 3) % 50) as f64 * 0.5) };
            let c = if h(i, 4) % 9 == 0 { CellValue::Empty } else { CellValue::String(cats[(h(i, 5) % 4) as usize].into()) };
            let d = CellValue::Integer((h(i, 6) % 6) as i64);
            let noise = h(i, 7) % 8 == 0;
            let label = ((a > 40.0) ^ matches!(&c, CellValue::String(s) if s == "blue") ^ matches!(b, CellValue::Empty)) ^ noise;
            cols[0].push(CellValue::Float(a));
            cols[1].push(b);
            cols[2].push(c);
            cols[3].push(d);
            labels.push(label as u8 as f32);
        }
        (cols, labels)
    }

    /// A forest trained from column values (feature space from the values), predicted on training rows and on
    /// unseen rows (with a category training never saw): the device's predictions are the host committee's, the
    /// value walk (upstream's) agrees with the bucket walk where their splits agree, and the forest has learned.
    #[test]
    fn device_predictions_are_the_committee_s() {
        if crate::gpu::shared::get().is_none() {
            eprintln!("no Metal device: skipped");
            return;
        }
        let n = 4000;
        let (cols, labels) = columns(n, 1);
        let named: Vec<(String, Vec<CellValue>)> = cols.iter().enumerate().map(|(i, c)| (format!("c{i}"), c.clone())).collect();
        let fs = Arc::new(DatasetFeatureSpace::from_columns(&named, FeatureType::Boolean).unwrap());
        assert_eq!(fs.column(0).unwrap().info, FeatureType::Real);
        assert_eq!(fs.column(2).unwrap().info, FeatureType::Categorical);
        assert!(fs.column(0).unwrap().distinct_values <= 256);
        let all = training_data(fs.clone(), &labels, None, None).unwrap();
        let config = RandomForestProcedureConfig { feature_vector_samplings: 3, feature_samplings: 4, max_depth: 10, feature_sampling_prop: 0.7, ..Default::default() };
        let forest = train_forest(&all, &config).unwrap();
        let flat = FlatForest::new(&forest).unwrap();
        assert!(flat.max_depth >= 3, "{}", flat.max_depth);

        let (mut test, test_labels) = columns(3000, 2);
        test[2][7] = CellValue::String("violet".into());
        test[1][8] = CellValue::String("text".into());
        for (what, c, l) in [("training", &cols, &labels), ("unseen", &test, &test_labels)] {
            let rows = RowBuckets::from_values(&fs, c).unwrap();
            let host = forest.predict_host(&rows).unwrap();
            let device = flat.predict_device(&rows).unwrap();
            for r in 0..host.len() {
                for k in 0..2 {
                    assert!((host[r][k] - device[r][k]).abs() < 1e-5, "{what} row {r}: host {:?}, device {:?}", host[r], device[r]);
                }
                assert!((host[r][0] + host[r][1] - 1.0).abs() < 1e-4, "{what} row {r}: {:?}", host[r]);
            }
            let right = host.iter().zip(l).filter(|(p, l)| (p[1] > 0.5) == (**l != 0.0)).count() as f64 / host.len() as f64;
            assert!(right > 0.75, "{what}: {right} right");
            eprintln!("{what}: {right:.3} right");
        }

        // Upstream's value walk, on the columns whose values and buckets agree (no nulls, no strings): numbers as
        // themselves. Only column 0 and 3 qualify; a forest over them alone.
        let named: Vec<(String, Vec<CellValue>)> = [0, 3].iter().map(|&i| (format!("c{i}"), cols[i].clone())).collect();
        let fs2 = Arc::new(DatasetFeatureSpace::from_columns(&named, FeatureType::Boolean).unwrap());
        let forest2 = train_forest(&training_data(fs2.clone(), &labels, None, None).unwrap(), &config).unwrap();
        let rows = RowBuckets::from_values(&fs2, &[cols[0].clone(), cols[3].clone()]).unwrap();
        let by_buckets = forest2.predict_host(&rows).unwrap();
        let committee = forest2.committee();
        let mut differ = 0;
        for r in 0..n {
            let p = committee.predict(&|t| t.predict_values(&|f| {
                let i = column_index(&fs2, f).unwrap();
                [&cols[0], &cols[3]][i][r].to_double() as f32
            }));
            if (p[1] - by_buckets[r][1]).abs() > 1e-5 {
                differ += 1;
            }
        }
        // Column 0's split points are values once it has more distinct values than buckets: a row on one goes
        // left by its bucket and right by `<`. Those rows are few.
        assert!(differ * 100 < n, "{differ} rows differ");
    }

    fn h(i: usize, k: u64) -> u64 {
        let mut x = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ k.wrapping_mul(0xD6E8_FEB8_6659_FD93);
        x ^= x >> 31;
        x = x.wrapping_mul(0x2C1B_3C6D_297A_2D39);
        x ^ (x >> 29)
    }

    /// A CSV line's fields (sqlite3 -csv: quoted when they hold spaces or commas).
    fn fields(line: &str) -> Vec<String> {
        let (mut out, mut cur, mut quoted) = (Vec::new(), String::new(), false);
        for c in line.chars() {
            match c {
                '"' => quoted = !quoted,
                ',' if !quoted => out.push(std::mem::take(&mut cur)),
                c => cur.push(c),
            }
        }
        out.push(cur);
        out
    }

    /// A job sample: lane, forms, launches, units, estimate, wall, cpu, cold, load, threads.
    #[derive(Clone)]
    struct Job { lane: String, forms: String, launches: i64, units: f64, estimate: f64, wall: f64, cold: i64, load: f64, threads: i64 }

    fn real_jobs() -> Vec<Job> {
        let Ok(path) = std::env::var("BENCH_SAMPLES") else { return Vec::new() };
        std::fs::read_to_string(path).unwrap().lines().filter_map(|l| {
            let f = fields(l);
            if f.len() != 10 { return None; }
            Some(Job { lane: f[0].clone(), forms: f[1].clone(), launches: f[2].parse().ok()?, units: f[3].parse().ok()?, estimate: f[4].parse().ok()?, wall: f[5].parse().ok()?, cold: f[7].parse().ok()?, load: f[8].parse().ok()?, threads: f[9].parse().ok()? })
        }).collect()
    }

    const LANES: [&str; 4] = ["cpu", "gpu", "sme", "ane"];

    /// The modelled wall of a job on a lane: the lane's submission plus the form's rate on the lane per unit,
    /// slowed by load, plus a compile when cold.
    fn modelled_wall(form: &str, lane: usize, units: f64, load: f64, cold: bool, noise: f64) -> f64 {
        let fh = form.bytes().fold(7u64, |a, b| a.wrapping_mul(31).wrapping_add(b as u64));
        let rate = 1e-9 * (1u64 << (h(fh as usize, lane as u64) % 10)) as f64;
        let overhead = [2e-6, 30e-6, 5e-6, 100e-6][lane];
        let slow = if lane == 0 { 1.0 + load / 20.0 } else { 1.0 };
        (overhead + rate * units * slow + if cold { 1e-3 } else { 0.0 }) * noise
    }

    /// `n` synthetic jobs drawn from the real ones (their units, loads, launches, threads): four lanes, about 300
    /// forms; labelled "this lane beats the CPU for this job" (the pairwise question).
    fn synthetic(real: &[Job], n: usize, seed: u64) -> (Vec<Job>, Vec<f32>) {
        let mut jobs = Vec::with_capacity(n);
        let mut labels = Vec::with_capacity(n);
        for i in 0..n {
            let r = &real[(h(i, seed) % real.len() as u64) as usize];
            let lane = (h(i, seed + 1) % 4) as usize;
            let forms = if h(i, seed + 2) % 3 == 0 { r.forms.clone() } else { format!("form{}", h(i, seed + 3) % 300) };
            let units = (r.units.max(1.0) * 2f64.powi((h(i, seed + 4) % 9) as i32 - 4)).round();
            let cold = h(i, seed + 5) % 50 == 0;
            let noise = 1.0 + (h(i, seed + 6) % 1000) as f64 / 2000.0;
            let wall = modelled_wall(&forms, lane, units, r.load, cold, noise);
            let cpu_wall = modelled_wall(&forms, 0, units, r.load, false, 1.25);
            labels.push((wall < cpu_wall) as u8 as f32);
            jobs.push(Job { lane: LANES[lane].into(), forms, launches: r.launches, units, estimate: 0.0, wall, cold: cold as i64, load: r.load, threads: r.threads });
        }
        (jobs, labels)
    }

    fn columns_of(jobs: &[Job]) -> Vec<Vec<CellValue>> {
        vec![
            jobs.iter().map(|j| CellValue::String(j.lane.clone())).collect(),
            jobs.iter().map(|j| CellValue::String(j.forms.clone())).collect(),
            jobs.iter().map(|j| CellValue::Integer(j.launches)).collect(),
            jobs.iter().map(|j| CellValue::Float(j.units)).collect(),
            jobs.iter().map(|j| CellValue::Integer(j.cold)).collect(),
            jobs.iter().map(|j| CellValue::Float(j.load)).collect(),
            jobs.iter().map(|j| CellValue::Integer(j.threads)).collect(),
        ]
    }

    fn timed<T>(f: impl FnOnce() -> T) -> (T, f64, f64) {
        let (w, c) = (std::time::Instant::now(), crate::exec::process_cpu());
        let r = f();
        (r, w.elapsed().as_secs_f64(), crate::exec::process_cpu() - c)
    }

    fn train_bench(what: &str, cols: &[Vec<CellValue>], labels: &[f32], config: &RandomForestProcedureConfig) -> (Arc<DatasetFeatureSpace>, Forest) {
        let names = ["lane", "forms", "launches", "units", "cold", "load", "threads"];
        let named: Vec<(String, Vec<CellValue>)> = names.iter().zip(cols).map(|(n, c)| (n.to_string(), c.clone())).collect();
        let (fs, w0, c0) = timed(|| Arc::new(DatasetFeatureSpace::from_columns(&named, FeatureType::Boolean).unwrap()));
        let (all, w1, c1) = timed(|| training_data(fs.clone(), labels, None, None).unwrap());
        let (forest, w2, c2) = timed(|| train_forest(&all, config).unwrap());
        for (i, t) in forest.trees.iter().enumerate() {
            if let Err(e) = FlatForest::new(&Forest { fs: forest.fs.clone(), trees: vec![t.clone()], weights: vec![1.0] }) {
                let r = t.node(t.root).map(|n| (n.child_true, n.child_false, n.child_missing, n.split_bucket, n.base.examples));
                panic!("{what}: tree {i}: {e}; root {:?} {r:?}", t.root);
            }
        }
        let flat = FlatForest::new(&forest).unwrap();
        eprintln!("train {what}: {} rows, {} bags x {} trees, depth {}: feature space {:.0} ms ({:.0} ms CPU), rows {:.0} ms ({:.0} ms CPU), forest {:.0} ms wall, {:.0} ms CPU; {} nodes, deepest {}",
            labels.len(), config.feature_vector_samplings, config.feature_samplings, config.max_depth, w0 * 1e3, c0 * 1e3, w1 * 1e3, c1 * 1e3, w2 * 1e3, c2 * 1e3, flat.nodes.len(), flat.max_depth);
        (fs, forest)
    }

    fn right(p: &[[f32; 2]], l: &[f32]) -> f64 {
        p.iter().zip(l).filter(|(p, l)| (p[1] > 0.5) == (**l != 0.0)).count() as f64 / l.len() as f64
    }

    /// Training and prediction on the engine's job samples (`BENCH_SAMPLES`, a CSV of costs.sqlite's samples) and
    /// on synthetic jobs drawn from them at the fit's window (20,000) and the store's size (500,000), sizes in
    /// `BENCH_SIZES`; prediction in batches from 1 to 65,536 rows: on the host (the committee), on the device with
    /// the forest uploaded per call, and with the forest resident.
    #[test]
    #[ignore = "benchmark: run by name with --ignored --nocapture, BENCH_SAMPLES set"]
    fn bench_training_and_prediction() {
        if crate::gpu::shared::get().is_none() {
            return;
        }
        let real = real_jobs();
        assert!(!real.is_empty(), "BENCH_SAMPLES");
        let fsp: f32 = std::env::var("BENCH_FSP").ok().and_then(|v| v.parse().ok()).unwrap_or(0.3);
        let full = RandomForestProcedureConfig { feature_sampling_prop: fsp, ..Default::default() };
        let shallow = RandomForestProcedureConfig { max_depth: 8, feature_sampling_prop: fsp, ..Default::default() };
        // Warm the kernels.
        let (wj, wl) = synthetic(&real, 2000, 99);
        train_bench("warm-up", &columns_of(&wj), &wl, &shallow);

        // The real samples: did the job take longer than its price said?
        let labels: Vec<f32> = real.iter().map(|j| (j.wall > j.estimate) as u8 as f32).collect();
        eprintln!("real: {} samples, {:.2} over their price", real.len(), labels.iter().sum::<f32>() / labels.len() as f32);
        for c in [&full, &shallow] {
            train_bench("real", &columns_of(&real), &labels, c);
        }

        let sizes: Vec<usize> = std::env::var("BENCH_SIZES").ok().map(|v| v.split(',').map(|x| x.parse().unwrap()).collect()).unwrap_or(vec![20000]);
        let (test_jobs, test_labels) = synthetic(&real, 65536, 7);
        let test_cols = columns_of(&test_jobs);
        let mut kept = None;
        for &n in &sizes {
            let (jobs, labels) = synthetic(&real, n, 1);
            for c in [&full, &shallow] {
                let (fs, forest) = train_bench("synthetic", &columns_of(&jobs), &labels, c);
                let rows = RowBuckets::from_values(&fs, &test_cols).unwrap();
                eprintln!("  held out: {:.3} right ({:.2} of the labels true)", right(&FlatForest::new(&forest).unwrap().predict_device(&rows).unwrap(), &test_labels), test_labels.iter().sum::<f32>() / test_labels.len() as f32);
                if n == sizes[0] && std::ptr::eq(c, &full) {
                    kept = Some((fs, forest));
                }
            }
        }

        let (fs, forest) = kept.unwrap();
        let flat = FlatForest::new(&forest).unwrap();
        let ((), wr, _) = timed(|| ());
        let _ = wr;
        let (resident, wu, _) = timed(|| DeviceForest::new(&flat).unwrap());
        eprintln!("predict: forest of {} trees, {} nodes, uploaded once in {:.0} us", flat.roots.len(), flat.nodes.len(), wu * 1e6);
        let host_forest = HostForest::new(&forest);
        for b in [1usize, 16, 256, 4096, 65536] {
            let cols: Vec<Vec<CellValue>> = test_cols.iter().map(|c| c[..b].to_vec()).collect();
            let reps = (2000 / b).clamp(3, 20);
            let per = |w: f64| w / reps as f64 * 1e6;
            let (rows, wb, cb) = timed(|| { let mut r = None; for _ in 0..reps { r = Some(RowBuckets::from_values(&fs, &cols).unwrap()); } r.unwrap() });
            eprintln!("  batch {b:>6} x{reps}: bucketize {:>9.1} us ({:>9.1} CPU)", per(wb), per(cb));
            let hreps = if b > 256 { 0 } else { reps };
            let (host, wh, ch) = timed(|| { let mut r = Vec::new(); for _ in 0..hreps { r = host_forest.predict(&rows).unwrap(); } r });
            let per = |w: f64| if hreps == 0 { 0.0 } else { w / reps as f64 * 1e6 };
            eprintln!("  batch {b:>6}: host {:>9.1} us ({:>9.1} CPU), {:>7.1} ns/row", per(wh), per(ch), per(wh) * 1e3 / b as f64);
            let per = |w: f64| w / reps as f64 * 1e6;
            let mut gpu = 0.0;
            let (res, wres, cres) = timed(|| { let mut r = Vec::new(); for _ in 0..reps { let (p, g) = resident.predict_timed(&rows).unwrap(); r = p; gpu += g; } r });
            eprintln!("  batch {b:>6}: resident {:>8.1} us wall ({:>7.1} CPU), GPU {:>7.1} us, {:>6.1} ns/row on the GPU", per(wres), per(cres), per(gpu), per(gpu) * 1e3 / b as f64);
            let (dev, wd, cd) = timed(|| { let mut r = Vec::new(); for _ in 0..reps.min(5) { r = flat.predict_device(&rows).unwrap(); } r });
            let per5 = |w: f64| w / reps.min(5) as f64 * 1e6;
            eprintln!("  batch {b:>6}: device+upload {:>8.1} us wall ({:>7.1} CPU)", per5(wd), per5(cd));
            let per = |w: f64| w / reps as f64 * 1e6;
            let _ = per;
            for r in 0..b {
                assert!((host.get(r).map_or(dev[r][1], |h| h[1]) - dev[r][1]).abs() < 1e-5 && (res[r][1] - dev[r][1]).abs() < 1e-5);
            }
        }
    }
}
