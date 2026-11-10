//! randomforest_procedure.h and the bagging of randomforest_procedure.cc, in Rust.
//!
//! Mathieu Marquis Bolduc, 11 March 2016. Copyright (c) 2016 mldb.ai inc. This file is part of MLDB. Apache-2.0
//! (see the LICENSE at `accel/src/forest/mldb/`).
//!
//! Trains a random forest binary classifier: the training rows (`training_data`: the rows kept, their labels and
//! weights), then per feature vector sampling a bag (`bag`: a bootstrap of the rows, in 32 stretches, reweighted
//! and compacted) and per feature sampling a tree over ceil(sqrt(n)) of the active features, all of a bag's trees
//! trained together (`trainMultipleSamplings`, on the device); the forest weighs every tree the same.
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
//! - the counts are 32 bit (upstream's 8 bit counts could wrap).
//! As upstream, `featureVectorSamplingProp` and `featureSamplingProp` are not used (the bag is a bootstrap of all
//! the rows; a tree's features are ceil(sqrt(n))), and the shuffle of the rows in training (all of them,
//! `trainprop` = 1) is not brought over, as it has no effect. Bags are trained one after the other (upstream: 8 at
//! once on CPU threads), each bag's trees together on the device. The RNG is MT19937 as upstream; the shuffles are
//! Fisher-Yates over `rng() % n`, as `std::shuffle` is not the same across C++ libraries.

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
    /// Proportion of feature vectors to select in each sample (unused, as upstream).
    pub feature_vector_sampling_prop: f32,
    /// Number of samplings of features (trees per bag).
    pub feature_samplings: usize,
    /// Proportion of features to select in each sample (unused, as upstream).
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
        writer.add_row_values(labels[i] != 0.0, weight(i), i as u32)?;
    }
    all_data.rows = writer.freeze(&mut s);
    Ok(all_data)
}

/// Bag `bag`'s count per row: a bootstrap in up to 32 stretches of the rows, each its own (seeded) draw of as many
/// rows as it has, or 1 each without sampling.
pub fn bag_counts(num_rows: usize, bag: usize, config: &RandomForestProcedureConfig) -> Vec<u32> {
    let num_partitions = num_rows.min(32);
    let mut counts = vec![0u32; num_rows];
    use rayon::prelude::*;
    let per = num_rows / num_partitions.max(1);
    let stretches: Vec<(usize, usize)> = (0..num_partitions).map(|i| (per * i, if i == num_partitions - 1 { num_rows } else { per * (i + 1) })).collect();
    let parts: Vec<Vec<u32>> = stretches.par_iter().enumerate().map(|(i, &(first, last))| {
        let n = last - first;
        if !config.sample_feature_vectors {
            return vec![1; n];
        }
        let seed = 245 + (bag * num_partitions) as u32 + i as u32;
        let mut myrng = random_forest_rng(seed);
        let mut w = vec![0u32; n];
        for _ in 0..n {
            w[myrng.below(n)] += 1;
        }
        w
    }).collect();
    for ((first, _), part) in stretches.iter().zip(parts) {
        counts[*first..*first + part.len()].copy_from_slice(&part);
    }
    counts
}

