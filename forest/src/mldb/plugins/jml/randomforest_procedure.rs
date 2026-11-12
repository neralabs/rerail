//! randomforest_procedure.h and the bagging of randomforest_procedure.cc, in Rust.
//!
//! Mathieu Marquis Bolduc, 11 March 2016. Copyright (c) 2016 mldb.ai inc. This file is part of MLDB. Apache-2.0
//! (see the LICENSE at `forest/src/mldb/`).
//!
//! Trains a random forest binary classifier: the training rows (`training_data`: the rows kept, their labels and
//! weights), then per feature vector sampling a bag (`bag`: in 32 stretches of the rows, a proportion
//! `featureVectorSamplingProp` of each kept by a seeded shuffle, and a bootstrap of the stretch weighting them,
//! reweighted and compacted) and per feature sampling a tree over the active features each kept with probability
//! `featureSamplingProp`, all of a bag's trees trained together (`trainMultipleSamplings`, on the device). Bags are
//! trained in parallel, up to 8 at once as upstream (`maxBagsAtOnce`); the forest weighs every tree the same.
//!
//! Still in the .cc: the procedure's configuration description and registration, the binding of its SQL (the
//! label, weight and where expressions, the feature space built from the dataset), and the saving of the
//! classifier; rerail gives the feature space and the label, weight and where columns directly.
//!
//! Departures, each a fault upstream:
//! - a row's example number is its row in the dataset (upstream numbered the kept rows 0, 1, 2…, while the feature
//!   space's buckets are over every row of the dataset: past the first row left out, rows read other rows'
//!   buckets);
//! - the last stretch of the bootstrap runs to the last row (upstream's test `i == numPartitions` is never true,
//!   so the last `numRows % 32` rows were in no bag);
//! - the bag's weights are scaled to total 1 (upstream divided by the total count, so the weights totalled the mean
//!   input weight: past 2, W32 overflowed);
//! - the counts are 32 bit (upstream's 8 bit counts could wrap);
//! - the two proportions are used: upstream kept every row (`trainprop = 1`, the proportion unread) and took
//!   ceil(sqrt(n)) features per tree (the proportion's branch was `if (false)`); its sqrt rule is not kept;
//! - the bootstrap's generator is seeded apart from the shuffle's (upstream seeded both with the same number, so the
//!   draws were the shuffle's), and the feature samplings' seeds are distinct (upstream's `bag + 371 + i` gives bag
//!   b's tree i + 1 and bag b + 1's tree i the same features once the proportion's branch runs).
//! The RNG is MT19937 as upstream; the shuffles are Fisher-Yates over `rng() % n` and the uniform draws `rng() /
//! 2^32`, as `std::shuffle` and `uniform_real_distribution` are not the same across C++ libraries.

use crate::dataset_feature_space::DatasetFeatureSpace;
use crate::memory_region::MemorySerializer;
use crate::randomforest::{PartitionData, TrainingScheme};
use crate::randomforest_types::get_leaf;
use crate::randomforest::{FeatureSamplingTrainerKernel, FeatureSamplingTrainerPartition, TreeRows};
use crate::randomforest_predict::{FlatColumns, FlatForest, FlatTree};
use crate::tree::Tree;
use std::sync::Arc;

/// `RandomForestProcedureConfig`: what the procedure is configured with (less its SQL, model file and function).
#[derive(Clone, Debug)]
pub struct RandomForestProcedureConfig {
    /// Number of samplings of feature vectors (bags). The total number of trees is
    /// `feature_vector_samplings * feature_samplings`.
    pub feature_vector_samplings: usize,
    /// Proportion of feature vectors (rows) to select in each sample, before the bootstrap.
    pub feature_vector_sampling_prop: f32,
    /// Number of samplings of features (trees per bag).
    pub feature_samplings: usize,
    /// Proportion of features to select in each sample (each active feature kept with this probability).
    pub feature_sampling_prop: f32,
    /// Maximum depth of the trees.
    pub max_depth: i32,
    /// Should we sample feature vectors? Only set to false for testing or with very few samplings.
    pub sample_feature_vectors: bool,
    /// How the trees are trained (upstream: partitioned).
    pub training_scheme: TrainingScheme,
    /// A categorical feature's splits: CART's best subset (rerail's extension) when set, else upstream's best
    /// bucket against the rest.
    pub categorical_subsets: bool,
    /// The fewest rows a leaf may have (rerail's addition; upstream's is 1). Without it a regression tree, whose
    /// nodes are never pure, splits down to single rows.
    pub min_leaf_examples: u32,
    /// The threads training takes at most (0: `MAX_BAGS_AT_ONCE`), each with its device memory for a tree.
    pub max_threads: usize,
    /// The quality of service training's threads run at (macOS).
    pub qos: Qos,
    /// Who draws a tree's row sample in the one-trainer scheme (`train_forest_weighted`).
    pub bags: BagDraw,
}

/// Who draws a tree's row sample (its bag) in `train_forest_weighted`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BagDraw {
    /// The device (`bagRows`): from a hash of the bag and the example, each example kept with the sampling's
    /// proportion and counted Poisson(1) times; the host does nothing a row a tree. The bags are not upstream's (a
    /// Mersenne Twister's shuffle and draws), only as diverse. Small or lopsided data falls back to `Host`.
    #[default]
    Device,
    /// The host, upstream's draws (`bag_counts`): the trees are those of `train_forest_compacted`.
    Host,
}

/// A thread's quality of service on macOS: what the scheduler gives it, on the cores and on the GPU.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Qos {
    /// The caller's.
    #[default]
    Inherit,
    /// Utility: below the engine's threads, yet run on the performance cores at a fair GPU priority.
    Utility,
    /// Background: only on the efficiency cores, at the GPU's lowest priority. Measured (2026-11-11) at about
    /// 10× the wall and 3× the GPU seconds of utility on an idle machine, far more under load.
    Background,
}

impl Default for RandomForestProcedureConfig {
    fn default() -> Self {
        RandomForestProcedureConfig { feature_vector_samplings: 5, feature_vector_sampling_prop: 0.3, feature_samplings: 20, feature_sampling_prop: 0.3, max_depth: 20, sample_feature_vectors: true, training_scheme: TrainingScheme::Partitioned, categorical_subsets: false, min_leaf_examples: 1, max_threads: 0, qos: Qos::Inherit, bags: BagDraw::Device }
    }
}

/// MT19937, as `std::mt19937`.
#[derive(Clone)]
pub struct Mt19937 {
    state: [u32; 624],
    index: usize,
}

