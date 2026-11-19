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

use std::collections::HashMap;

use crate::bucket::{BucketList, CellValue};
use crate::compute_kernel_grid::ComputeKernelArgument;
use crate::compute_kernel_metal::{alloc_uninitialized_array_sync, transfer_to_device_immutable, MetalComputeQueue};
use crate::dataset_feature_space::{ColumnData, DatasetFeatureSpace};
use crate::jml::committee::Committee;
use crate::jml::decision_tree::DecisionTree;
use crate::jml::split::Op;
use crate::jml::tree::{Ptr, Tree};
use crate::memory_region::Pod;
use crate::randomforest_kernels_grid::kernel;
use crate::randomforest_procedure::Forest;

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

/// The bits of a column's buckets at prediction (`ColumnInfo::bucketize`: its buckets and the unseen one).
pub fn prediction_bucket_bits(column: &crate::dataset_feature_space::ColumnInfo) -> u32 {
    crate::bucket::highest_bit(crate::dataset_feature_space::unseen_bucket(&column.bucket_descriptions) + 1, 0) as u32 + 1
}

/// The column index of a feature.
fn column_index(fs: &DatasetFeatureSpace, feature: &crate::jml::feature::Feature) -> Result<usize, String> {
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
    index: std::collections::HashMap<crate::jml::feature::Feature, usize>,
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
    pub feature: u16,
    /// The bits of each of the feature's buckets (`RowBuckets` packs the feature's column with them).
    pub bucket_bits: u8,
    /// 1: true is below the split bucket; 0: true is the split bucket; 2: true is in subset `split_bucket`.
    pub ordinal: u8,
    pub split_bucket: u32,
    /// A node's index when at least 0; leaf -(child + 1) when below 0.
    pub child_true: i32,
    pub child_false: i32,
}
unsafe impl Pod for PredictNode {}

/// A forest flattened for the device: every tree's nodes in one array, each tree's root, the trees' weights and
/// the leaves' distributions (two per leaf).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FlatForest {
    pub nodes: Vec<PredictNode>,
    pub roots: Vec<i32>,
    pub weights: Vec<f32>,
    pub leaf_preds: Vec<f32>,
    /// The categorical subset splits' sets, 16 words each (`PredictNode::split_bucket` of an `ordinal` 2 node).
    pub subsets: Vec<u32>,
    /// The deepest walk (nodes from a root to a leaf).
    pub max_depth: u32,
    pub num_features: usize,
    /// Each feature's bucket bits at prediction (`prediction_bucket_bits`), which the rows' columns must have.
    pub feature_bits: Vec<u32>,
}

impl FlatForest {
    /// The forest's prediction for one row on the host, from the row's bucket in each feature (the device's walk,
    /// `predictForestImpl`, tree after tree: the same leaves; the sums may differ in the last bit, in another order).
    pub fn predict_row(&self, buckets: &[u32]) -> Result<[f32; 2], String> {
        let (mut p0, mut p1) = (0f32, 0f32);
        for (&root, &w) in self.roots.iter().zip(&self.weights) {
            let mut at = root;
            for _ in 0..self.max_depth + 1 {
                if at < 0 {
                    break;
                }
                let n = &self.nodes[at as usize];
                let b = buckets[n.feature as usize];
                let is_true = match n.ordinal {
                    1 => b < n.split_bucket,
                    0 => b == n.split_bucket,
                    _ => b < 512 && (self.subsets[n.split_bucket as usize * 16 + (b >> 5) as usize] >> (b & 31)) & 1 != 0,
                };
                at = if is_true { n.child_true } else { n.child_false };
            }
            if at >= 0 {
                return Err("a walk did not reach a leaf".into());
            }
            let leaf = (-(at + 1)) as usize;
            p0 += w * self.leaf_preds[2 * leaf];
            p1 += w * self.leaf_preds[2 * leaf + 1];
        }
        Ok([p0, p1])
    }

    pub fn new(forest: &Forest) -> Result<Self, String> {
        let trees = forest.trees.iter().map(|t| FlatTree::from_tree(&forest.fs, t)).collect::<Result<Vec<_>, _>>()?;
        Ok(FlatForest::from_trees(&forest.fs, trees, forest.weights.clone()))
    }