/// Bag `bag`: its rows (`reweightAndCompact`, the weights totalling 1) and the active features of each of its
/// trees (ceil(sqrt(n)) of the active features, from successive seeded shuffles).
pub fn bag(all_data: &PartitionData, bag: usize, config: &RandomForestProcedureConfig) -> Result<(PartitionData, Vec<Vec<i32>>), String> {
    let counts = bag_counts(all_data.rows.row_count(), bag, config);
    let total: f64 = counts.iter().enumerate().map(|(i, &c)| all_data.rows.get_weight(i) as f64 * c as f64).sum();
    if total == 0.0 {
        return Err("No training weight in RandomForest bag; try training with more examples".into());
    }
    let mut s = MemorySerializer;
    let data = all_data.reweight_and_compact(&counts, 1.0 / total, &mut s)?;
    let mut active_feature_list: Vec<i32> = data.features.iter().enumerate().filter(|(_, f)| f.active).map(|(i, _)| i as i32).collect();
    let mut sampling_active_features = Vec::with_capacity(config.feature_samplings);
    for i in 0..config.feature_samplings {
        let mut rng = Mt19937::new((bag + 371 + i) as u32);
        let num_features_required = ((active_feature_list.len() as f64).sqrt().ceil() as usize).min(active_feature_list.len());
        if num_features_required == 0 {
            return Err("no active features".into());
        }
        rng.shuffle(&mut active_feature_list);
        sampling_active_features.push(active_feature_list[..num_features_required].to_vec());
    }
    Ok((data, sampling_active_features))
}

/// A forest: its trees (bag after bag, each bag's in sampling order), each weighed the same.
pub struct Forest {
    pub fs: Arc<DatasetFeatureSpace>,
    pub trees: Vec<Tree>,
    pub weights: Vec<f32>,
}

/// Trains the forest over the training rows (`training_data`).
pub fn train_forest(all_data: &PartitionData, config: &RandomForestProcedureConfig) -> Result<Forest, String> {
    let total = config.feature_vector_samplings * config.feature_samplings;
    let mut trees = Vec::with_capacity(total);
    for b in 0..config.feature_vector_samplings {
        let debug_name = format!("bag {b}");
        let (data, samplings) = bag(all_data, b, config).map_err(|e| format!("error training {debug_name}: {e}"))?;
        let bag_trees = data.train_multiple_samplings(b as i32, &debug_name, config.max_depth, &samplings, config.training_scheme).map_err(|e| format!("error training {debug_name}: {e}"))?;
        if bag_trees.len() != samplings.len() {
            return Err(format!("{debug_name}: {} trees for {} samplings", bag_trees.len(), samplings.len()));
        }
        trees.extend(bag_trees);
    }
    Ok(Forest { fs: all_data.fs.clone(), weights: vec![1.0 / total as f32; total], trees })
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
    /// row left out. Every row of the dataset is in some bag's draw (upstream never drew the last `n % 32`).
    #[test]
    fn bag_rows_are_their_dataset_rows() {
        let n = 2003;
        let (all, labels, wheres, weights) = training(n, &|i| 1.0 + (i % 7) as f32 * 0.37);
        let kept: Vec<usize> = (0..n as usize).filter(|&i| !labels[i].is_nan() && wheres[i] != 0.0 && weights[i] > 0.0).collect();
        assert_eq!(all.rows.row_count(), kept.len());
        let config = RandomForestProcedureConfig { feature_vector_samplings: 3, feature_samplings: 4, ..Default::default() };
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
                assert_eq!(s.len(), (data.features.len() as f64).sqrt().ceil() as usize);
            }
        }
        assert!(drawn[kept.len() - 5..].iter().any(|&d| d), "the last rows are drawn");
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
        let config = RandomForestProcedureConfig::default();
        let cases: [(&dyn Fn(usize) -> f32, WeightFormat); 3] = [(&|_| 1.0, WeightFormat::WfIntMultiple), (&|i| 1.0 + (i % 7) as f32 * 0.37, WeightFormat::WfTable), (&|i| 1.0 + (i % 5000) as f32 * 1e-3, WeightFormat::WfFloat)];
        for (weight, format) in cases {
            let (all, ..) = training(20000, weight);
            let (data, _) = bag(&all, 0, &config).unwrap();
            assert_eq!(data.rows.weight_encoder.weight_format, format);
            let trainer = FeatureSamplingTrainerKernel::init("decode", 4, &data.rows, &data.features, data.bucket_memory.clone(), &data.fs).unwrap();
            let device = transfer_to_host_sync::<f32>(&trainer.expanded_row_data, data.rows.row_count());
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
}