impl Mt19937 {
    pub fn new(seed: u32) -> Self {
        let mut state = [0u32; 624];
        state[0] = seed;
        for i in 1..624 {
            state[i] = 1812433253u32.wrapping_mul(state[i - 1] ^ (state[i - 1] >> 30)).wrapping_add(i as u32);
        }
        Mt19937 { state, index: 624 }
    }

    pub fn next_u32(&mut self) -> u32 {
        if self.index >= 624 {
            for i in 0..624 {
                let y = (self.state[i] & 0x8000_0000) | (self.state[(i + 1) % 624] & 0x7fff_ffff);
                let mut v = self.state[(i + 397) % 624] ^ (y >> 1);
                if y & 1 != 0 {
                    v ^= 0x9908_b0df;
                }
                self.state[i] = v;
            }
            self.index = 0;
        }
        let mut y = self.state[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^ (y >> 18)
    }

    /// `RandomForestRNG::operator()`: `rng() % val`.
    pub fn below(&mut self, val: usize) -> usize {
        self.next_u32() as usize % val
    }

    /// A uniform draw in [0, 1): `rng() / 2^32`.
    pub fn uniform01(&mut self) -> f64 {
        self.next_u32() as f64 / 4294967296.0
    }

    /// Fisher-Yates over `below`.
    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            v.swap(i, self.below(i + 1));
        }
    }
}

/// `RandomForestRNG`: MT19937 seeded with `seed` (1 for 0).
pub fn random_forest_rng(seed: u32) -> Mt19937 {
    Mt19937::new(seed + (seed == 0) as u32)
}

/// The training rows over the feature space (whose buckets are over every row of the dataset): the rows kept (a
/// weight above zero, a label, a non-zero where, none of them NaN), each numbered by its row in the dataset, with
/// its label (non-zero is true) and weight; the weights' total is scaled to 1 in the rows' wAll (the rows keep
/// their weights). `wheres` and `weights` default to 1.
pub fn training_data(fs: Arc<DatasetFeatureSpace>, labels: &[f32], wheres: Option<&[f32]>, weights: Option<&[f32]>) -> Result<PartitionData, String> {
    training_rows(fs, labels, wheres, weights, &mut |w, i, weight| w.add_row_values(labels[i] != 0.0, weight, i as u32))
}

/// A regression's targets, `[lo, hi]`, as the trainer takes them, `[0, 1]` (soft labels: `RowWeights`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TargetScale {
    pub lo: f64,
    pub hi: f64,
}

impl TargetScale {
    /// A target in [0, 1] (all 0.5 when every target is the same).
    pub fn to_unit(&self, y: f64) -> f32 {
        if self.hi > self.lo { ((y - self.lo) / (self.hi - self.lo)).clamp(0.0, 1.0) as f32 } else { 0.5 }
    }

    /// A prediction's target: the leaves' mean of the unit targets (a prediction's true side), scaled back.
    pub fn from_unit(&self, p: f32) -> f64 {
        if self.hi > self.lo { self.lo + p as f64 * (self.hi - self.lo) } else { self.lo }
    }
}

/// `training_data` for a regression: each kept row's target (not NaN) scaled into [0, 1] over the kept targets'
/// range. The feature space's label must be real (`FeatureType::Real`), which makes the splits' criterion the
/// variance (`SplitCriterion::of`).
pub fn regression_training_data(fs: Arc<DatasetFeatureSpace>, targets: &[f32], wheres: Option<&[f32]>, weights: Option<&[f32]>) -> Result<(PartitionData, TargetScale), String> {
    if fs.label_info != crate::dataset_feature_space::FeatureType::Real {
        return Err(format!("a regression's label must be real, not {:?}", fs.label_info));
    }
    if targets.iter().any(|t| t.is_infinite()) {
        return Err("a target is infinite".into());
    }
    let finite = || targets.iter().filter(|t| t.is_finite()).map(|&t| t as f64);
    let scale = TargetScale { lo: finite().fold(f64::INFINITY, f64::min), hi: finite().fold(f64::NEG_INFINITY, f64::max) };
    let data = training_rows(fs, targets, wheres, weights, &mut |w, i, weight| w.add_row_target(scale.to_unit(targets[i] as f64), weight, i as u32))?;
    Ok((data, scale))
}

/// The training rows, each kept row written by `write` (the writer, the row, its weight).
fn training_rows(fs: Arc<DatasetFeatureSpace>, labels: &[f32], wheres: Option<&[f32]>, weights: Option<&[f32]>, write: &mut dyn FnMut(&mut crate::randomforest_types::RowWriter, usize, f32) -> Result<(), String>) -> Result<PartitionData, String> {
    let n = labels.len();
    if wheres.is_some_and(|w| w.len() != n) || weights.is_some_and(|w| w.len() != n) {
        return Err("the label, where and weight columns differ in length".into());
    }
    if let Some(c) = fs.column_info.values().find(|c| c.buckets.num_entries != n) {
        return Err(format!("column {} has {} rows, the labels {n}", c.column_name, c.buckets.num_entries));
    }
    let weight = |i: usize| weights.map_or(1.0, |w| w[i]);
    let where_ = |i: usize| wheres.map_or(1.0, |w| w[i]);
    let keep_example = |i: usize| !weight(i).is_nan() && weight(i) > 0.0 && !labels[i].is_nan() && !where_(i).is_nan() && where_(i) != 0.0;
    let kept: Vec<usize> = (0..n).filter(|&i| keep_example(i)).collect();
    let total_kept_weight: f64 = kept.iter().map(|&i| weight(i) as f64).sum();
    let mut all_data = PartitionData::new(fs);
    let mut s = MemorySerializer;
    let mut writer = all_data.rows.get_row_writer(kept.len(), n, &mut s, false, (1.0 / total_kept_weight) as f32);
    for &i in &kept {
        write(&mut writer, i, weight(i))?;
    }
    all_data.rows = writer.freeze(&mut s);
    Ok(all_data)
}