    /// The forest of flat trees (each tree's arrays after the last's), each with its weight.
    pub fn from_trees(fs: &DatasetFeatureSpace, trees: Vec<FlatTree>, weights: Vec<f32>) -> Self {
        let nf = fs.column_info.len();
        let feature_bits = (0..nf).map(|i| fs.column(i).map_or(0, prediction_bucket_bits)).collect();
        let mut f = FlatForest { num_features: nf, feature_bits, weights, ..Default::default() };
        for t in trees {
            let (node, leaf, set) = (f.nodes.len() as i32, (f.leaf_preds.len() / 2) as i32, (f.subsets.len() / 16) as u32);
            let moved = |c: i32| if c >= 0 { c + node } else { c - leaf };
            f.roots.push(moved(t.root));
            f.nodes.extend(t.nodes.into_iter().map(|mut n| {
                n.child_true = moved(n.child_true);
                n.child_false = moved(n.child_false);
                if n.ordinal == 2 {
                    n.split_bucket += set;
                }
                n
            }));
            f.leaf_preds.extend(t.leaf_preds);
            f.subsets.extend(t.subsets);
            f.max_depth = f.max_depth.max(t.depth);
        }
        f
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
                let feature = column_index(fs, node.split.feature())?;
                let column = fs.column(feature).ok_or("a split's feature has no column")?;
                if node.split_bucket > u16::MAX as i32 || feature > u16::MAX as usize {
                    return Err("a split past 65535 buckets or features".into());
                }
                // A categorical subset split: its set's place among the subsets.
                let (split_bucket, ordinal) = if !node.split_subset.is_empty() {
                    let at = self.subsets.len() / 16;
                    if node.split_subset.len() != 16 {
                        return Err("a subset not of 512 buckets".into());
                    }
                    self.subsets.extend_from_slice(&node.split_subset);
                    (at as u32, 2)
                } else {
                    (node.split_bucket as u32, (node.split.op() == Op::Less) as u8)
                };
                self.nodes.push(PredictNode {
                    feature: feature as u16,
                    bucket_bits: prediction_bucket_bits(column) as u8,
                    split_bucket,
                    ordinal,
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

/// One tree flattened (`FlatForest`'s layout, its indexes its own): its nodes, leaves' distributions and subsets,
/// its root, and its deepest walk.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FlatTree {
    pub nodes: Vec<PredictNode>,
    pub leaf_preds: Vec<f32>,
    pub subsets: Vec<u32>,
    pub root: i32,
    pub depth: u32,
}

impl FlatTree {
    /// A tree flattened (preorder: a node, then its true side, then its false side).
    pub fn from_tree(fs: &DatasetFeatureSpace, tree: &Tree) -> Result<FlatTree, String> {
        let mut f = FlatForest::default();
        let root = f.flatten(fs, tree, tree.root, 0)?;
        Ok(FlatTree { nodes: f.nodes, leaf_preds: f.leaf_preds, subsets: f.subsets, root, depth: f.max_depth })
    }
}

/// What a flat node needs of each trainer feature (by its index among the trainer's features), found once: its
/// column, its bucket bits at prediction, and its split's kind (`PredictNode::ordinal`).
#[derive(Clone, Debug, Default)]
pub struct FlatColumns {
    pub column: Vec<u16>,
    pub bits: Vec<u8>,
    pub kind: Vec<u8>,
}

impl FlatColumns {
    pub fn new(features: &[crate::randomforest_types::Feature], fs: &DatasetFeatureSpace) -> Result<FlatColumns, String> {
        let mut c = FlatColumns::default();
        for f in features {
            let (column, bits) = match f.info.and_then(|h| fs.column_info.get(&h)) {
                Some(info) if info.index <= u16::MAX as _ => (info.index as u16, prediction_bucket_bits(info) as u8),
                Some(_) => return Err("a feature past 65535 columns".into()),
                // A feature with no column is never split on (`get_node` refuses it).
                None => (u16::MAX, 0),
            };
            c.column.push(column);
            c.bits.push(bits);
            c.kind.push(if f.ordinal { 1 } else if f.subsets { 2 } else { 0 });
        }
        Ok(c)
    }
}

/// The threads a row takes in `predictForestKernel`: enough that a batch keeps the device's threads busy (about
/// 64k of them), at most one a tree and at most the group's threads; a power of 2.
pub fn lanes_per_row(num_rows: usize, num_trees: usize) -> u32 {
    let group = crate::randomforest_kernels_grid::PREDICT_GROUP_THREADS as usize;
    let want = (65536 / num_rows.max(1)).clamp(1, group).min(num_trees.max(1).next_power_of_two());
    // The largest power of 2 not above `want`.
    1 << (usize::BITS - 1 - want.leading_zeros())
}

/// Flattened forests resident on the device, uploaded once and predicting batch after batch: every forest over
/// every row in one dispatch (a model of several forests, such as the K-bucket regressor's, is one wait). The rows'
/// and the predictions' buffers are kept from call to call (grown when a batch needs more), so a call allocates
/// nothing on the device: it writes the rows into the kept buffer, dispatches, waits and reads the predictions.
pub struct DeviceForest {
    nodes: metal::Buffer,
    roots: metal::Buffer,
    weights: metal::Buffer,
    leaves: metal::Buffer,
    subsets: metal::Buffer,
    forest_tree_start: metal::Buffer,
    num_forests: u32,
    /// The most trees a forest has (the lanes a prediction takes).
    max_trees: u32,
    max_depth: u32,
    num_features: usize,
    feature_bits: Vec<u32>,
    scratch: std::sync::Mutex<Scratch>,
}

/// What a forest keeps from call to call: the rows' bucket columns and the predictions (with their capacities, in
/// u32 and f32), its queue (whose next command buffer is made as the last commits), and the launch plans by
/// (items, lanes a row, column stride): a call whose batch shape was seen before skips the grid's solve.
#[derive(Default)]
struct Scratch {
    data: Option<(metal::Buffer, usize)>,
    out: Option<(metal::Buffer, usize)>,
    queue: Option<MetalComputeQueue>,
    plans: HashMap<(usize, u32, usize), crate::compute_kernel_grid::GridLaunchPlan>,
    /// `predict_values`: rowBuckets' plans by (rows, stride), and the columns' descriptions, numbers, codes and
    /// code tables.
    row_plans: HashMap<(usize, usize), crate::compute_kernel_grid::GridLaunchPlan>,
    columns: Option<(metal::Buffer, usize)>,
    numbers: Option<(metal::Buffer, usize)>,
    codes: Option<(metal::Buffer, usize)>,
    tables: Option<(metal::Buffer, usize)>,
}

/// The words describing a column to `rowBuckets` (`ROW_BUCKET_WORDS` in the kernels).
const ROW_BUCKET_WORDS: usize = 8;

/// A feature space's bucket rules for prediction rows (`ColumnInfo::bucket_of`): numbers by the numeric splits,
/// strings by name, a null in bucket 0 when the column had nulls, anything else in the unseen bucket. Plain data,
/// built once per feature space: the host bucketizes by them (`bucket_of_number`, `bucket_of_name`) and the device
/// takes them (`DeviceBucketizer`), and they give the same buckets.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BucketRules {
    pub columns: Vec<BucketizerColumn>,
    /// Every numeric column's split keys (`order_key`), column after column.
    pub split_keys: Vec<i64>,
}

/// One column's rules (`BucketRules`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BucketizerColumn {
    pub bits: u32,
    pub split_start: u32,
    pub num_splits: u32,
    pub numeric_offset: u32,
    pub null_bucket: u32,
    pub unseen: u32,
    pub strings: HashMap<String, u32>,
}

impl BucketRules {
    pub fn new(fs: &DatasetFeatureSpace) -> Result<Self, String> {
        use crate::dataset_feature_space::{order_key, unseen_bucket};
        let mut keys: Vec<i64> = Vec::new();
        let mut columns = Vec::new();
        for i in 0..fs.column_info.len() {
            let c = fs.column(i).ok_or_else(|| format!("no column {i}"))?;
            let d = &c.bucket_descriptions;
            let unseen = unseen_bucket(d);
            let numeric = d.numeric.active;
            columns.push(BucketizerColumn {
                bits: prediction_bucket_bits(c),
                split_start: keys.len() as u32,
                num_splits: if numeric { d.numeric.splits.len() as u32 } else { 0 },
                numeric_offset: if numeric { d.numeric.offset } else { unseen },
                null_bucket: if d.has_nulls { 0 } else { unseen },
                unseen,
                strings: d.strings.buckets.iter().enumerate().map(|(j, s)| (s.clone(), d.strings.offset + j as u32)).collect(),
            });
            if numeric {
                keys.extend(d.numeric.splits.iter().map(|&s| order_key(s)));
            }
        }
        Ok(BucketRules { columns, split_keys: keys })
    }

    /// Column `c`'s bucket for a number (a NaN is a null), as `rowBucketOf` gives it.
    pub fn bucket_of_number(&self, c: usize, v: f64) -> u32 {
        let col = &self.columns[c];
        if v.is_nan() {
            return col.null_bucket;
        }
        let k = crate::dataset_feature_space::order_key(v);
        let splits = &self.split_keys[col.split_start as usize..(col.split_start + col.num_splits) as usize];
        col.numeric_offset + splits.partition_point(|&s| s < k) as u32
    }

    /// Column `c`'s bucket for a string (`None`: a null).
    pub fn bucket_of_name(&self, c: usize, name: Option<&str>) -> u32 {
        let col = &self.columns[c];
        match name {
            None => col.null_bucket,
            Some(n) => col.strings.get(n).copied().unwrap_or(col.unseen),
        }
    }
}

/// The bucket rules on the device (`DeviceForest::predict_values`).
pub struct DeviceBucketizer {
    columns: Vec<BucketizerColumn>,
    split_keys: metal::Buffer,
}

impl DeviceBucketizer {
    pub fn new(fs: &DatasetFeatureSpace) -> Result<Self, String> {
        Self::from_rules(&BucketRules::new(fs)?)
    }

    pub fn from_rules(rules: &BucketRules) -> Result<Self, String> {
        let shared = crate::gpu::shared::get().ok_or("no Metal device")?;
        let mut keys = rules.split_keys.clone();
        keys.push(0);
        Ok(DeviceBucketizer { columns: rules.columns.clone(), split_keys: transfer_to_device_immutable(&shared.device, &keys) })
    }
}

/// A kept buffer of at least `n` elements of `T` (`slot`'s when it is big enough, else a new one, twice the size).
fn kept<T>(device: &metal::DeviceRef, slot: &mut Option<(metal::Buffer, usize)>, n: usize) -> metal::Buffer {
    match slot {
        Some((b, cap)) if *cap >= n => b.clone(),
        _ => {
            let cap = (n * 2).max(1024);
            let b = alloc_uninitialized_array_sync::<T>(device, cap);
            *slot = Some((b.clone(), cap));
            b
        }
    }
}

impl DeviceForest {
    pub fn new(flat: &FlatForest) -> Result<Self, String> {
        Self::new_many(&[flat])
    }

