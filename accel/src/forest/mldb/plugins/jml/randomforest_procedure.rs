//! randomforest_procedure.h and the bagging of randomforest_procedure.cc, in Rust.
//!
//! Mathieu Marquis Bolduc, 11 March 2016. Copyright (c) 2016 mldb.ai inc. This file is part of MLDB. Apache-2.0
//! (see the LICENSE at `accel/src/forest/mldb/`).
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

use crate::forest::dataset_feature_space::DatasetFeatureSpace;
use crate::forest::memory_region::MemorySerializer;
use crate::forest::randomforest::{PartitionData, TrainingScheme};
use crate::forest::tree::Tree;
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
}

impl Default for RandomForestProcedureConfig {
    fn default() -> Self {
        RandomForestProcedureConfig { feature_vector_samplings: 5, feature_vector_sampling_prop: 0.3, feature_samplings: 20, feature_sampling_prop: 0.3, max_depth: 20, sample_feature_vectors: true, training_scheme: TrainingScheme::Partitioned }
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
    if fs.label_info != crate::forest::dataset_feature_space::FeatureType::Real {
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
fn training_rows(fs: Arc<DatasetFeatureSpace>, labels: &[f32], wheres: Option<&[f32]>, weights: Option<&[f32]>, write: &mut dyn FnMut(&mut crate::forest::randomforest_types::RowWriter, usize, f32) -> Result<(), String>) -> Result<PartitionData, String> {
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
    Ok((data, sampling_active_features))
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
    const MAX_BAGS_AT_ONCE: usize = 8;
    let total = config.feature_vector_samplings * config.feature_samplings;
    let pool = rayon::ThreadPoolBuilder::new().num_threads(config.feature_vector_samplings.clamp(1, MAX_BAGS_AT_ONCE)).build().map_err(|e| e.to_string())?;
    use rayon::prelude::*;
    let bags: Vec<Result<Vec<Tree>, String>> = pool.install(|| {
        (0..config.feature_vector_samplings).into_par_iter().map(|b| {
            let debug_name = format!("bag {b}");
            let (data, samplings) = bag(all_data, b, config).map_err(|e| format!("error training {debug_name}: {e}"))?;
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
    use crate::forest::compute_kernel_metal::transfer_to_host_sync;
    use crate::forest::randomforest::tests::deep_dataset;
    use crate::forest::randomforest::{compare_trees, FeatureSamplingTrainerKernel};
    use crate::forest::randomforest_recursive::decode_rows;
    use crate::forest::randomforest_types::{PartitionIndex, WeightFormat};

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
        let config = RandomForestProcedureConfig { feature_vector_samplings: 2, feature_samplings: 3, max_depth: 8, ..Default::default() };
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
        fs.label_info = crate::forest::dataset_feature_space::FeatureType::Real;
        let fs = Arc::new(fs);
        let bucket = |f: usize, r: u32| d.features[f].buckets.get(r) as f32;
        // A function of three features, with noise; some rows without a target, or left out.
        let targets: Vec<f32> = (0..n).map(|r| if r % 17 == 4 { f32::NAN } else { 10.0 + 3.0 * bucket(0, r) - 2.0 * bucket(3, r) + bucket(12, r) * bucket(1, r) + ((r * 7919) % 10) as f32 * 0.1 }).collect();
        let wheres: Vec<f32> = (0..n).map(|r| if r % 7 == 3 { 0.0 } else { 1.0 }).collect();
        let config = RandomForestProcedureConfig { feature_vector_samplings: 2, feature_samplings: 3, max_depth: 8, feature_sampling_prop: 1.0, ..Default::default() };
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
        let columns: Vec<Vec<crate::forest::bucket::CellValue>> = (0..d.features.len()).map(|f| (0..n).map(|r| crate::forest::bucket::CellValue::Float(bucket(f, r) as f64)).collect()).collect();
        let rows = crate::forest::randomforest_predict::RowBuckets::from_values(&device.fs, &columns).unwrap();
        let host = device.predict_host(&rows).unwrap();
        let on_device = crate::forest::randomforest_predict::FlatForest::new(&device).unwrap().predict_device(&rows).unwrap();
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