/// Bag `bag`'s count per row: in up to 32 stretches of the rows, a seeded shuffle keeps the first
/// `featureVectorSamplingProp` of each stretch, and a bootstrap (as many draws as the stretch has rows, from a
/// second generator) weighs them; a row's count is its draws if it is kept. Without sampling every count is 1.
pub fn bag_counts(num_rows: usize, bag: usize, config: &RandomForestProcedureConfig) -> Vec<u32> {
    let num_partitions = num_rows.min(32);
    let mut counts = vec![0u32; num_rows];
    if num_partitions == 0 {
        return counts;
    }
    use rayon::prelude::*;
    let per = num_rows / num_partitions;
    let stretches: Vec<(usize, usize)> = (0..num_partitions).map(|i| (per * i, if i == num_partitions - 1 { num_rows } else { per * (i + 1) })).collect();
    let parts: Vec<Vec<u32>> = stretches.par_iter().enumerate().map(|(i, &(first, last))| {
        let n = last - first;
        if !config.sample_feature_vectors {
            return vec![1; n];
        }
        let seed = 245 + (bag * num_partitions) as u32 + i as u32;
        let mut in_training = vec![false; n];
        let mut order: Vec<usize> = (0..n).collect();
        Mt19937::new(seed).shuffle(&mut order);
        // As upstream's `i < numRowsInPartition * trainprop`: ceil(n * prop) rows.
        let kept = ((n as f64 * config.feature_vector_sampling_prop as f64).ceil() as usize).min(n);
        for &k in &order[..kept] {
            in_training[k] = true;
        }
        let mut myrng = random_forest_rng(seed ^ 0x9E37_79B9);
        let mut w = vec![0u32; n];
        for _ in 0..n {
            w[myrng.below(n)] += 1;
        }
        w.iter().zip(&in_training).map(|(&c, &t)| if t { c } else { 0 }).collect()
    }).collect();
    for ((first, _), part) in stretches.iter().zip(parts) {
        counts[*first..*first + part.len()].copy_from_slice(&part);
    }
    counts
}

/// Bag `bag`: its rows (`reweightAndCompact`, the weights totalling 1) and the active features of each of its
/// trees (each active feature with probability `featureSamplingProp`; at least one, in up to 10 attempts).
pub fn bag(all_data: &PartitionData, bag: usize, config: &RandomForestProcedureConfig) -> Result<(PartitionData, Vec<Vec<i32>>), String> {
    let counts = bag_counts(all_data.rows.row_count(), bag, config);
    let total: f64 = counts.iter().enumerate().map(|(i, &c)| all_data.rows.get_weight(i) as f64 * c as f64).sum();
    if total == 0.0 {
        return Err("No training weight in RandomForest bag; try training with more examples".into());
    }
    let mut s = MemorySerializer;
    let data = all_data.reweight_and_compact(&counts, 1.0 / total, &mut s)?;
    let samplings = bag_feature_samplings(&data, bag, config)?;
    Ok((data, samplings))
}

/// The active features of each of bag `bag`'s trees (each of the data's active features with probability
/// `featureSamplingProp`; at least one, in up to 10 attempts).
pub fn bag_feature_samplings(data: &PartitionData, bag: usize, config: &RandomForestProcedureConfig) -> Result<Vec<Vec<i32>>, String> {
    let active: Vec<i32> = data.features.iter().enumerate().filter(|(_, f)| f.active).map(|(i, _)| i as i32).collect();
    if active.is_empty() {
        return Err("no active features".into());
    }
    let mut sampling_active_features = Vec::with_capacity(config.feature_samplings);
    for i in 0..config.feature_samplings {
        let mut rng = Mt19937::new((371 + bag * config.feature_samplings + i) as u32);
        let mut active_features = Vec::new();
        // Cull the features according to the sampling proportion.
        for attempt in 0.. {
            active_features = active.iter().copied().filter(|_| rng.uniform01() <= config.feature_sampling_prop as f64).collect();
            if !active_features.is_empty() {
                break;
            }
            if attempt == 9 {
                return Err("Feature partition had no features; consider increasing featureSamplingProp".into());
            }
        }
        sampling_active_features.push(active_features);
    }
    Ok(sampling_active_features)
}

/// A forest: its trees (bag after bag, each bag's in sampling order), each weighed the same.
pub struct Forest {
    pub fs: Arc<DatasetFeatureSpace>,
    pub trees: Vec<Tree>,
    pub weights: Vec<f32>,
    /// A regression's target scale (`regression_training_data`): a prediction's true side is its unit target.
    pub target_scale: Option<TargetScale>,
}

impl Forest {
    /// A regression's prediction from the forest's distribution (`Forest::predict_*`).
    pub fn regression_value(&self, p: [f32; 2]) -> Option<f64> {
        self.target_scale.map(|s| s.from_unit(p[1]))
    }
}

/// A regression forest: the training rows from the targets (`regression_training_data`), then `train_forest`.
pub fn train_regression_forest(fs: Arc<DatasetFeatureSpace>, targets: &[f32], wheres: Option<&[f32]>, weights: Option<&[f32]>, config: &RandomForestProcedureConfig) -> Result<Forest, String> {
    let (all, scale) = regression_training_data(fs, targets, wheres, weights)?;
    let mut forest = train_forest(&all, config)?;
    forest.target_scale = Some(scale);
    Ok(forest)
}

/// Trains the forest over the training rows (`training_data`): the bags in parallel, up to 8 at once.
pub fn train_forest(all_data: &PartitionData, config: &RandomForestProcedureConfig) -> Result<Forest, String> {
    if config.training_scheme == TrainingScheme::Partitioned {
        return train_forest_weighted(all_data, config);
    }
    train_forest_compacted(all_data, config)
}