    /// Several forests over the same feature space, predicted together: `predict` gives each row's predictions
    /// forest after forest.
    pub fn new_many(flats: &[&FlatForest]) -> Result<Self, String> {
        let first = flats.first().ok_or("no forest")?;
        let mut all = FlatForest { num_features: first.num_features, feature_bits: first.feature_bits.clone(), ..Default::default() };
        let mut starts = vec![0u32];
        for f in flats {
            if f.num_features != first.num_features || f.feature_bits != first.feature_bits {
                return Err("forests over different feature spaces".into());
            }
            let (node0, leaf0, subset0) = (all.nodes.len() as i32, (all.leaf_preds.len() / 2) as i32, (all.subsets.len() / 16) as u32);
            // A child is a node (at least 0) or a leaf (-(leaf + 1)): each moves past the forests before.
            let moved = |c: i32| if c >= 0 { c + node0 } else { c - leaf0 };
            all.nodes.extend(f.nodes.iter().map(|n| PredictNode {
                child_true: moved(n.child_true),
                child_false: moved(n.child_false),
                split_bucket: if n.ordinal == 2 { n.split_bucket + subset0 } else { n.split_bucket },
                ..*n
            }));
            all.roots.extend(f.roots.iter().map(|&r| moved(r)));
            all.weights.extend_from_slice(&f.weights);
            all.leaf_preds.extend_from_slice(&f.leaf_preds);
            all.subsets.extend_from_slice(&f.subsets);
            all.max_depth = all.max_depth.max(f.max_depth);
            starts.push(all.roots.len() as u32);
        }
        let shared = crate::gpu::shared::get().ok_or("no Metal device")?;
        let device = &shared.device;
        let one = [PredictNode::default()];
        Ok(DeviceForest {
            nodes: transfer_to_device_immutable(device, if all.nodes.is_empty() { &one[..] } else { &all.nodes }),
            roots: transfer_to_device_immutable(device, &all.roots),
            weights: transfer_to_device_immutable(device, &all.weights),
            leaves: transfer_to_device_immutable(device, &all.leaf_preds),
            subsets: transfer_to_device_immutable(device, if all.subsets.is_empty() { &[0u32; 16][..] } else { &all.subsets }),
            forest_tree_start: transfer_to_device_immutable(device, &starts),
            num_forests: flats.len() as u32,
            max_trees: starts.windows(2).map(|w| w[1] - w[0]).max().unwrap_or(0),
            max_depth: all.max_depth,
            num_features: all.num_features,
            feature_bits: all.feature_bits,
            scratch: Default::default(),
        })
    }

    pub fn num_forests(&self) -> usize {
        self.num_forests as usize
    }

    /// The predictions for a batch of rows: their buckets written to the kept buffer, one dispatch, one wait. Row
    /// `r`'s prediction by forest `f` is at `r * num_forests() + f`.
    pub fn predict(&self, rows: &RowBuckets) -> Result<Vec<[f32; 2]>, String> {
        Ok(self.predict_timed(rows)?.0)
    }

    /// `predict`, with the seconds the GPU spent on the dispatch.
    pub fn predict_timed(&self, rows: &RowBuckets) -> Result<(Vec<[f32; 2]>, f64), String> {
        if rows.columns.len() != self.num_features {
            return Err(format!("{} columns for a forest over {}", rows.columns.len(), self.num_features));
        }
        if rows.num_rows == 0 {
            return Ok((Vec::new(), 0.0));
        }
        if let Some(i) = (0..self.num_features).find(|&i| rows.columns[i].entry_bits as u32 != self.feature_bits[i]) {
            return Err(format!("column {i} has {}-bit buckets, the forest's {}", rows.columns[i].entry_bits, self.feature_bits[i]));
        }
        let shared = crate::gpu::shared::get().ok_or("no Metal device")?;
        let mut scratch = self.scratch.lock().map_err(|_| "a prediction panicked")?;
        // Every column at the same stride (the widest column's words), so a node finds its column without a load.
        let stride = rows.columns.iter().map(|c| c.storage.data().len()).max().unwrap_or(0) + 1;
        let words = stride * rows.columns.len() + 1;
        let d_data = kept::<u32>(&shared.device, &mut scratch.data, words);
        let data = unsafe { std::slice::from_raw_parts_mut(d_data.contents().cast::<u32>(), words) };
        for (i, c) in rows.columns.iter().enumerate() {
            data[i * stride..i * stride + c.storage.data().len()].copy_from_slice(c.storage.data());
        }
        self.launch(&mut scratch, &d_data, rows.num_rows, stride)
    }

    /// The predictions on rows of values (`RowBuckets::from_values`'s columns as `ColumnData`, in the feature
    /// space's column order), bucketized on the device (`rowBuckets`) in the command buffer that predicts them; the
    /// host only copies the values and maps each categorical column's names (not its rows) to buckets.
    pub fn predict_values(&self, b: &DeviceBucketizer, columns: &[ColumnData]) -> Result<Vec<[f32; 2]>, String> {
        Ok(self.predict_values_timed(b, columns)?.0)
    }

    /// `predict_values`, with the GPU's seconds (bucketizing and predicting).
    pub fn predict_values_timed(&self, b: &DeviceBucketizer, columns: &[ColumnData]) -> Result<(Vec<[f32; 2]>, f64), String> {
        use ComputeKernelArgument::{Buffer as B, Int};
        if columns.len() != self.num_features || b.columns.len() != self.num_features {
            return Err(format!("{} columns and a bucketizer of {} for a forest over {}", columns.len(), b.columns.len(), self.num_features));
        }
        if let Some(i) = (0..self.num_features).find(|&i| b.columns[i].bits != self.feature_bits[i]) {
            return Err(format!("column {i} has {}-bit buckets, the forest's {}", b.columns[i].bits, self.feature_bits[i]));
        }
        let num_rows = columns.first().map_or(0, |c| c.len());
        if columns.iter().any(|c| c.len() != num_rows) {
            return Err("the columns differ in length".into());
        }
        if num_rows == 0 {
            return Ok((Vec::new(), 0.0));
        }
        let shared = crate::gpu::shared::get().ok_or("no Metal device")?;
        let device = &shared.device;
        let mut scratch = self.scratch.lock().map_err(|_| "a prediction panicked")?;
        let words: Vec<usize> = b.columns.iter().map(|c| crate::bucket::WritableBucketList::words_required(num_rows, (1u32 << c.bits) - 1)).collect();
        let stride = words.iter().copied().max().unwrap_or(0) + 1;
        let (mut num_numbers, mut num_codes, mut num_table) = (0usize, 0usize, 0usize);
        for c in columns {
            match c {
                ColumnData::Numeric(v) => num_numbers += v.len(),
                ColumnData::Categorical { codes, names } => {
                    num_codes += codes.len();
                    num_table += names.len() + 1;
                }
            }
        }
        let d_columns = kept::<u32>(device, &mut scratch.columns, columns.len() * ROW_BUCKET_WORDS);
        let d_numbers = kept::<u64>(device, &mut scratch.numbers, num_numbers.max(1));
        let d_codes = kept::<u32>(device, &mut scratch.codes, num_codes.max(1));
        let d_tables = kept::<u32>(device, &mut scratch.tables, num_table.max(1));
        let view = |buf: &metal::Buffer, n: usize| unsafe { std::slice::from_raw_parts_mut(buf.contents().cast::<u32>(), n) };
        let desc = view(&d_columns, columns.len() * ROW_BUCKET_WORDS);
        let numbers = unsafe { std::slice::from_raw_parts_mut(d_numbers.contents().cast::<f64>(), num_numbers) };
        let codes_out = view(&d_codes, num_codes);
        let tables = view(&d_tables, num_table);
        let (mut at_number, mut at_code, mut at_table) = (0usize, 0usize, 0usize);
        for (i, (c, bc)) in columns.iter().zip(&b.columns).enumerate() {
            let d = &mut desc[i * ROW_BUCKET_WORDS..(i + 1) * ROW_BUCKET_WORDS];
            d[1] = bc.bits;
            d[7] = words[i] as u32;
            match c {
                ColumnData::Numeric(v) => {
                    numbers[at_number..at_number + v.len()].copy_from_slice(v);
                    d[0] = 0;
                    d[2] = at_number as u32;
                    d[3] = bc.num_splits;
                    d[4] = bc.split_start;
                    d[5] = bc.numeric_offset;
                    d[6] = bc.null_bucket;
                    at_number += v.len();
                }
                ColumnData::Categorical { codes, names } => {
                    codes_out[at_code..at_code + codes.len()].copy_from_slice(codes);
                    let t = &mut tables[at_table..at_table + names.len() + 1];
                    for (e, name) in t.iter_mut().zip(names) {
                        *e = bc.strings.get(name.as_str()).copied().unwrap_or(bc.unseen);
                    }
                    t[names.len()] = bc.null_bucket;
                    d[0] = 1;
                    d[2] = at_code as u32;
                    d[3] = names.len() as u32;
                    d[4] = at_table as u32;
                    d[5] = 0;
                    d[6] = bc.null_bucket;
                    at_code += codes.len();
                    at_table += names.len() + 1;
                }
            }
        }
        let d_data = kept::<u32>(device, &mut scratch.data, stride * columns.len() + 1);
        let k = kernel("rowBuckets")?;
        let buffers: [(&str, &metal::BufferRef); 6] =
            [("columns", &d_columns), ("numbers", &d_numbers), ("codes", &d_codes), ("splitKeys", &b.split_keys), ("codeTables", &d_tables), ("bucketDataOut", &d_data)];
        let key = (num_rows, stride);
        if !scratch.row_plans.contains_key(&key) {
            let mut named = vec![("numRows", Int(num_rows as i64)), ("numColumns", Int(columns.len() as i64)), ("columnStride", Int(stride as i64))];
            named.extend(buffers.iter().map(|&(n, b)| (n, B(b))));
            if scratch.row_plans.len() >= 64 {
                scratch.row_plans.clear();
            }
            scratch.row_plans.insert(key, k.plan(&named, &[(columns.len() * stride) as u32])?);
        }
        {
            let Scratch { queue, row_plans: plans, .. } = &mut *scratch;
            let queue = queue.get_or_insert_with(|| MetalComputeQueue::new(&shared.queue));
            let mut ctx = queue.new_bind_context(k.function);
            k.launch_plan(&plans[&key], &buffers, &mut ctx)?;
        }
        self.launch(&mut scratch, &d_data, num_rows, stride)
    }

    /// Predicts the rows whose bucket columns are (or will be, earlier in the queue's command buffer) in `d_data`
    /// at `stride` words a column, and commits the command buffer.
    fn launch(&self, scratch: &mut Scratch, d_data: &metal::BufferRef, num_rows: usize, stride: usize) -> Result<(Vec<[f32; 2]>, f64), String> {
        use ComputeKernelArgument::{Buffer as B, Int};
        let shared = crate::gpu::shared::get().ok_or("no Metal device")?;
        let device = &shared.device;
        let items = num_rows * self.num_forests as usize;
        let d_out = kept::<f32>(device, &mut scratch.out, items * 2);
        let k = kernel("predictForest")?;
        let lanes = lanes_per_row(items, self.max_trees as usize);
        let buffers: [(&str, &metal::BufferRef); 8] = [
            ("bucketData", d_data),
            ("nodes", &self.nodes),
            ("subsets", &self.subsets),
            ("forestTreeStart", &self.forest_tree_start),
            ("treeRoots", &self.roots),
            ("treeWeights", &self.weights),
            ("leafPreds", &self.leaves),
            ("predictionsOut", &d_out),
        ];
        let key = (items, lanes, stride);
        if !scratch.plans.contains_key(&key) {
            let mut named = vec![
                ("numItems", Int(items as i64)),
                ("numForests", Int(self.num_forests as i64)),
                ("maxDepth", Int(self.max_depth as i64 + 1)),
                ("lanesPerRow", Int(lanes as i64)),
                ("columnStride", Int(stride as i64)),
            ];
            named.extend(buffers.iter().map(|&(n, b)| (n, B(b))));
            if scratch.plans.len() >= 64 {
                scratch.plans.clear();
            }
            scratch.plans.insert(key, k.plan(&named, &[items as u32])?);
        }
        let Scratch { queue, plans, .. } = scratch;
        let queue = queue.get_or_insert_with(|| MetalComputeQueue::new(&shared.queue));
        let mut ctx = queue.new_bind_context(k.function);
        k.launch_plan(&plans[&key], &buffers, &mut ctx)?;
        drop(ctx);
        let event = queue.flush();
        event.await_()?;
        let out = unsafe { std::slice::from_raw_parts(d_out.contents().cast::<[f32; 2]>(), items) };
        if out.iter().any(|v| v[0].is_nan()) {
            return Err("a walk did not reach a leaf".into());
        }
        Ok((out.to_vec(), event.gpu_seconds()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset_feature_space::FeatureType;
    use crate::randomforest_procedure::{train_forest, train_regression_forest, training_data, RandomForestProcedureConfig};
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
        let mut flats = Vec::new();
        for subsets in [false, true] {
        let config = RandomForestProcedureConfig { feature_vector_samplings: 3, feature_samplings: 4, max_depth: 10, feature_sampling_prop: 0.7, categorical_subsets: subsets, ..Default::default() };
        let forest = train_forest(&all, &config).unwrap();
        let flat = FlatForest::new(&forest).unwrap();
        flats.push(flat.clone());
        assert!(flat.max_depth >= 3, "{}", flat.max_depth);
        assert_eq!(!flat.subsets.is_empty(), subsets, "categorical subset splits to predict with");

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
            // Small batches take many lanes a row (up to one a tree): the same predictions.
            let resident = DeviceForest::new(&flat).unwrap();
            for b in [1usize, 2, 5, 33, 300] {
                assert!(lanes_per_row(b, flat.roots.len()) > 1 || b == 300);
                let few: Vec<Vec<CellValue>> = c.iter().map(|col| col[..b].to_vec()).collect();
                let p = resident.predict(&RowBuckets::from_values(&fs, &few).unwrap()).unwrap();
                for r in 0..b {
                    assert!((p[r][1] - host[r][1]).abs() < 1e-5, "{what}, batch {b}, row {r}: {:?} {:?}", p[r], host[r]);
                }
            }
            let right = host.iter().zip(l).filter(|(p, l)| (p[1] > 0.5) == (**l != 0.0)).count() as f64 / host.len() as f64;
            assert!(right > 0.75, "{what}: {right} right");
            eprintln!("{what}, subsets {subsets}: {right:.3} right");
        }
        }
        // Both forests (one with subset splits) in one dispatch: each row's predictions are each forest's alone, and
        // the kept buffers serve batches that shrink and grow.
        let both = DeviceForest::new_many(&[&flats[0], &flats[1]]).unwrap();
        let alone: Vec<DeviceForest> = flats.iter().map(|f| DeviceForest::new(f).unwrap()).collect();
        for b in [3000usize, 7, 3000, 1] {
            let few: Vec<Vec<CellValue>> = cols.iter().map(|col| col[..b].to_vec()).collect();
            let rows = RowBuckets::from_values(&fs, &few).unwrap();
            let p = both.predict(&rows).unwrap();
            assert_eq!(p.len(), 2 * b);
            for (f, a) in alone.iter().enumerate() {
                let q = a.predict(&rows).unwrap();
                for r in 0..b {
                    // (Within rounding: more items give a row fewer lanes, so its trees sum in another order.)
                    assert!((p[2 * r + f][1] - q[r][1]).abs() < 1e-6, "batch {b}, row {r}, forest {f}: {:?} {:?}", p[2 * r + f], q[r]);
                }
            }
        }
        let config = RandomForestProcedureConfig { feature_vector_samplings: 3, feature_samplings: 4, max_depth: 10, feature_sampling_prop: 0.7, ..Default::default() };

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
        let (w, c) = (std::time::Instant::now(), crate::process_cpu());
        let r = f();
        (r, w.elapsed().as_secs_f64(), crate::process_cpu() - c)
    }

    /// `timed`, with the GPU seconds of the forest kernels' command buffers the run awaited.
    fn timed_gpu<T>(f: impl FnOnce() -> T) -> (T, f64, f64, f64) {
        let g = crate::compute_kernel_metal::gpu_seconds_awaited();
        let (r, w, c) = timed(f);
        (r, w, c, crate::compute_kernel_metal::gpu_seconds_awaited() - g)
    }

    fn train_bench(what: &str, cols: &[Vec<CellValue>], labels: &[f32], config: &RandomForestProcedureConfig) -> (Arc<DatasetFeatureSpace>, Forest) {
        let names = ["lane", "forms", "launches", "units", "cold", "load", "threads"];
        let named: Vec<(String, Vec<CellValue>)> = names.iter().zip(cols).map(|(n, c)| (n.to_string(), c.clone())).collect();
        let (fs, w0, c0) = timed(|| Arc::new(DatasetFeatureSpace::from_columns(&named, FeatureType::Boolean).unwrap()));
        let typed: Vec<(String, crate::dataset_feature_space::ColumnData)> = named.iter().map(|(n, v)| (n.clone(), crate::dataset_feature_space::ColumnData::from_values(n, v).unwrap())).collect();
        let _ = DatasetFeatureSpace::from_columns_device(&typed[..1], FeatureType::Boolean).unwrap();
        let (fsd, wd, cd) = timed(|| DatasetFeatureSpace::from_columns_device(&typed, FeatureType::Boolean).unwrap());
        for i in 0..names.len() {
            assert_eq!(format!("{:?}", fs.column(i).unwrap().bucket_descriptions), format!("{:?}", fsd.column(i).unwrap().bucket_descriptions));
        }
        eprintln!("feature space {what}, {} rows: host {:.0} ms ({:.0} ms CPU), device {:.0} ms ({:.0} ms CPU)", labels.len(), w0 * 1e3, c0 * 1e3, wd * 1e3, cd * 1e3);
        let (all, w1, c1) = timed(|| training_data(fs.clone(), labels, None, None).unwrap());
        let (forest, w2, c2) = timed(|| train_forest(&all, config).unwrap());
        for (i, t) in forest.trees.iter().enumerate() {
            if let Err(e) = FlatForest::new(&Forest { fs: forest.fs.clone(), trees: vec![t.clone()], weights: vec![1.0], target_scale: None }) {
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

    /// One data set of the comparison with other libraries: rows to train on and rows held out, each with a label
    /// (classification) and a target (regression).
    struct Suite {
        name: String,
        train: Vec<Job>,
        test: Vec<Job>,
        train_labels: Vec<f32>,
        test_labels: Vec<f32>,
        train_targets: Vec<f32>,
        test_targets: Vec<f32>,
    }

    const NAMES: [&str; 7] = ["lane", "forms", "launches", "units", "cold", "load", "threads"];

    fn named(cols: Vec<Vec<CellValue>>) -> Vec<(String, Vec<CellValue>)> {
        NAMES.iter().zip(cols).map(|(n, c)| (n.to_string(), c)).collect()
    }

    fn regression_scores(p: &[f64], t: &[f32]) -> (f64, f64) {
        let mean = t.iter().map(|&t| t as f64).sum::<f64>() / t.len() as f64;
        let se: f64 = p.iter().zip(t).map(|(p, &t)| (p - t as f64).powi(2)).sum();
        let st: f64 = t.iter().map(|&t| (t as f64 - mean).powi(2)).sum();
        ((se / t.len() as f64).sqrt(), 1.0 - se / st)
    }

    /// `x` solved from `a x = b` (n x n, row major), by elimination with partial pivoting.
    fn solve(mut a: Vec<f64>, mut b: Vec<f64>) -> Vec<f64> {
        let n = b.len();
        for c in 0..n {
            let p = (c..n).max_by(|&i, &j| a[i * n + c].abs().total_cmp(&a[j * n + c].abs())).unwrap();
            for k in 0..n { a.swap(c * n + k, p * n + k); }
            b.swap(c, p);
            let d = a[c * n + c];
            if d.abs() < 1e-300 { continue; }
            for r in 0..n {
                if r != c {
                    let f = a[r * n + c] / d;
                    for k in 0..n { a[r * n + k] -= f * a[c * n + k]; }
                    b[r] -= f * b[c];
                }
            }
        }
        (0..n).map(|i| if a[i * n + i].abs() < 1e-300 { 0.0 } else { b[i] / a[i * n + i] }).collect()
    }

    /// A probability's log odds, the probability kept off 0 and 1 (a leaf of one label's rows gives either).
    fn log_odds(p: f32) -> f64 {
        let p = (p as f64).clamp(1e-4, 1.0 - 1e-4);
        (p / (1.0 - p)).ln()
    }

    /// The logistic curve `1 / (1 + exp(-(a x + b)))` of least log loss over `(x, label)` (Platt 1999): the labels
    /// smoothed to Platt's targets, `(n+ + 1) / (n+ + 2)` and `1 / (n- + 2)`, so that separable data has a finite
    /// fit; Newton's method, each step halved until the loss falls.
    fn platt(x: &[f64], labels: &[f32]) -> (f64, f64) {
        let pos = labels.iter().filter(|&&l| l != 0.0).count() as f64;
        let neg = labels.len() as f64 - pos;
        let (hi, lo) = ((pos + 1.0) / (pos + 2.0), 1.0 / (neg + 2.0));
        let y: Vec<f64> = labels.iter().map(|&l| if l != 0.0 { hi } else { lo }).collect();
        let loss = |a: f64, b: f64| -> f64 {
            x.iter().zip(&y).map(|(&x, &y)| {
                let z = a * x + b;
                // log(1 + e^z) - y z, stably
                (if z > 0.0 { z + (-z).exp().ln_1p() } else { z.exp().ln_1p() }) - y * z
            }).sum()
        };
        let (mut a, mut b) = (1.0, 0.0);
        let mut current = loss(a, b);
        for _ in 0..100 {
            let (mut ga, mut gb, mut haa, mut hab, mut hbb) = (0.0, 0.0, 1e-12, 0.0, 1e-12);
            for (&x, &y) in x.iter().zip(&y) {
                let q = 1.0 / (1.0 + (-(a * x + b)).exp());
                let (g, h) = (q - y, q * (1.0 - q));
                ga += g * x;
                gb += g;
                haa += h * x * x;
                hab += h * x;
                hbb += h;
            }
            let det = haa * hbb - hab * hab;
            let (da, db) = ((hbb * ga - hab * gb) / det, (haa * gb - hab * ga) / det);
            let mut step = 1.0;
            while step > 1e-8 && loss(a - step * da, b - step * db) > current {
                step /= 2.0;
            }
            if step <= 1e-8 {
                break;
            }
            a -= step * da;
            b -= step * db;
            let next = loss(a, b);
            let done = current - next < 1e-12 * current.abs().max(1.0);
            current = next;
            if done {
                break;
            }
        }
        (a, b)
    }

    /// `cols` cycled to `n` rows.
    fn cycled(cols: &[Vec<CellValue>], n: usize) -> Vec<Vec<CellValue>> {
        cols.iter().map(|c| (0..n).map(|i| c[i % c.len()].clone()).collect()).collect()
    }

    /// Batch prediction on resident forests (one dispatch and wait each): per batch size, the call's wall and CPU
    /// and the GPU's time, the mean of several calls.
    /// Batch prediction at each size, three ways: the values bucketized on the host (`RowBuckets::from_values`,
    /// timed alone), the forest on those buckets (`predict`), and values to predictions on the device
    /// (`predict_values`: bucketizing in the same command buffer). The values come as typed columns
    /// (`ColumnData`), as a caller holds them; turning cells into them is not timed.
    fn batch_timings(what: &str, fs: &DatasetFeatureSpace, forests: &DeviceForest, test_cols: &[Vec<CellValue>]) {
        let bucketizer = DeviceBucketizer::new(fs).unwrap();
        for b in [1usize, 16, 256, 4096, 65536] {
            let cells = cycled(test_cols, b);
            let typed = typed_columns(&cells);
            let reps = if b <= 256 { 20 } else { 3 };
            let r = reps as f64;
            let (rows, hw, hc) = timed(|| {
                let mut rows = None;
                for _ in 0..reps {
                    rows = Some(RowBuckets::from_values(fs, &cells).unwrap());
                }
                rows.unwrap()
            });
            let (gpu, w, c) = timed(|| (0..reps).map(|_| forests.predict_timed(&rows).unwrap().1).sum::<f64>());
            let (dgpu, dw, dc) = timed(|| (0..reps).map(|_| forests.predict_values_timed(&bucketizer, &typed).unwrap().1).sum::<f64>());
            eprintln!("BATCH {what} batch {b:6}: host bucketize {:8.1} us wall {:8.1} us CPU; predict buckets {:8.1} us wall {:8.1} us CPU {:8.1} us GPU; \
                device bucketize+predict {:8.1} us wall {:8.1} us CPU {:8.1} us GPU a call; {:8.1} ns/row wall",
                hw / r * 1e6, hc / r * 1e6, w / r * 1e6, c / r * 1e6, gpu / r * 1e6, dw / r * 1e6, dc / r * 1e6, dgpu / r * 1e6, dw / r / b as f64 * 1e9);
        }
    }

    /// Cells as the typed columns a caller holds (`ColumnData`).
    fn typed_columns(cols: &[Vec<CellValue>]) -> Vec<ColumnData> {
        cols.iter().enumerate().map(|(i, c)| ColumnData::from_values(&format!("c{i}"), c).unwrap()).collect()
    }

    /// One data set: exported for the other libraries (`BENCH_EXPORT`), then this forest's runs.
    /// - classification and regression, upstream's categorical split and CART's, depth 20 and 8;
    /// - the K-bucket regressor for each K in `BENCH_K`: K - 1 forests, forest k predicting "the target is above
    ///   the training targets' k/K quantile", trained on 80% of the training rows; the value from those
    ///   probabilities by the bucket medians (expected value) and by least squares fitted on the other 20%;
    /// - batch prediction for the regression and K-bucket forests.
    fn run_suite(s: &Suite) {
        if let Ok(dir) = std::env::var("BENCH_EXPORT") {
            for (part, jobs, labels, targets) in [("train", &s.train, &s.train_labels, &s.train_targets), ("test", &s.test, &s.test_labels, &s.test_targets)] {
                let mut out = String::from("lane,forms,launches,units,cold,load,threads,label,target\n");
                for ((j, l), t) in jobs.iter().zip(labels.iter()).zip(targets.iter()) {
                    out += &format!("{},{},{},{},{},{},{},{},{}\n", j.lane, j.forms.replace(',', ";"), j.launches, j.units, j.cold, j.load, j.threads, l, t);
                }
                std::fs::write(format!("{dir}/{}_{part}.csv", s.name), out).unwrap();
            }
        }
        let only = std::env::var("BENCH_ONLY").unwrap_or_default();
        let want = |w: &str| only.is_empty() || only.split(',').any(|o| o == w);
        let (train_cols, test_cols) = (named(columns_of(&s.train)), columns_of(&s.test));
        let fsp: f32 = std::env::var("BENCH_FSP").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
        let fs_class = Arc::new(DatasetFeatureSpace::from_columns(&train_cols, FeatureType::Boolean).unwrap());
        let fs_real = Arc::new(DatasetFeatureSpace::from_columns(&train_cols, FeatureType::Real).unwrap());
        // Warm the kernels.
        train_forest(&training_data(fs_class.clone(), &s.train_labels, None, None).unwrap(), &RandomForestProcedureConfig { feature_vector_samplings: 1, feature_samplings: 2, ..Default::default() }).unwrap();
        let nt = s.test.len();
        // The rows a calibrated model trains on (80% of the training rows) and calibrates on (the other 20%).
        let fit: Vec<usize> = (0..s.train.len()).filter(|&i| h(i, 13) % 5 != 0).collect();
        let cal: Vec<usize> = (0..s.train.len()).filter(|&i| h(i, 13) % 5 == 0).collect();
        let pick = |cols: &[(String, Vec<CellValue>)], rows: &[usize]| -> Vec<Vec<CellValue>> { cols.iter().map(|(_, c)| rows.iter().map(|&i| c[i].clone()).collect()).collect() };
        let fit_cols = named(pick(&train_cols, &fit));
        let cal_cols: Vec<Vec<CellValue>> = pick(&train_cols, &cal);
        let fs_fit = Arc::new(DatasetFeatureSpace::from_columns(&fit_cols, FeatureType::Boolean).unwrap());
        // BENCH_MIN_LEAF: the fewest rows a leaf may have; BENCH_PER_TREE: a row sample per tree (100 bags of one
        // tree) rather than upstream's 5 bags of 20.
        let min_leaf: u32 = std::env::var("BENCH_MIN_LEAF").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
        let per_tree = std::env::var("BENCH_PER_TREE").is_ok();
        let depths: Vec<i32> = std::env::var("BENCH_DEPTHS").ok().map(|v| v.split(',').map(|x| x.parse().unwrap()).collect()).unwrap_or(vec![20, 8]);
        let tag = format!("{}{}", if min_leaf > 1 { format!("-leaf{min_leaf}") } else { String::new() }, if per_tree { "-pertree" } else { "" });
        for &depth in &depths {
            for subsets in [false, true] {
                let mut config = RandomForestProcedureConfig { max_depth: depth, categorical_subsets: subsets, feature_sampling_prop: fsp, min_leaf_examples: min_leaf, ..Default::default() };
                if per_tree {
                    config.feature_vector_samplings = 100;
                    config.feature_samplings = 1;
                }
                let kind = format!("{}{tag}", if subsets { "cart" } else { "one-vs-rest" });
                if want("classification") {
                    let (forest, w, c, tg) = timed_gpu(|| train_forest(&training_data(fs_class.clone(), &s.train_labels, None, None).unwrap(), &config).unwrap());
                    let flat = FlatForest::new(&forest).unwrap();
                    let rows = RowBuckets::from_values(&fs_class, &test_cols).unwrap();
                    let resident = DeviceForest::new(&flat).unwrap();
                    let ((p, gpu), pw, pc) = timed(|| resident.predict_timed(&rows).unwrap());
                    let ll = p.iter().zip(&s.test_labels).map(|(p, &l)| -((if l != 0.0 { p[1] } else { p[0] }).clamp(1e-6, 1.0) as f64).ln()).sum::<f64>() / p.len() as f64;
                    eprintln!("RESULT {} rerail-{kind} classification depth {depth}: train {:.0} ms wall {:.0} ms CPU {:.0} ms GPU; {} nodes; predict {nt} {:.1} ms wall {:.1} ms CPU {:.2} ms GPU; accuracy {:.4} logloss {:.4}",
                        s.name, w * 1e3, c * 1e3, tg * 1e3, flat.nodes.len(), pw * 1e3, pc * 1e3, gpu * 1e3, right(&p, &s.test_labels), ll);
                }
                if want("calibrated") {
                    // The forest on 80% of the training rows; a logistic curve over its log odds fitted on the other
                    // 20% (Platt's calibration, mldb's GLZ probabilizer with a logit link).
                    let fit_labels: Vec<f32> = fit.iter().map(|&i| s.train_labels[i]).collect();
                    let cal_labels: Vec<f32> = cal.iter().map(|&i| s.train_labels[i]).collect();
                    let ((resident, a, b), w, c, tg) = timed_gpu(|| {
                        let forest = train_forest(&training_data(fs_fit.clone(), &fit_labels, None, None).unwrap(), &config).unwrap();
                        let resident = DeviceForest::new(&FlatForest::new(&forest).unwrap()).unwrap();
                        let p = resident.predict(&RowBuckets::from_values(&fs_fit, &cal_cols).unwrap()).unwrap();
                        let (a, b) = platt(&p.iter().map(|p| log_odds(p[1])).collect::<Vec<_>>(), &cal_labels);
                        (resident, a, b)
                    });
                    let rows = RowBuckets::from_values(&fs_fit, &test_cols).unwrap();
                    let ((p, gpu), pw, pc) = timed(|| {
                        let (p, gpu) = resident.predict_timed(&rows).unwrap();
                        (p.iter().map(|p| { let q = 1.0 / (1.0 + (-(a * log_odds(p[1]) + b)).exp()); [(1.0 - q) as f32, q as f32] }).collect::<Vec<_>>(), gpu)
                    });
                    let ll = p.iter().zip(&s.test_labels).map(|(p, &l)| -((if l != 0.0 { p[1] } else { p[0] }).clamp(1e-6, 1.0) as f64).ln()).sum::<f64>() / p.len() as f64;
                    eprintln!("RESULT {} rerail-{kind}-platt classification depth {depth}: train {:.0} ms wall {:.0} ms CPU {:.0} ms GPU; curve {a:.3} x + {b:.3}; predict {nt} {:.1} ms wall {:.1} ms CPU {:.2} ms GPU; accuracy {:.4} logloss {:.4}",
                        s.name, w * 1e3, c * 1e3, tg * 1e3, pw * 1e3, pc * 1e3, gpu * 1e3, right(&p, &s.test_labels), ll);
                }
                if want("regression") {
                    let (forest, w, c, tg) = timed_gpu(|| train_regression_forest(fs_real.clone(), &s.train_targets, None, None, &config).unwrap());
                    let flat = FlatForest::new(&forest).unwrap();
                    let rows = RowBuckets::from_values(&fs_real, &test_cols).unwrap();
                    let resident = DeviceForest::new(&flat).unwrap();
                    let ((p, gpu), pw, pc) = timed(|| resident.predict_timed(&rows).unwrap());
                    let values: Vec<f64> = p.iter().map(|p| forest.regression_value(*p).unwrap()).collect();
                    let (rmse, r2) = regression_scores(&values, &s.test_targets);
                    eprintln!("RESULT {} rerail-{kind} regression depth {depth}: train {:.0} ms wall {:.0} ms CPU {:.0} ms GPU; {} nodes; predict {nt} {:.1} ms wall {:.1} ms CPU {:.2} ms GPU; rmse {rmse:.4} r2 {r2:.4}",
                        s.name, w * 1e3, c * 1e3, tg * 1e3, flat.nodes.len(), pw * 1e3, pc * 1e3, gpu * 1e3);
                    if depth == 20 && !subsets && want("batch") {
                        batch_timings(&format!("{} regression depth {depth}", s.name), &fs_real, &resident, &test_cols);
                    }
                }
                if !want("kbucket") {
                    continue;
                }
                let ks: Vec<usize> = std::env::var("BENCH_K").ok().map(|v| v.split(',').map(|x| x.parse().unwrap()).collect()).unwrap_or(vec![2, 4, 8, 16]);
                let fs = &fs_fit;
                let mut sorted: Vec<f32> = fit.iter().map(|&i| s.train_targets[i]).collect();
                sorted.sort_by(f32::total_cmp);
                for &k in &ks {
                    let thresholds: Vec<f32> = (1..k).map(|q| sorted[q * sorted.len() / k]).collect();
                    // Each bucket's median target, bucket b holding the targets between thresholds b - 1 and b.
                    let medians: Vec<f64> = (0..k).map(|b| sorted[(2 * b + 1) * sorted.len() / (2 * k)] as f64).collect();
                    // The K - 1 forests, resident together: one dispatch predicts them all.
                    let ((forests, nodes), w, c, tg) = timed_gpu(|| {
                        let mut flats = Vec::new();
                        for &t in &thresholds {
                            let labels: Vec<f32> = fit.iter().map(|&i| (s.train_targets[i] > t) as u8 as f32).collect();
                            let forest = train_forest(&training_data(fs.clone(), &labels, None, None).unwrap(), &config).unwrap();
                            flats.push(FlatForest::new(&forest).unwrap());
                        }
                        let nodes: usize = flats.iter().map(|f| f.nodes.len()).sum();
                        (DeviceForest::new_many(&flats.iter().collect::<Vec<_>>()).unwrap(), nodes)
                    });
                    // The probabilities "above threshold k", per row, for a set of rows (bucketized on the device).
                    let bucketizer = DeviceBucketizer::new(&fs).unwrap();
                    let probs = |cols: &[ColumnData]| -> (Vec<Vec<f64>>, f64) {
                        let (p, gpu) = forests.predict_values_timed(&bucketizer, cols).unwrap();
                        (p.chunks(k - 1).map(|r| r.iter().map(|p| p[1] as f64).collect()).collect(), gpu)
                    };
                    // Least squares over [1, p_1 .. p_{K-1}] on the calibration rows (a little ridge).
                    let (cal_p, _) = probs(&typed_columns(&cal_cols));
                    let test_typed = typed_columns(&test_cols);
                    let n = k;
                    let (mut a, mut b) = (vec![0.0; n * n], vec![0.0; n]);
                    for (p, &i) in cal_p.iter().zip(&cal) {
                        let x: Vec<f64> = std::iter::once(1.0).chain(p.iter().copied()).collect();
                        for r in 0..n {
                            b[r] += x[r] * s.train_targets[i] as f64;
                            for q in 0..n {
                                a[r * n + q] += x[r] * x[q];
                            }
                        }
                    }
                    for r in 1..n {
                        a[r * n + r] += 1e-3;
                    }
                    let coef = solve(a, b);
                    let ((test_p, gpu), pw, pc) = timed(|| probs(&test_typed));
                    let expected: Vec<f64> = test_p.iter().map(|p| medians[0] + p.iter().enumerate().map(|(q, &pq)| pq * (medians[q + 1] - medians[q])).sum::<f64>()).collect();
                    let fitted: Vec<f64> = test_p.iter().map(|p| coef[0] + p.iter().zip(&coef[1..]).map(|(p, c)| p * c).sum::<f64>()).collect();
                    let (rmse_e, r2_e) = regression_scores(&expected, &s.test_targets);
                    let (rmse_l, r2_l) = regression_scores(&fitted, &s.test_targets);
                    eprintln!("RESULT {} rerail-{kind}-k{k} kbucket depth {depth}: train {:.0} ms wall {:.0} ms CPU {:.0} ms GPU; {nodes} nodes; predict {nt} {:.1} ms wall {:.1} ms CPU {:.2} ms GPU; expected rmse {rmse_e:.4} r2 {r2_e:.4}; least squares rmse {rmse_l:.4} r2 {r2_l:.4}",
                        s.name, w * 1e3, c * 1e3, tg * 1e3, pw * 1e3, pc * 1e3, gpu * 1e3);
                    if depth == 20 && !subsets && k == 8 && want("batch") {
                        batch_timings(&format!("{} kbucket k{k} depth {depth}", s.name), &fs, &forests, &test_cols);
                    }
                }
            }
        }
    }

    /// The comparison with other libraries (`BENCH_DATA`: `synthetic`, `real` or both, the default):
    /// - synthetic: the synthetic jobs, `BENCH_SIZES[0]` rows to train on and 20,000 held out; the label "the lane
    ///   beats the CPU", the target ln(wall);
    /// - real: the job samples (`BENCH_SAMPLES`), 70% to train on and 30% held out; the label "slower than its
    ///   price", the target ln(wall).
    #[test]
    #[ignore = "benchmark: run by name with --ignored --nocapture, BENCH_SAMPLES and BENCH_EXPORT set"]
    fn compare_with_libraries() {
        if crate::gpu::shared::get().is_none() {
            return;
        }
        let real = real_jobs();
        assert!(!real.is_empty(), "BENCH_SAMPLES");
        let data = std::env::var("BENCH_DATA").unwrap_or("synthetic,real".into());
        let target = |j: &Job| j.wall.max(1e-9).ln() as f32;
        if data.split(',').any(|d| d == "synthetic") {
            let n: usize = std::env::var("BENCH_SIZES").ok().and_then(|v| v.split(',').next()?.parse().ok()).unwrap_or(20000);
            let (train, train_labels) = synthetic(&real, n, 1);
            let (test, test_labels) = synthetic(&real, 20000, 7);
            let (train_targets, test_targets) = (train.iter().map(target).collect(), test.iter().map(target).collect());
            run_suite(&Suite { name: format!("synthetic{n}"), train, test, train_labels, test_labels, train_targets, test_targets });
        }
        if data.split(',').any(|d| d == "real") {
            let (train, test): (Vec<(usize, &Job)>, Vec<(usize, &Job)>) = real.iter().enumerate().partition(|(i, _)| h(*i, 11) % 10 < 7);
            let train: Vec<Job> = train.into_iter().map(|(_, j)| j.clone()).collect();
            let test: Vec<Job> = test.into_iter().map(|(_, j)| j.clone()).collect();
            let label = |j: &Job| (j.wall > j.estimate) as u8 as f32;
            run_suite(&Suite {
                name: "real".into(),
                train_labels: train.iter().map(label).collect(),
                test_labels: test.iter().map(label).collect(),
                train_targets: train.iter().map(target).collect(),
                test_targets: test.iter().map(target).collect(),
                train,
                test,
            });
        }
    }

    /// Rows of values bucketized on the device give the host's bucket words (`RowBuckets::from_values`) and
    /// predictions, with values training never saw: numbers past the splits, unseen strings, nulls in a column that
    /// had none, and numbers in a categorical column.
    #[test]
    fn device_bucketizing_is_the_host_s() {
        if crate::gpu::shared::get().is_none() {
            return;
        }
        let (cols, labels) = columns(3000, 3);
        let named: Vec<(String, Vec<CellValue>)> = cols.iter().enumerate().map(|(i, c)| (format!("c{i}"), c.clone())).collect();
        let fs = Arc::new(DatasetFeatureSpace::from_columns(&named, FeatureType::Boolean).unwrap());
        let forest = train_forest(&training_data(fs.clone(), &labels, None, None).unwrap(), &RandomForestProcedureConfig::default()).unwrap();
        let f = DeviceForest::new(&FlatForest::new(&forest).unwrap()).unwrap();
        let b = DeviceBucketizer::new(&fs).unwrap();
        let (mut test, _) = columns(777, 9);
        for (i, c) in test.iter_mut().enumerate() {
            for (r, v) in c.iter_mut().enumerate() {
                match (r % 11, i) {
                    (0, _) => *v = CellValue::Empty,
                    (1, _) if matches!(v, CellValue::String(_)) => *v = CellValue::String(format!("never seen {r}")),
                    (2, _) if matches!(v, CellValue::Float(_) | CellValue::Integer(_)) => *v = CellValue::Float(1e300 * if r % 2 == 0 { 1.0 } else { -1.0 }),
                    (3, _) if matches!(v, CellValue::Float(_) | CellValue::Integer(_)) => *v = CellValue::Float(-0.0),
                    _ => {}
                }
            }
        }
        for n in [777usize, 1, 33] {
            let part: Vec<Vec<CellValue>> = test.iter().map(|c| c[..n].to_vec()).collect();
            // A categorical column given numbers: every number is unseen there.
            let mut values: Vec<ColumnData> = part.iter().enumerate().map(|(i, c)| ColumnData::from_values(&format!("c{i}"), c).unwrap()).collect();
            let host = RowBuckets::from_values(&fs, &part).unwrap();
            let want = f.predict(&host).unwrap();
            let got = f.predict_values(&b, &values).unwrap();
            assert_eq!(got, want, "{n} rows");
            let stride = host.columns.iter().map(|c| c.storage.data().len()).max().unwrap() + 1;
            let data = f.scratch.lock().unwrap().data.clone().unwrap().0;
            let data = unsafe { std::slice::from_raw_parts(data.contents().cast::<u32>(), stride * host.columns.len()) };
            for (i, c) in host.columns.iter().enumerate() {
                assert_eq!(&data[i * stride..i * stride + c.storage.data().len()], c.storage.data(), "{n} rows, column {i}");
            }
            if let Some(i) = values.iter().position(|v| matches!(v, ColumnData::Categorical { .. })) {
                values[i] = ColumnData::Numeric((0..n).map(|r| r as f64).collect());
                let mut swapped = part.clone();
                swapped[i] = (0..n).map(|r| CellValue::Float(r as f64)).collect();
                assert_eq!(f.predict_values(&b, &values).unwrap(), f.predict(&RowBuckets::from_values(&fs, &swapped).unwrap()).unwrap());
            }
        }
    }

    /// Where a single-row call's time goes: each phase of `predict_timed`, the mean over many calls (wall and the
    /// process's CPU).
    #[test]
    #[ignore = "benchmark: run by name with --ignored --nocapture"]
    fn single_row_call_phases() {
        use ComputeKernelArgument::{Buffer as B, Int};
        if crate::gpu::shared::get().is_none() {
            return;
        }
        let (cols, labels) = columns(4000, 1);
        let named: Vec<(String, Vec<CellValue>)> = cols.iter().enumerate().map(|(i, c)| (format!("c{i}"), c.clone())).collect();
        let fs = Arc::new(DatasetFeatureSpace::from_columns(&named, FeatureType::Boolean).unwrap());
        let forest = train_forest(&training_data(fs.clone(), &labels, None, None).unwrap(), &RandomForestProcedureConfig::default()).unwrap();
        let f = DeviceForest::new(&FlatForest::new(&forest).unwrap()).unwrap();
        let one: Vec<Vec<CellValue>> = cols.iter().map(|c| c[..1].to_vec()).collect();
        let rows = RowBuckets::from_values(&fs, &one).unwrap();
        for _ in 0..20 {
            f.predict(&rows).unwrap();
        }
        let n = 300;
        let thread_cpu = || { let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 }; unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) }; ts.tv_sec as f64 + ts.tv_nsec as f64 * 1e-9 };
        let wait = std::env::var("WAIT").unwrap_or("wait".into());
        let mut t = [(0.0f64, 0.0f64); 8];
        let mut tt = [0.0f64; 8];
        let names = ["lock + write rows", "queue (command buffer)", "encoder", "grid enqueue", "end encoder", "commit", "wait", "read out"];
        let shared = crate::gpu::shared::get().unwrap();
        for _ in 0..n {
            let mut mark = (std::time::Instant::now(), crate::process_cpu(), thread_cpu());
            let mut phase = |i: usize, t: &mut [(f64, f64); 8]| {
                let now = (std::time::Instant::now(), crate::process_cpu(), thread_cpu());
                t[i].0 += (now.0 - mark.0).as_secs_f64();
                t[i].1 += now.1 - mark.1;
                tt[i] += now.2 - mark.2;
                mark = now;
            };
            let mut scratch = f.scratch.lock().unwrap();
            let stride = rows.columns.iter().map(|c| c.storage.data().len()).max().unwrap_or(0) + 1;
            let words = stride * rows.columns.len() + 1;
            let d_data = kept::<u32>(&shared.device, &mut scratch.data, words);
            let data = unsafe { std::slice::from_raw_parts_mut(d_data.contents().cast::<u32>(), words) };
            for (i, c) in rows.columns.iter().enumerate() {
                data[i * stride..i * stride + c.storage.data().len()].copy_from_slice(c.storage.data());
            }
            let d_out = kept::<f32>(&shared.device, &mut scratch.out, 2);
            phase(0, &mut t);
            let mut queue = MetalComputeQueue::new(&shared.queue);
            phase(1, &mut t);
            let k = kernel("predictForest").unwrap();
            let mut ctx = queue.new_bind_context(k.function);
            phase(2, &mut t);
            k.enqueue(&[
                ("numItems", Int(1)), ("numForests", Int(1)), ("maxDepth", Int(f.max_depth as i64 + 1)),
                ("lanesPerRow", Int(lanes_per_row(1, f.max_trees as usize) as i64)), ("columnStride", Int(stride as i64)),
                ("bucketData", B(&d_data)), ("nodes", B(&f.nodes)), ("subsets", B(&f.subsets)), ("forestTreeStart", B(&f.forest_tree_start)),
                ("treeRoots", B(&f.roots)), ("treeWeights", B(&f.weights)), ("leafPreds", B(&f.leaves)), ("predictionsOut", B(&d_out)),
            ], &[1], &mut ctx).unwrap();
            phase(3, &mut t);
            drop(ctx);
            phase(4, &mut t);
            let event = queue.flush();
            phase(5, &mut t);
            match wait.as_str() {
                // Spinning on the status: the thread's CPU while the GPU works, no wake-up.
                "spin" => while !matches!(event.command_buffer.status(), metal::MTLCommandBufferStatus::Completed | metal::MTLCommandBufferStatus::Error) { std::hint::spin_loop() },
                "yield" => while !matches!(event.command_buffer.status(), metal::MTLCommandBufferStatus::Completed | metal::MTLCommandBufferStatus::Error) { std::thread::yield_now() },
                _ => event.await_().unwrap(),
            }
            phase(6, &mut t);
            let _ = unsafe { std::slice::from_raw_parts(d_out.contents().cast::<[f32; 2]>(), 1) }.to_vec();
            drop(queue);
            phase(7, &mut t);
        }
        for (name, (w, c)) in names.iter().zip(t) {
            let i = names.iter().position(|x| x == name).unwrap();
            eprintln!("PHASE {wait} {name:24} {:7.1} us wall {:7.1} us process CPU {:7.1} us this thread", w / n as f64 * 1e6, c / n as f64 * 1e6, tt[i] / n as f64 * 1e6);
        }
        // The whole call as `predict` makes it (a kept queue, a cached plan).
        let (w0, c0, t0) = (std::time::Instant::now(), crate::process_cpu(), thread_cpu());
        for _ in 0..n {
            f.predict(&rows).unwrap();
        }
        eprintln!("PHASE predict() whole call           {:7.1} us wall {:7.1} us process CPU {:7.1} us this thread", w0.elapsed().as_secs_f64() / n as f64 * 1e6, (crate::process_cpu() - c0) / n as f64 * 1e6, (thread_cpu() - t0) / n as f64 * 1e6);
    }
}