/// Puts the calling thread at `qos` (macOS; `Inherit` leaves it as it is).
pub fn set_qos(qos: Qos) {
    #[cfg(target_os = "macos")]
    unsafe {
        match qos {
            Qos::Inherit => {}
            Qos::Utility => {
                libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0);
            }
            Qos::Background => {
                libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_BACKGROUND, 0);
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = qos;
}

/// The most bags trained at once (threads, each with its device memory for a tree).
const MAX_BAGS_AT_ONCE: usize = 8;

/// The forest from one device trainer over all the rows (`train_forest` on the device): each tree weighs the rows
/// by its bag's draw (`TreeRows`: zero for the rows the bag leaves out), so a bag costs its draw and its trees, not a
/// trainer of its own. With `BagDraw::Host` the trees are those of `train_forest_compacted`. Bags go to up to
/// `MAX_BAGS_AT_ONCE` threads, each scheduling its trees a few ahead of the one it finishes.
pub fn train_forest_weighted(all_data: &PartitionData, config: &RandomForestProcedureConfig) -> Result<Forest, String> {
    let trees = train_weighted(all_data, config, false)?;
    let trees: Vec<Tree> = trees.into_iter().map(|t| match t {
        Trained::Tree(t) => t,
        Trained::Flat(_) => unreachable!("a flat tree from a tree's training"),
    }).collect();
    let total = trees.len();
    Ok(Forest { fs: all_data.fs.clone(), weights: vec![1.0 / total as f32; total], trees, target_scale: None })
}

/// `train_forest_weighted`, each tree built flat from the device's splits (`FlatTree`): the forest prediction
/// takes, with no `Tree` made on the way.
pub fn train_flat_forest(all_data: &PartitionData, config: &RandomForestProcedureConfig) -> Result<FlatForest, String> {
    if config.training_scheme != TrainingScheme::Partitioned {
        return FlatForest::new(&train_forest(all_data, config)?);
    }
    let trees: Vec<FlatTree> = train_weighted(all_data, config, true)?.into_iter().map(|t| match t {
        Trained::Flat(t) => t,
        Trained::Tree(_) => unreachable!("a tree from a flat training"),
    }).collect();
    let total = trees.len();
    Ok(FlatForest::from_trees(&all_data.fs, trees, vec![1.0 / total as f32; total]))
}

/// A regression forest as `train_regression_forest` trains it, flat (`train_flat_forest`), and its target scale.
pub fn train_regression_flat(fs: Arc<DatasetFeatureSpace>, targets: &[f32], wheres: Option<&[f32]>, weights: Option<&[f32]>, config: &RandomForestProcedureConfig) -> Result<(FlatForest, TargetScale), String> {
    let (all, scale) = regression_training_data(fs, targets, wheres, weights)?;
    Ok((train_flat_forest(&all, config)?, scale))
}

/// A tree as `train_weighted` gives it.
enum Trained {
    Tree(Tree),
    Flat(FlatTree),
}

/// The fewest rows a bag the device draws expects (its proportion of the rows): fewer, and the host draws the bags.
const DEVICE_BAG_MIN_ROWS: f64 = 256.0;

/// The device's bags (`bagRows`): each example's weight and target ([w, t], zero for an example with no row), once
/// on the device, and the draw's settings. `None` when the host should draw: too few rows for a bag's total to stay
/// near 1 (the fixed-point accumulators hold under 2), or a row heavy enough to push it there alone.
struct DeviceBags {
    base: metal::Buffer,
    keep24: u32,
    sample: u32,
    scale: f32,
}

impl DeviceBags {
    fn new(rows: &[(f32, f32, u32)], num_examples: usize, config: &RandomForestProcedureConfig) -> Result<Option<DeviceBags>, String> {
        if config.bags != BagDraw::Device {
            return Ok(None);
        }
        let sum: f64 = rows.iter().map(|r| r.0 as f64).sum();
        let max = rows.iter().map(|r| r.0 as f64).fold(0.0, f64::max);
        let (prop, sample) = if config.sample_feature_vectors { (config.feature_vector_sampling_prop.clamp(0.0, 1.0) as f64, 1) } else { (1.0, 0) };
        // The total weight a bag expects is 1: each example kept with `prop` and counted Poisson(1) times.
        let expected = sum * prop;
        if sum <= 0.0 || (sample == 1 && (rows.len() as f64 * prop < DEVICE_BAG_MIN_ROWS || 8.0 * max / expected > 0.25)) {
            return Ok(None);
        }
        let mut base = vec![[0f32; 2]; num_examples];
        for r in rows {
            base[r.2 as usize] = [r.0, r.1];
        }
        let shared = crate::gpu::shared::get().ok_or("no Metal device")?;
        Ok(Some(DeviceBags {
            base: crate::compute_kernel_metal::transfer_to_device_immutable(&shared.device, &base),
            keep24: ((prop * (1u32 << 24) as f64).ceil() as u32).min(1 << 24),
            sample,
            scale: (1.0 / expected) as f32,
        }))
    }

    /// Bag `b`'s rows, drawn on `partition`'s queue (ahead of its tree's work).
    fn draw(&self, partition: &FeatureSamplingTrainerPartition, num_examples: usize, b: usize) -> Result<TreeRows, String> {
        use crate::compute_kernel_grid::ComputeKernelArgument::{Buffer as B, Float, Int};
        let shared = crate::gpu::shared::get().ok_or("no Metal device")?;
        let decoded = crate::compute_kernel_metal::alloc_uninitialized_array_sync::<crate::randomforest_types::RowWeights>(&shared.device, num_examples);
        partition.launch("bagRows", &[
            ("numExamples", Int(num_examples as i64)),
            ("seed", Int(bag_seed(b) as i64)),
            ("keep24", Int(self.keep24 as i64)),
            ("sample", Int(self.sample as i64)),
            ("scale", Float(self.scale as f64)),
            ("baseRows", B(&self.base)),
            ("decodedRowsOut", B(&decoded)),
        ], &[num_examples as u32])?;
        Ok(TreeRows::on_device(decoded))
    }
}

/// Bag `b`'s seed for `bagRows`.
pub fn bag_seed(b: usize) -> u32 {
    245u32.wrapping_add(b as u32)
}

/// The host's copy of `bagCount` (the kernels'): example `e`'s count in a bag of `seed`.
pub fn bag_count(seed: u32, e: u32, keep24: u32, sample: u32) -> u32 {
    fn hash(mut x: u32) -> u32 {
        x ^= x >> 16;
        x = x.wrapping_mul(0x7feb_352d);
        x ^= x >> 15;
        x = x.wrapping_mul(0x846c_a68b);
        x ^ (x >> 16)
    }
    const POISSON1_24: [u32; 8] = [6171851, 12343702, 15429615, 16458249, 16715393, 16766823, 16775396, 16776615];
    if sample == 0 {
        return 1;
    }
    let h = hash(e.wrapping_mul(0x9E37_79B9) ^ hash(seed));
    if h >> 8 >= keep24 {
        return 0;
    }
    let u = hash(h ^ 0x68E3_1DA4) >> 8;
    POISSON1_24.iter().filter(|&&t| u >= t).count() as u32
}

/// The one-trainer forest's trees, bag after bag (each bag's in sampling order), as `Tree`s or flat.
fn train_weighted(all_data: &PartitionData, config: &RandomForestProcedureConfig, flat: bool) -> Result<Vec<Trained>, String> {
    use rayon::prelude::*;
    const TREES_IN_FLIGHT: usize = 4;
    let n = all_data.rows.row_count();
    let mut data = all_data.with_bucket_memory(&mut MemorySerializer);
    data.set_categorical_subsets(config.categorical_subsets);
    if config.min_leaf_examples > 1 {
        data.fs = Arc::new(DatasetFeatureSpace { min_leaf_examples: config.min_leaf_examples, ..(*data.fs).clone() });
    }
    // Each row's weight, target and example number: a tree's weights are by example number (its row r is example
    // r, the examples the rows leave out weighing nothing).
    let rows: Vec<(f32, f32, u32)> = (0..n).map(|i| { let r = data.rows.get_decoded_row(i); (r.weight, r.target, r.example_num) }).collect();
    let trainer = FeatureSamplingTrainerKernel::init_for_tree_rows("forest", config.max_depth, &data.rows, &data.features, data.bucket_memory.clone(), &data.fs)?;
    let num_examples = trainer.num_rows as usize;
    let device_bags = DeviceBags::new(&rows, num_examples, config)?;
    let cols = if flat { Some(FlatColumns::new(&data.features, &data.fs)?) } else { None };
    let mut row_of_example = vec![u32::MAX; num_examples];
    for (i, r) in rows.iter().enumerate() {
        row_of_example[r.2 as usize] = i as u32;
    }
    let bags = config.feature_vector_samplings;
    let samplings: Vec<Vec<Vec<i32>>> = (0..bags).map(|b| bag_feature_samplings(&data, b, config)).collect::<Result<_, _>>()?;
    let threads = bags.clamp(1, if config.max_threads > 0 { config.max_threads } else { MAX_BAGS_AT_ONCE });
    let chunks: Vec<Vec<usize>> = (0..threads).map(|t| (t..bags).step_by(threads).collect()).collect();
    let qos = config.qos;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .start_handler(move |_| set_qos(qos))
        .build()
        .map_err(|e| e.to_string())?;
    let trained: Vec<Result<Vec<(usize, Vec<Trained>)>, String>> = pool.install(|| chunks.par_iter().map(|chunk| {
        let mine: Vec<Vec<i32>> = chunk.iter().flat_map(|&b| samplings[b].iter().cloned()).collect();
        let mut partition = FeatureSamplingTrainerPartition::allocate(&trainer, &mine, "forest")?;
        let mut in_flight = std::collections::VecDeque::new();
        let mut out: Vec<(usize, Vec<Trained>)> = Vec::new();
        let finish = |partition: &FeatureSamplingTrainerPartition, (b, i, scheduled, d): (usize, usize, crate::randomforest::ScheduledTree, crate::randomforest::PartitionControlData), out: &mut Vec<(usize, Vec<Trained>)>| -> Result<(), String> {
            scheduled.event.await_()?;
            let err = |e: String| format!("error training bag {b}: {e}");
            let tree = match &cols {
                Some(cols) => Trained::Flat(match partition.finish_flat(&d, &scheduled, cols).map_err(err)? {
                    Some(t) => t,
                    None => {
                        let mut tree = Tree::new();
                        tree.root = get_leaf(&mut tree, &d.root_w_all(&trainer)?);
                        FlatTree::from_tree(&data.fs, &tree)?
                    }
                }),
                None => {
                    let mut tree = partition.finish(i as i32, &format!("bag {b}"), &d, &scheduled).map_err(err)?;
                    if !tree.root.is_some() {
                        tree.root = get_leaf(&mut tree, &d.root_w_all(&trainer)?);
                    }
                    Trained::Tree(tree)
                }
            };
            partition.release(&scheduled);
            match out.last_mut() {
                Some((last, trees)) if *last == b => trees.push(tree),
                _ => out.push((b, vec![tree])),
            }
            Ok(())
        };
        for &b in chunk {
            let tree_rows = match &device_bags {
                Some(bags) => bags.draw(&partition, num_examples, b)?,
                None => {
                    let counts = bag_counts(n, b, config);
                    let total: f64 = rows.iter().zip(&counts).map(|(r, &c)| r.0 as f64 * c as f64).sum();
                    if total == 0.0 {
                        return Err(format!("error training bag {b}: No training weight in RandomForest bag; try training with more examples"));
                    }
                    let scale = 1.0 / total;
                    let row = |e: usize| row_of_example[e] as usize;
                    TreeRows::new(num_examples, |e| if row_of_example[e] == u32::MAX { 0.0 } else { (rows[row(e)].0 as f64 * counts[row(e)] as f64 * scale) as f32 }, |e| if row_of_example[e] == u32::MAX { 0.0 } else { rows[row(e)].1 })?
                }
            };
            for (i, af) in samplings[b].iter().enumerate() {
                let d = partition.init_with_rows(i as i32, b as i32, &format!("bag {b}"), af, Some(tree_rows.clone())).map_err(|e| format!("error training bag {b}: {e}"))?;
                let scheduled = partition.schedule_training(&format!("bag {b}"), &d).map_err(|e| format!("error training bag {b}: {e}"))?;
                in_flight.push_back((b, i, scheduled, d));
                if in_flight.len() > TREES_IN_FLIGHT {
                    finish(&partition, in_flight.pop_front().unwrap(), &mut out)?;
                }
            }
        }
        while let Some(t) = in_flight.pop_front() {
            finish(&partition, t, &mut out)?;
        }
        Ok(out)
    }).collect());
    let mut by_bag: Vec<Vec<Trained>> = (0..bags).map(|_| Vec::new()).collect();
    for t in trained {
        for (b, trees) in t? {
            by_bag[b].extend(trees);
        }
    }
    let total = bags * config.feature_samplings;
    let trees: Vec<Trained> = by_bag.into_iter().flatten().collect();
    if trees.len() != total {
        return Err(format!("{} trees for {total} samplings", trees.len()));
    }
    Ok(trees)
}

/// The forest bag by bag (upstream's): each bag's rows compacted (`bag`), and a trainer per bag.
pub fn train_forest_compacted(all_data: &PartitionData, config: &RandomForestProcedureConfig) -> Result<Forest, String> {
    let total = config.feature_vector_samplings * config.feature_samplings;
    let pool = rayon::ThreadPoolBuilder::new().num_threads(config.feature_vector_samplings.clamp(1, MAX_BAGS_AT_ONCE)).build().map_err(|e| e.to_string())?;
    use rayon::prelude::*;
    let bags: Vec<Result<Vec<Tree>, String>> = pool.install(|| {
        (0..config.feature_vector_samplings).into_par_iter().map(|b| {
            let debug_name = format!("bag {b}");
            let (mut data, samplings) = bag(all_data, b, config).map_err(|e| format!("error training {debug_name}: {e}"))?;
            data.set_categorical_subsets(config.categorical_subsets);
            if config.min_leaf_examples > 1 {
                data.fs = Arc::new(DatasetFeatureSpace { min_leaf_examples: config.min_leaf_examples, ..(*data.fs).clone() });
            }
            let trees = data.train_multiple_samplings(b as i32, &debug_name, config.max_depth, &samplings, config.training_scheme).map_err(|e| format!("error training {debug_name}: {e}"))?;
            if trees.len() != samplings.len() {
                return Err(format!("{debug_name}: {} trees for {} samplings", trees.len(), samplings.len()));
            }
            Ok(trees)
        }).collect()
    });
    let mut trees = Vec::with_capacity(total);
    for b in bags {
        trees.extend(b?);
    }
    Ok(Forest { fs: all_data.fs.clone(), weights: vec![1.0 / total as f32; total], trees, target_scale: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compute_kernel_metal::transfer_to_host_sync;
    use crate::randomforest::tests::deep_dataset;
    use crate::randomforest::{compare_trees, FeatureSamplingTrainerKernel};
    use crate::randomforest_recursive::decode_rows;
    use crate::randomforest_types::{PartitionIndex, WeightFormat};

    #[test]
    fn mt19937_is_the_standard_one() {
        // The C++ standard's check: the 10000th value of a default-constructed std::mt19937 (seed 5489).
        let mut rng = Mt19937::new(5489);
        let v = (0..10000).map(|_| rng.next_u32()).last().unwrap();
        assert_eq!(v, 4123659995);
    }

    /// The training rows of a dataset with the label, weight and where columns given; rows dropped as asked.
    fn training(n: u32, weight: &dyn Fn(usize) -> f32) -> (PartitionData, Vec<f32>, Vec<f32>, Vec<f32>) {
        let d = deep_dataset(n);
        let labels: Vec<f32> = (0..n as usize).map(|i| if i % 11 == 5 { f32::NAN } else { d.rows.get_row(i).label() as u8 as f32 }).collect();
        let wheres: Vec<f32> = (0..n as usize).map(|i| if i % 7 == 3 { 0.0 } else { 1.0 }).collect();
        let weights: Vec<f32> = (0..n as usize).map(|i| if i % 13 == 2 { 0.0 } else { weight(i) }).collect();
        let all = training_data(d.fs.clone(), &labels, Some(&wheres), Some(&weights)).unwrap();
        (all, labels, wheres, weights)
    }

    /// Every bag row is the dataset row it came from (its label, its weight times its count, scaled, and its
    /// buckets), with rows left out of the training data: upstream's rows read other rows' buckets past the first
    /// row left out. With every row kept, the last rows are in some bag's draw (upstream never drew the last
    /// `n % 32`); with a proportion, a bag draws about that proportion times 1 - 1/e of the rows; each tree has
    /// some of the features.
    #[test]
    fn bag_rows_are_their_dataset_rows() {
        let n = 2003;
        let (all, labels, wheres, weights) = training(n, &|i| 1.0 + (i % 7) as f32 * 0.37);
        let kept: Vec<usize> = (0..n as usize).filter(|&i| !labels[i].is_nan() && wheres[i] != 0.0 && weights[i] > 0.0).collect();
        assert_eq!(all.rows.row_count(), kept.len());
        let config = RandomForestProcedureConfig { feature_vector_samplings: 3, feature_samplings: 4, feature_vector_sampling_prop: 1.0, ..Default::default() };
        let mut drawn = vec![false; kept.len()];
        for b in 0..3 {
            let counts = bag_counts(kept.len(), b, &config);
            assert_eq!(counts.iter().sum::<u32>() as usize, kept.len());
            let (data, samplings) = bag(&all, b, &config).unwrap();
            assert_eq!(data.rows.weight_encoder.weight_format, WeightFormat::WfTable);
            assert!((data.rows.w_all.total() - 1.0).abs() < 1e-4, "bag weights total {}", data.rows.w_all.total());
            let total: f64 = (0..kept.len()).map(|k| weights[kept[k]] as f64 * counts[k] as f64).sum();
            let mut j = 0;
            for (k, &c) in counts.iter().enumerate() {
                if c == 0 {
                    continue;
                }
                drawn[k] = true;
                let src = kept[k];
                let row = data.rows.get_decoded_row(j);
                assert_eq!(row.label, labels[src] != 0.0, "bag {b} row {j}");
                assert_eq!(row.weight, (weights[src] as f64 * c as f64 / total) as f32, "bag {b} row {j}");
                for (f, feat) in data.features.iter().enumerate() {
                    let col = all.fs.column_info.values().find(|c| c.index == f as i32).unwrap();
                    assert_eq!(feat.buckets.get(j as u32), col.buckets.get(src as u32), "bag {b} row {j} (dataset row {src}) feature {f}");
                }
                j += 1;
            }
            assert_eq!(j, data.rows.row_count());
            assert_eq!(samplings.len(), 4);
            for s in &samplings {
                assert!(!s.is_empty() && s.len() < data.features.len(), "{s:?}");
            }
        }
        assert!(drawn[kept.len() - 5..].iter().any(|&d| d), "the last rows are drawn");
        let config = RandomForestProcedureConfig { feature_vector_sampling_prop: 0.3, ..config };
        let counts = bag_counts(kept.len(), 0, &config);
        let share = counts.iter().filter(|&&c| c > 0).count() as f64 / kept.len() as f64;
        assert!((share - 0.3 * (1.0 - (-1f64).exp())).abs() < 0.03, "share of rows drawn {share}");
        let (_, samplings) = bag(&all, 0, &RandomForestProcedureConfig { feature_samplings: 20, ..config }).unwrap();
        let mean = samplings.iter().map(|s| s.len()).sum::<usize>() as f64 / 20.0;
        assert!((mean - 0.3 * 14.0).abs() < 1.5, "mean features per tree {mean}");
    }

    /// The device decodes a bag's rows as the host does whatever their weight format: int multiples (uniform
    /// weights), a table (weights that are not multiples of each other), floats (4096 weights or more). Upstream's
    /// device read every format as int multiples, and lost the label above bit 31.
    #[test]
    fn the_device_decodes_every_weight_format() {
        if crate::gpu::shared::get().is_none() {
            eprintln!("no Metal device: skipped");
            return;
        }
        let config = RandomForestProcedureConfig { feature_vector_sampling_prop: 1.0, ..Default::default() };
        let cases: [(&dyn Fn(usize) -> f32, WeightFormat); 3] = [(&|_| 1.0, WeightFormat::WfIntMultiple), (&|i| 1.0 + (i % 7) as f32 * 0.37, WeightFormat::WfTable), (&|i| 1.0 + (i % 5000) as f32 * 1e-3, WeightFormat::WfFloat)];
        for (weight, format) in cases {
            let (all, ..) = training(20000, weight);
            let (data, _) = bag(&all, 0, &config).unwrap();
            assert_eq!(data.rows.weight_encoder.weight_format, format);
            let trainer = FeatureSamplingTrainerKernel::init("decode", 4, &data.rows, &data.features, data.bucket_memory.clone(), &data.fs).unwrap();
            let device = transfer_to_host_sync::<[f32; 2]>(&trainer.expanded_row_data, data.rows.row_count());
            assert_eq!(device, decode_rows(&data.rows), "{format:?}");
        }
    }

    /// A forest trained on the device is the forest the recursive trainer gives, tree by tree, and the same again
    /// when trained again; the bags' weights are tables (non-integer weights).
    #[test]
    fn forests_on_the_device_match_the_recursive_trainer() {
        if crate::gpu::shared::get().is_none() {
            eprintln!("no Metal device: skipped");
            return;
        }
        let (all, ..) = training(5000, &|i| 1.0 + (i % 7) as f32 * 0.37);
        let config = RandomForestProcedureConfig { feature_vector_samplings: 2, feature_samplings: 3, max_depth: 8, bags: BagDraw::Host, ..Default::default() };
        let device = train_forest(&all, &config).unwrap();
        let again = train_forest(&all, &config).unwrap();
        let recursive = train_forest(&all, &RandomForestProcedureConfig { training_scheme: TrainingScheme::Recursive, ..config.clone() }).unwrap();
        assert_eq!(device.trees.len(), 6);
        assert!(device.weights.iter().all(|&w| w == 1.0 / 6.0));
        for (i, t) in device.trees.iter().enumerate() {
            for (other, what) in [(&again.trees[i], "again"), (&recursive.trees[i], "recursive")] {
                let mut why = String::new();
                assert!(compare_trees(other, other.root, t, t.root, PartitionIndex::root(), &all.fs, &mut why), "tree {i}, {what}:\n{why}");
            }
            assert!(t.node(t.root).is_some(), "tree {i} splits");
        }
    }

    /// One trainer over all the rows, each tree weighing them by its bag (`train_forest_weighted`), gives the trees
    /// of a trainer per compacted bag (`train_forest_compacted`), node for node: classification and regression,
    /// rows left out and weighted, both categorical splits, a minimum leaf, and a sample per tree.
    #[test]
    fn one_trainer_over_weighted_rows_is_a_trainer_per_bag() {
        if crate::gpu::shared::get().is_none() {
            return;
        }
        let n = 3000u32;
        let d = deep_dataset(n);
        let bucket = |f: usize, r: u32| d.features[f].buckets.get(r) as f32;
        let labels: Vec<f32> = (0..n).map(|r| if r % 11 == 5 { f32::NAN } else { d.rows.get_row(r as usize).label() as u8 as f32 }).collect();
        let targets: Vec<f32> = (0..n).map(|r| if r % 17 == 4 { f32::NAN } else { 3.0 * bucket(0, r) - 2.0 * bucket(3, r) + ((r * 7919) % 10) as f32 * 0.1 }).collect();
        let wheres: Vec<f32> = (0..n).map(|r| if r % 7 == 3 { 0.0 } else { 1.0 }).collect();
        let weights: Vec<f32> = (0..n).map(|r| 1.0 + (r % 3) as f32).collect();
        let mut real = (*d.fs).clone();
        real.label_info = crate::dataset_feature_space::FeatureType::Real;
        let real = Arc::new(real);
        let base = RandomForestProcedureConfig { feature_vector_samplings: 3, feature_samplings: 2, max_depth: 8, feature_sampling_prop: 0.6, bags: BagDraw::Host, ..Default::default() };
        let configs = [
            base.clone(),
            RandomForestProcedureConfig { categorical_subsets: true, min_leaf_examples: 4, ..base.clone() },
            RandomForestProcedureConfig { feature_vector_samplings: 10, feature_samplings: 1, max_depth: 12, ..base.clone() },
        ];
        for (c, config) in configs.iter().enumerate() {
            for regression in [false, true] {
                let all = if regression {
                    regression_training_data(real.clone(), &targets, Some(&wheres), Some(&weights)).unwrap().0
                } else {
                    training_data(d.fs.clone(), &labels, Some(&wheres), Some(&weights)).unwrap()
                };
                let one = train_forest_weighted(&all, config).unwrap();
                let per_bag = train_forest_compacted(&all, config).unwrap();
                assert_eq!(one.trees.len(), per_bag.trees.len());
                for (i, (a, b)) in one.trees.iter().zip(&per_bag.trees).enumerate() {
                    let mut why = String::new();
                    assert!(compare_trees(b, b.root, a, a.root, PartitionIndex::root(), &all.fs, &mut why), "config {c} regression {regression} tree {i}:\n{why}");
                }
                // Built flat from the device's splits, the same forest as the trees flattened.
                assert!(FlatForest::new(&one).unwrap() == train_flat_forest(&all, config).unwrap(), "config {c} regression {regression}: flat");
            }
        }
    }

    /// The device's bags (`bagRows`) are the host's copy of the draw (`bag_count`), weight for weight: each example
    /// kept with the proportion, counted Poisson(1) times, a bag's total near 1.
    #[test]
    fn device_bags_are_the_hosts_draw() {
        let Some(shared) = crate::gpu::shared::get() else { return };
        let n = 20_000usize;
        let base: Vec<[f32; 2]> = (0..n).map(|e| if e % 13 == 0 { [0.0, 0.0] } else { [1.0 + (e % 3) as f32, ((e * 7919) % 100) as f32 / 100.0] }).collect();
        let (prop, keep24) = (0.3f64, ((0.3 * (1u32 << 24) as f64).ceil() as u32));
        let sum: f64 = base.iter().map(|b| b[0] as f64).sum();
        let scale = (1.0 / (sum * prop)) as f32;
        let base_buf = crate::compute_kernel_metal::transfer_to_device_immutable(&shared.device, &base);
        let out = crate::compute_kernel_metal::alloc_uninitialized_array_sync::<[f32; 2]>(&shared.device, n);
        let mut q = crate::compute_kernel_metal::MetalComputeQueue::new(&shared.queue);
        for b in [0usize, 1, 57] {
            use crate::compute_kernel_grid::ComputeKernelArgument::{Buffer as B, Float, Int};
            let k = crate::randomforest_kernels_grid::kernel("bagRows").unwrap();
            let mut ctx = q.new_bind_context(k.function);
            k.enqueue(&[("numExamples", Int(n as i64)), ("seed", Int(bag_seed(b) as i64)), ("keep24", Int(keep24 as i64)), ("sample", Int(1)), ("scale", Float(scale as f64)), ("baseRows", B(&base_buf)), ("decodedRowsOut", B(&out))], &[n as u32], &mut ctx).unwrap();
            drop(ctx);
            q.flush().await_().unwrap();
            let got = crate::compute_kernel_metal::transfer_to_host_sync::<[f32; 2]>(&out, n);
            let (mut kept, mut counted, mut total) = (0usize, 0u64, 0f64);
            for e in 0..n {
                let c = bag_count(bag_seed(b), e as u32, keep24, 1);
                let w = base[e][0] * c as f32 * scale;
                let want = if c == 0 { [0.0, 0.0] } else { crate::randomforest_types::row_weights(w, base[e][1]) };
                // The same count (zero exactly where the host's is); the weights to the float's last bits (Metal's
                // fast math orders the products its own way).
                let close = |a: f32, b: f32| (a - b).abs() <= 1e-6 * b.abs().max(1e-30);
                assert!((got[e] == [0.0, 0.0]) == (want == [0.0, 0.0]) && close(got[e][0], want[0]) && close(got[e][1], want[1]), "bag {b} example {e} (count {c}): {:?} {want:?}", got[e]);
                if base[e][0] > 0.0 && (h_kept(bag_seed(b), e as u32, keep24)) {
                    kept += 1;
                    counted += c as u64;
                }
                total += (got[e][0] + got[e][1]) as f64;
            }
            let rows = n - n.div_ceil(13);
            let (share, mean) = (kept as f64 / rows as f64, counted as f64 / kept as f64);
            assert!((share - prop).abs() < 0.02 && (mean - 1.0).abs() < 0.05 && (total - 1.0).abs() < 0.05, "bag {b}: kept {share:.3}, mean count {mean:.3}, total {total:.3}");
        }
        assert_ne!((0..1000).map(|e| bag_count(bag_seed(0), e, keep24, 1)).collect::<Vec<_>>(), (0..1000).map(|e| bag_count(bag_seed(1), e, keep24, 1)).collect::<Vec<_>>(), "bags differ");
    }

    /// Whether `bag_count` kept example `e` (before its Poisson count, which may be 0).
    fn h_kept(seed: u32, e: u32, keep24: u32) -> bool {
        fn hash(mut x: u32) -> u32 {
            x ^= x >> 16;
            x = x.wrapping_mul(0x7feb_352d);
            x ^= x >> 15;
            x = x.wrapping_mul(0x846c_a68b);
            x ^ (x >> 16)
        }
        hash(e.wrapping_mul(0x9E37_79B9) ^ hash(seed)) >> 8 < keep24
    }

    /// With the device's bags, the forest built flat is the trees flattened, and both are deterministic.
    #[test]
    fn device_bagged_flat_forest_is_the_trees_flattened() {
        if crate::gpu::shared::get().is_none() {
            return;
        }
        let n = 3000u32;
        let d = deep_dataset(n);
        let bucket = |f: usize, r: u32| d.features[f].buckets.get(r) as f32;
        let targets: Vec<f32> = (0..n).map(|r| 3.0 * bucket(0, r) - 2.0 * bucket(3, r) + ((r * 7919) % 10) as f32 * 0.1).collect();
        let mut real = (*d.fs).clone();
        real.label_info = crate::dataset_feature_space::FeatureType::Real;
        let all = regression_training_data(Arc::new(real), &targets, None, None).unwrap().0;
        let config = RandomForestProcedureConfig { feature_vector_samplings: 20, feature_samplings: 1, feature_sampling_prop: 1.0, max_depth: 12, min_leaf_examples: 5, ..Default::default() };
        assert_eq!(config.bags, BagDraw::Device);
        let trees = train_forest_weighted(&all, &config).unwrap();
        let flat = train_flat_forest(&all, &config).unwrap();
        assert!(FlatForest::new(&trees).unwrap() == flat, "flat");
        assert!(train_flat_forest(&all, &config).unwrap() == flat, "deterministic");
        // The device's bags differ from the host's: another forest.
        assert!(train_flat_forest(&all, &RandomForestProcedureConfig { bags: BagDraw::Host, ..config.clone() }).unwrap() != flat);
    }

    /// A regression forest: trained on the device it is the recursive trainer's, tree by tree (soft labels, the
    /// variance criterion, and the targets carried through the bags and splits); its predictions follow the
    /// targets, on the host and on the device.
    #[test]
    fn regression_forests_on_the_device_match_the_recursive_trainer() {
        if crate::gpu::shared::get().is_none() {
            eprintln!("no Metal device: skipped");
            return;
        }
        let n = 5000u32;
        let d = deep_dataset(n);
        let mut fs = (*d.fs).clone();
        fs.label_info = crate::dataset_feature_space::FeatureType::Real;
        let fs = Arc::new(fs);
        let bucket = |f: usize, r: u32| d.features[f].buckets.get(r) as f32;
        // A function of three features, with noise; some rows without a target, or left out.
        let targets: Vec<f32> = (0..n).map(|r| if r % 17 == 4 { f32::NAN } else { 10.0 + 3.0 * bucket(0, r) - 2.0 * bucket(3, r) + bucket(12, r) * bucket(1, r) + ((r * 7919) % 10) as f32 * 0.1 }).collect();
        let wheres: Vec<f32> = (0..n).map(|r| if r % 7 == 3 { 0.0 } else { 1.0 }).collect();
        let config = RandomForestProcedureConfig { feature_vector_samplings: 2, feature_samplings: 3, max_depth: 8, feature_sampling_prop: 1.0, bags: BagDraw::Host, ..Default::default() };
        let device = train_regression_forest(fs.clone(), &targets, Some(&wheres), None, &config).unwrap();
        let recursive = train_regression_forest(fs.clone(), &targets, Some(&wheres), None, &RandomForestProcedureConfig { training_scheme: TrainingScheme::Recursive, ..config.clone() }).unwrap();
        let scale = device.target_scale.unwrap();
        assert_eq!(device.target_scale, recursive.target_scale);
        for (i, t) in device.trees.iter().enumerate() {
            let mut why = String::new();
            assert!(compare_trees(&recursive.trees[i], recursive.trees[i].root, t, t.root, PartitionIndex::root(), &fs, &mut why), "tree {i}:\n{why}");
        }
        // The predictions: the mean squared error is a small part of the targets' variance.
        // The dataset's bucket b is the value b.
        let columns: Vec<Vec<crate::bucket::CellValue>> = (0..d.features.len()).map(|f| (0..n).map(|r| crate::bucket::CellValue::Float(bucket(f, r) as f64)).collect()).collect();
        let rows = crate::randomforest_predict::RowBuckets::from_values(&device.fs, &columns).unwrap();
        let host = device.predict_host(&rows).unwrap();
        let on_device = crate::randomforest_predict::FlatForest::new(&device).unwrap().predict_device(&rows).unwrap();
        let kept: Vec<usize> = (0..n as usize).filter(|&r| targets[r].is_finite()).collect();
        let mean = kept.iter().map(|&r| targets[r] as f64).sum::<f64>() / kept.len() as f64;
        let var = kept.iter().map(|&r| (targets[r] as f64 - mean).powi(2)).sum::<f64>() / kept.len() as f64;
        let mse = kept.iter().map(|&r| (device.regression_value(host[r]).unwrap() - targets[r] as f64).powi(2)).sum::<f64>() / kept.len() as f64;
        eprintln!("regression: targets {:.2}..{:.2}, variance {var:.3}, forest mse {mse:.3}", scale.lo, scale.hi);
        assert!(mse < 0.25 * var, "mse {mse}, variance {var}");
        for r in 0..n as usize {
            assert!((host[r][1] - on_device[r][1]).abs() < 1e-5, "row {r}: {:?} {:?}", host[r], on_device[r]);
        }
    }
}
