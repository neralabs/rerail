//! randomforest.h and randomforest.cc, in Rust.
//!
//! Mathieu Marquis Bolduc, 11 March 2016; Jeremy Barnes. Copyright (c) 2016 mldb.ai inc. This file is part of
//! MLDB. Apache-2.0 (see the LICENSE at `accel/src/forest/mldb/`).
//!
//! Optimized random forest algorithm for dense data and binary classification.
//!
//! Here so far: `compareTrees`; `PartitionData` and its recursive training (`train`: the best split over every
//! feature, the rows split, each side trained; `split`, re-indexing when the example numbers are sparse), and the
//! partitioned trainer that runs on the device: `FeatureSamplingTrainerKernel` (the rows decoded on the device, the
//! bucket data moved there once) and `FeatureSamplingTrainerPartition` (the device memory pools; per tree, `init`
//! fills the training info and the root's buckets, `scheduleTraining` enqueues every depth's kernels with no wait
//! between them and copies the results out, and `finish` awaits them and builds the tree from the splits),
//! `trainPartitionedEndToEnd`, `PartitionData::trainPartitioned` and `trainMultipleSamplings`, and
//! `reweightAndCompact` (a bag's rows and buckets).
//!
//! The device is Metal (the shared device); upstream's choice among OpenCL, Metal, the CPU and the comparing `MULTI`
//! runtime is not kept, nor the `DEBUG_RF_KERNELS` checks of each kernel against the host (the grid kernels' test
//! makes them), nor the timings. The device trains every level while the tree's width (its active partitions)
//! fits its partitions and the levels' splits fit its pool; before a level that would not, it stops
//! (`TREE_DEPTH_STATUS_STOPPED`), and the tree continues on the host's partitioned trainer from the partitions the
//! device left, each tree from its own copy of them. Upstream's continuation was behind an early return (the tree
//! came back unfinished), and the device ran every level, dropping the partitions past its width, unread. Training below the root (`depth != 0`) goes to the CPU's partitioned trainer, as
//! upstream. The recursive trainer's sides are trained one after the other, as upstream (`singleThreadOnly`); the
//! trees of `trainMultipleSamplings` in parallel.

use crate::forest::compute_kernel_grid::ComputeKernelArgument::{self, Buffer as B, Float, Int};
use crate::forest::compute_kernel_metal::{alloc_uninitialized_array_sync, transfer_to_device_immutable, transfer_to_host_sync, MetalComputeEvent, MetalComputeQueue};
use crate::forest::dataset_feature_space::DatasetFeatureSpace;
use crate::forest::memory_region::FrozenMemoryRegionT;
use crate::forest::randomforest_kernels_grid::kernel;
use crate::forest::bucket::{BucketList, WritableBucketList};
use crate::forest::memory_region::{MappedSerializer, MemorySerializer};
use crate::forest::randomforest_recursive::{extract_tree, side_of, split_and_recurse_partitioned, test_all};
use crate::forest::randomforest_types::*;
use crate::forest::tree::{Ptr, Tree};
use metal::Buffer;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::sync::Arc;

/// Compares the subtrees at `left` (in `lt`) and `right` (in `rt`): true when they are the same; the differences
/// are written to `stream`.
pub fn compare_trees(lt: &Tree, left: Ptr, rt: &Tree, right: Ptr, index: PartitionIndex, fs: &DatasetFeatureSpace, stream: &mut String) -> bool {
    let depth = index.depth();
    let summary = |t: &Tree, p: Ptr| -> String {
        if !p.is_some() {
            return "null".into();
        }
        let pred = t.pred(p);
        let mut r = format!("ex={} pred={} {}", t.examples(p), pred.first().unwrap_or(&0.0), pred.get(1).unwrap_or(&0.0));
        if let Some(n) = t.node(p) {
            let _ = write!(r, " z={:.12} {} {:?} {} ex {}:{}", n.z, fs.print_feature(n.split.feature()), n.split.op(), n.split.split_val(), t.examples(n.child_false) as i32, t.examples(n.child_true) as i32);
        }
        r
    };
    let do_print = |stream: &mut String| -> bool {
        let _ = writeln!(stream, "difference at depth {depth} index {index}\n  rec:  {}\n  part: {}", summary(lt, left), summary(rt, right));
        false
    };
    let mut different = false;
    if lt.pred(left) != rt.pred(right) {
        let _ = writeln!(stream, "different predictions at depth {depth} index {index}: {:?} vs {:?}", lt.pred(left), rt.pred(right));
        different = true;
    }
    if lt.examples(left) != rt.examples(right) {
        let _ = writeln!(stream, "different examples at depth {depth} index {index}: {} vs {}", lt.examples(left), rt.examples(right));
        different = true;
    }
    match (lt.node(left), rt.node(right)) {
        (Some(l), Some(r)) => {
            if l.split != r.split {
                let _ = writeln!(stream, "different split: {:?} vs {:?}", l.split, r.split);
                different = true;
            }
            if l.z != r.z {
                let _ = writeln!(stream, "different z: {} vs {}", l.z, r.z);
                different = true;
            }
            if !different {
                if !compare_trees(lt, l.child_true, rt, r.child_true, index.left_child(), fs, stream) {
                    let _ = writeln!(stream, "different left");
                    return do_print(stream);
                }
                if !compare_trees(lt, l.child_false, rt, r.child_false, index.right_child(), fs, stream) {
                    let _ = writeln!(stream, "different right");
                    return do_print(stream);
                }
            }
            if different { do_print(stream) } else { true }
        }
        _ if left.is_leaf() && right.is_leaf() => {
            // Already compared.
            if different { do_print(stream) } else { true }
        }
        _ => {
            let _ = writeln!(stream, "different type at depth {depth}");
            do_print(stream)
        }
    }
}

/// Holds the set of data for a partition of a decision tree.
#[derive(Clone, Debug, Default)]
pub struct PartitionData {
    pub fs: Arc<DatasetFeatureSpace>,
    pub rows: Rows,
    pub bucket_memory: FrozenMemoryRegionT<u32>,
    pub features: Vec<Feature>,
}

/// `PartitionData::TrainingScheme`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrainingScheme {
    /// Train by partitioning and doing a single pass through the partitions.
    Partitioned = 1,
    /// Train by recursively descending each split.
    Recursive = 2,
    /// Train both of the above, and compare the two results.
    BothAndCompare = 3,
}

fn bytes_of<T: crate::forest::memory_region::Pod>(v: &[T]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), std::mem::size_of_val(v)) }
}

fn shared() -> Result<&'static crate::gpu::shared::Shared, String> {
    crate::gpu::shared::get().ok_or_else(|| "no Metal device".to_string())
}

/// The trainer over one set of rows and features: what every tree trained from them shares.
pub struct FeatureSamplingTrainerKernel<'a> {
    pub bucket_memory: FrozenMemoryRegionT<u32>,
    pub fs: &'a DatasetFeatureSpace,
    pub max_partition_count: u32,
    pub rows: Rows,
    pub features: &'a [Feature],
    pub num_rows: u32,
    pub nf: u32,
    /// The depth of the trees (`maxDepth` as asked).
    pub tree_max_depth: u32,
    /// Upstream's `maxDepth`: one more than the trees' depth (the levels of a schedule that finishes the tree,
    /// the last only searching splits).
    pub max_depth: u32,
    /// The levels the device splits at most (`device_levels <= tree_max_depth`); a deeper tree, or one the device
    /// stops on (its width), continues on the host.
    pub device_levels: u32,
    pub expanded_row_data: Buffer,
    /// Offset in the buckets memory blob per feature [nf + 1].
    pub bucket_memory_offsets: Vec<u32>,
    /// How many bits per bucket [nf].
    pub bucket_entry_bits: Vec<u32>,
    /// For each feature: is it ordinal (1) vs categorical (0) [nf].
    pub feature_is_ordinal: Vec<u32>,
    pub bucket_data: Buffer,
    pub row_data: Buffer,
    pub weight_data: Buffer,
    pub device_bucket_entry_bits: Buffer,
    pub device_bucket_data_offsets: Buffer,
    pub device_feature_is_ordinal: Buffer,
}

impl<'a> FeatureSamplingTrainerKernel<'a> {
    /// The most levels the device splits (a partition's index, 2^depth plus its path, is 32 bits). The device's
    /// bound is the tree's width, not its depth: `assignPartitionNumbers` reuses the numbers of partitions that stop,
    /// so it needs as many partitions as are active at a level (`MAX_DEVICE_WIDTH`, and the bucket budget), and
    /// records the partitions that stop at a level in its threadgroup list of `INACTIVE_PARTITIONS_LENGTH` (16300),
    /// which a width under that never overflows. Before a level past either bound the device stops, and the host
    /// continues. Deep trees are narrow: 200,000 rows of noisy data to depth 20 are at most 7630 wide (depth 17).
    pub const MAX_DEVICE_LEVELS: u32 = 30;
    /// The most partitions active at a level on the device (under `INACTIVE_PARTITIONS_LENGTH`).
    pub const MAX_DEVICE_WIDTH: u32 = 16256;

    pub fn init(debug_name: &str, max_depth: i32, rows: &Rows, features: &'a [Feature], bucket_memory: FrozenMemoryRegionT<u32>, fs: &'a DatasetFeatureSpace) -> Result<Self, String> {
        Self::init_with_device_levels(debug_name, max_depth, rows, features, bucket_memory, fs, Self::MAX_DEVICE_LEVELS)
    }

    /// `init`, the device splitting at most `max_device_levels` levels (at most `MAX_DEVICE_LEVELS`).
    pub fn init_with_device_levels(_debug_name: &str, max_depth: i32, rows: &Rows, features: &'a [Feature], bucket_memory: FrozenMemoryRegionT<u32>, fs: &'a DatasetFeatureSpace, max_device_levels: u32) -> Result<Self, String> {
        let shared = shared()?;
        let device = &shared.device;
        let num_rows = rows.row_count() as u32;
        let nf = features.len() as u32;
        if max_depth < 1 {
            return Err(format!("max depth {max_depth}"));
        }
        // The kernels read each row's buckets at its row number.
        if rows.example_num_bits != 0 && (0..rows.row_count()).any(|i| rows.get_example_num(i) != i as u32) {
            return Err("the rows' example numbers are not their row numbers".into());
        }
        let device_levels = (max_depth as u32).min(max_device_levels.clamp(1, Self::MAX_DEVICE_LEVELS));
        let max_partition_count = (1u32 << device_levels).min(Self::MAX_DEVICE_WIDTH);

        let (mut bucket_entry_bits, mut feature_is_ordinal, mut bucket_memory_offsets) = (Vec::new(), Vec::new(), Vec::new());
        let mut last_bucket_data_offset = 0u32;
        let base = bucket_memory.data().as_ptr() as usize;
        for f in features {
            let buckets = &f.buckets;
            bucket_entry_bits.push(buckets.entry_bits as u32);
            feature_is_ordinal.push(f.ordinal as u32);
            if !f.active {
                // Never read: its buckets need not be in the bucket memory (a bag carries none).
                bucket_memory_offsets.push(0);
                continue;
            }
            let p = buckets.storage.data().as_ptr() as usize;
            if p < base || p > base + bucket_memory.length() * 4 {
                return Err("a feature's buckets are not in the bucket memory".into());
            }
            let offset = ((p - base) / 4) as u32;
            bucket_memory_offsets.push(offset);
            last_bucket_data_offset = offset + bucket_memory.length() as u32;
        }
        bucket_memory_offsets.push(last_bucket_data_offset);

        let row_data = transfer_to_device_immutable(device, rows.row_data.data());
        let weight_data = transfer_to_device_immutable(device, rows.weight_encoder.weight_format_table.data());
        let bucket_data = transfer_to_device_immutable(device, bucket_memory.data());
        let device_feature_is_ordinal = transfer_to_device_immutable(device, &feature_is_ordinal);
        let expanded_row_data = alloc_uninitialized_array_sync::<f32>(device, num_rows as usize);

        let mut queue = MetalComputeQueue::new(&shared.queue);
        run(&queue, "decodeRows", &[
            ("rowData", B(&row_data)),
            ("rowDataLength", Int(rows.row_data.length() as i64)),
            ("weightBits", Int(rows.weight_encoder.weight_bits as i64)),
            ("exampleNumBits", Int(rows.example_num_bits as i64)),
            ("numRows", Int(num_rows as i64)),
            ("weightFormat", Int(rows.weight_encoder.weight_format as i64)),
            ("weightMultiplier", Float(rows.weight_encoder.weight_multiplier as f64)),
            ("weightData", B(&weight_data)),
            ("decodedRowsOut", B(&expanded_row_data)),
        ], &[num_rows])?;
        let device_bucket_entry_bits = transfer_to_device_immutable(device, &bucket_entry_bits);
        let device_bucket_data_offsets = transfer_to_device_immutable(device, &bucket_memory_offsets);
        queue.flush().await_()?;

        Ok(FeatureSamplingTrainerKernel {
            bucket_memory,
            fs,
            max_partition_count,
            rows: rows.clone(),
            features,
            num_rows,
            nf,
            tree_max_depth: max_depth as u32,
            max_depth: max_depth as u32 + 1,
            device_levels,
            expanded_row_data,
            bucket_memory_offsets,
            bucket_entry_bits,
            feature_is_ordinal,
            bucket_data,
            row_data,
            weight_data,
            device_bucket_entry_bits,
            device_bucket_data_offsets,
            device_feature_is_ordinal,
        })
    }

    /// Trains one tree per set of active features, all scheduled before any is awaited.
    pub fn train_partitioned(&self, feature_vector_sampling: i32, debug_name_in: &str, all_active_features_in: &[Vec<i32>]) -> Result<Vec<Tree>, String> {
        Ok(self.train_partitioned_with_depths(feature_vector_sampling, debug_name_in, all_active_features_in)?.into_iter().map(|t| t.0).collect())
    }

    /// `train_partitioned`, with the depth the device reached in each tree.
    pub fn train_partitioned_with_depths(&self, feature_vector_sampling: i32, debug_name_in: &str, all_active_features_in: &[Vec<i32>]) -> Result<Vec<(Tree, u32)>, String> {
        let debug_name = format!("{debug_name_in}-{feature_vector_sampling}");
        let mut partition = FeatureSamplingTrainerPartition::allocate(self, all_active_features_in, &debug_name)?;
        let mut outputs = Vec::new();
        for (i, af) in all_active_features_in.iter().enumerate() {
            let bag_debug_name = format!("{debug_name}-{i}");
            let data = partition.init(i as i32, feature_vector_sampling, &bag_debug_name, af).map_err(|e| format!("Error training {debug_name} partition {i}: {e}"))?;
            let scheduled = partition.schedule_training(&bag_debug_name, &data).map_err(|e| format!("Error training {debug_name} partition {i}: {e}"))?;
            outputs.push((scheduled, data));
        }
        let mut result = Vec::new();
        for (i, (scheduled, data)) in outputs.into_iter().enumerate() {
            scheduled.event.await_()?;
            result.push(partition.finish_with_depth(i as i32, &format!("{debug_name}-{i}"), &data, &scheduled).map_err(|e| format!("Error training {debug_name} partition {i}: {e}"))?);
        }
        Ok(result)
    }
}

/// Enqueues grid kernel `name` on the queue.
fn run(queue: &MetalComputeQueue, name: &str, args: &[(&str, ComputeKernelArgument)], grid: &[u32]) -> Result<(), String> {
    let k = kernel(name)?;
    let mut ctx = queue.new_bind_context(k.function);
    k.enqueue(args, grid, &mut ctx)
}

/// What one tree's training is set up with.
#[derive(Clone, Debug, Default)]
pub struct PartitionControlData {
    pub active_features_in: Vec<i32>,
    pub bucket_numbers: Vec<u32>,
    pub feature_is_active: Vec<u32>,
    pub active_feature_list: Vec<u32>,
    pub tree_training_info: TreeTrainingInfo,
    pub tree_depth_info0: TreeDepthInfo,
    pub num_active_features: u32,
    pub num_active_buckets: u32,
}

/// The device memory one tree's training works in, reused tree after tree (each tree's work is queued after the
/// last's).
pub struct FeatureSamplingTrainerPartition<'o, 'a> {
    pub owner: &'o FeatureSamplingTrainerKernel<'a>,
    /// The most partitions active at a level (`device_width`).
    pub width: u32,
    /// The length of the pool of every level's splits (`device_splits_pool`).
    pub splits_pool: usize,
    /// Whether the device can stop before its levels (on width or on the splits pool).
    pub can_stop: bool,
    pub depth_queue: MetalComputeQueue,
    pub device_tree_training_info: Buffer,
    pub device_bucket_numbers: Buffer,
    pub device_feature_is_active: Buffer,
    pub device_active_feature_list: Buffer,
    pub device_w_all_pool: Buffer,
    pub device_partition_bucket_pool: Buffer,
    pub device_tree_depth_info: Buffer,
    pub device_partition_index_pool: Buffer,
    pub device_partition_info_pool: Buffer,
    pub device_partitions: Buffer,
    pub directions: Buffer,
    pub non_zero_direction_indices: Buffer,
    pub num_non_zero_direction_indices: Buffer,
    pub small_side_indexes: Buffer,
    pub small_side_index_to_partition_numbers: Buffer,
    pub device_all_partition_splits_pool: Buffer,
    pub device_feature_partition_splits_pool: Buffer,
}

/// The bytes of a tree's partitions' buckets on the device (per bag in training); the width is cut to fit.
pub const DEVICE_BUCKET_POOL_BYTES: usize = 64 << 20;
/// The most partition splits a tree keeps on the device (2.4 MB each, per tree in flight); the device stops before
/// a level whose splits would not fit.
pub const DEVICE_SPLITS_POOL_MAX: usize = 1 << 16;

/// The device's width (active partitions at a level) for trees of `max_num_active_buckets` buckets: the
/// trainer's width, cut so that the buckets of every partition fit `DEVICE_BUCKET_POOL_BYTES` (at least 2).
pub fn device_width(max_partition_count: u32, max_num_active_buckets: usize) -> u32 {
    let fit = DEVICE_BUCKET_POOL_BYTES / (max_num_active_buckets.max(1) * std::mem::size_of::<W>());
    max_partition_count.min(fit.min(u32::MAX as usize) as u32).max(2)
}

/// The device's pool of every level's partition splits (`deviceAllPartitionSplitsPool`) for `levels` levels of at
/// most `width` partitions: the finished partitions of every level and the last level's search, written after them,
/// up to `DEVICE_SPLITS_POOL_MAX`; and whether that is all of them. Upstream had 256000 entries whatever the depth,
/// and overran them from depth 18.
pub fn device_splits_pool(levels: u32, width: u32) -> (usize, bool) {
    let all: usize = (0..=levels + 1).map(|d| (1usize << d.min(40)).min(width as usize)).sum();
    (all.min(DEVICE_SPLITS_POOL_MAX), all <= DEVICE_SPLITS_POOL_MAX)
}

/// One tree's scheduled training: its event, and the memory its results land in, in queue order (so the next
/// tree's use of the pools does not touch them). When the device leaves levels for the host, the rows' partitions,
/// and the active partitions' weights and places in the tree, as the last device level left them.
pub struct ScheduledTree {
    pub event: MetalComputeEvent,
    pub partition_splits: Buffer,
    pub tree_depth_info: Buffer,
    pub continuation: Option<(Buffer, Buffer, Buffer)>,
}

impl<'o, 'a> FeatureSamplingTrainerPartition<'o, 'a> {
    pub fn allocate(owner: &'o FeatureSamplingTrainerKernel<'a>, active_features_in: &[Vec<i32>], _debug_name: &str) -> Result<Self, String> {
        let shared = shared()?;
        let d = &shared.device;
        let (nf, nr) = (owner.nf as usize, owner.num_rows as usize);
        let mut max_num_active_buckets = 0usize;
        let mut max_num_active_features = 0usize;
        for af in active_features_in {
            max_num_active_features = max_num_active_features.max(af.len());
            let n: usize = af.iter().map(|&f| owner.features[f as usize].buckets.num_buckets as usize).sum();
            max_num_active_buckets = max_num_active_buckets.max(n);
        }
        let width = device_width(owner.max_partition_count, max_num_active_buckets);
        let mpc = width as usize;
        let (splits_pool, all_splits_fit) = device_splits_pool(owner.device_levels, width);
        let can_stop = !all_splits_fit || (width as u64) < (1u64 << owner.device_levels);
        Ok(FeatureSamplingTrainerPartition {
            owner,
            width,
            splits_pool,
            can_stop,
            depth_queue: MetalComputeQueue::new(&shared.queue),
            device_tree_training_info: alloc_uninitialized_array_sync::<TreeTrainingInfo>(d, 1),
            device_bucket_numbers: alloc_uninitialized_array_sync::<u32>(d, nf + 1),
            device_feature_is_active: alloc_uninitialized_array_sync::<u32>(d, nf),
            device_active_feature_list: alloc_uninitialized_array_sync::<u32>(d, max_num_active_features),
            device_w_all_pool: alloc_uninitialized_array_sync::<W>(d, mpc),
            device_partition_bucket_pool: alloc_uninitialized_array_sync::<W>(d, mpc * max_num_active_buckets),
            device_tree_depth_info: alloc_uninitialized_array_sync::<TreeDepthInfo>(d, 1),
            device_partition_index_pool: alloc_uninitialized_array_sync::<PartitionIndex>(d, mpc),
            device_partition_info_pool: alloc_uninitialized_array_sync::<PartitionInfo>(d, mpc),
            device_partitions: alloc_uninitialized_array_sync::<RowPartitionInfo>(d, nr),
            directions: alloc_uninitialized_array_sync::<u32>(d, nr.div_ceil(32)),
            non_zero_direction_indices: alloc_uninitialized_array_sync::<UpdateWorkEntry>(d, nr / 2 + 2),
            num_non_zero_direction_indices: alloc_uninitialized_array_sync::<u32>(d, 1),
            small_side_indexes: alloc_uninitialized_array_sync::<u8>(d, mpc),
            small_side_index_to_partition_numbers: alloc_uninitialized_array_sync::<u16>(d, 256),
            device_all_partition_splits_pool: alloc_uninitialized_array_sync::<IndexedPartitionSplit>(d, splits_pool),
            device_feature_partition_splits_pool: alloc_uninitialized_array_sync::<PartitionSplit>(d, mpc * nf),
        })
    }

    /// Sets up one tree: its training info, active features and bucket numbering, the root's wAll and buckets, and
    /// the root's buckets accumulated (`testFeature`), all enqueued.
    pub fn init(&mut self, feature_sampling: i32, feature_vector_sampling: i32, _debug_name: &str, active_features_in: &[i32]) -> Result<PartitionControlData, String> {
        let o = self.owner;
        let nf = o.nf as usize;
        // In feature order: the kernels take the first of equal scores in this list's order, and the tree of a set of
        // features should not depend on the order it is listed in (upstream took the bag's shuffle order, so ties
        // went another way than in the recursive trainer).
        let mut active_features_in = active_features_in.to_vec();
        active_features_in.sort_unstable();
        active_features_in.dedup();
        let mut data = PartitionControlData { active_feature_list: active_features_in.iter().map(|&f| f as u32).collect(), active_features_in, bucket_numbers: vec![0], feature_is_active: vec![0; nf], ..Default::default() };
        let active_feature_set: BTreeSet<i32> = data.active_features_in.iter().copied().collect();
        let num_active_features = data.active_feature_list.len() as u32;
        let mut num_active_buckets = 0u32;
        for i in 0..nf {
            if active_feature_set.contains(&(i as i32)) {
                num_active_buckets += o.features[i].buckets.num_buckets as u32;
                data.feature_is_active[i] = 1;
            }
            data.bucket_numbers.push(num_active_buckets);
        }
        if num_active_features == 0 {
            return Err("no active features".into());
        }
        data.num_active_features = num_active_features;
        data.num_active_buckets = num_active_buckets;
        data.tree_training_info = TreeTrainingInfo {
            max_depth: o.max_depth as u16,
            num_active_features: num_active_features as u16,
            num_active_buckets,
            num_features: nf as u16,
            max_num_active_partitions: self.width as u16,
            max_num_finished_partitions: self.splits_pool as u32,
            num_rows: o.num_rows,
            feature_sampling: feature_sampling as u16,
            feature_vector_sampling: feature_vector_sampling as u16,
        };
        data.tree_depth_info0.num_active_partitions = 1;

        let q = &self.depth_queue;
        q.enqueue_copy_from_host(&self.device_tree_training_info, 0, bytes_of(std::slice::from_ref(&data.tree_training_info)));
        q.enqueue_copy_from_host(&self.device_bucket_numbers, 0, bytes_of(&data.bucket_numbers));
        q.enqueue_copy_from_host(&self.device_feature_is_active, 0, bytes_of(&data.feature_is_active));
        q.enqueue_copy_from_host(&self.device_active_feature_list, 0, bytes_of(&data.active_feature_list));
        q.enqueue_fill_array(&self.device_w_all_pool, &o.rows.w_all, 0, Some(1));
        q.enqueue_fill_array(&self.device_partition_bucket_pool, &W::default(), 0, Some(num_active_buckets as usize));
        q.enqueue_fill_array(&self.device_tree_depth_info, &data.tree_depth_info0, 0, Some(1));
        run(q, "testFeature", &[
            ("decodedRows", B(&o.expanded_row_data)),
            ("numRows", Int(o.num_rows as i64)),
            ("bucketData", B(&o.bucket_data)),
            ("bucketDataOffsets", B(&o.device_bucket_data_offsets)),
            ("bucketNumbers", B(&self.device_bucket_numbers)),
            ("bucketEntryBits", B(&o.device_bucket_entry_bits)),
            ("activeFeatureList", B(&self.device_active_feature_list)),
            ("partitionBuckets", B(&self.device_partition_bucket_pool)),
        ], &[num_active_features, o.num_rows])?;
        Ok(data)
    }

    /// Enqueues every device level of the tree's training (no waiting between them), then the copies of the results
    /// out of the pools; commits, and gives the event and the results' memory. The device splits `device_levels`
    /// levels, then searches the next level's splits (unused: upstream's schedule; they are written past the
    /// finished partitions and not read).
    pub fn schedule_training(&mut self, _debug_name: &str, data: &PartitionControlData) -> Result<ScheduledTree, String> {
        let o = self.owner;
        let shared = shared()?;
        let (naf, nab, nr) = (data.num_active_features, data.num_active_buckets, o.num_rows);
        let result_partition_splits = alloc_uninitialized_array_sync::<IndexedPartitionSplit>(&shared.device, self.splits_pool);
        let result_tree_depth_info = alloc_uninitialized_array_sync::<TreeDepthInfo>(&shared.device, 1);
        let q = &self.depth_queue;
        let levels = o.device_levels + 1;
        for depth in 0..levels {
            run(q, "getPartitionSplits", &[
                ("treeTrainingInfo", B(&self.device_tree_training_info)),
                ("bucketNumbers", B(&self.device_bucket_numbers)),
                ("activeFeatureList", B(&self.device_active_feature_list)),
                ("featureIsOrdinal", B(&o.device_feature_is_ordinal)),
                ("buckets", B(&self.device_partition_bucket_pool)),
                ("wAll", B(&self.device_w_all_pool)),
                ("featurePartitionSplitsOut", B(&self.device_feature_partition_splits_pool)),
                ("treeDepthInfo", B(&self.device_tree_depth_info)),
            ], &[naf])?;
            run(q, "bestPartitionSplit", &[
                ("treeTrainingInfo", B(&self.device_tree_training_info)),
                ("treeDepthInfo", B(&self.device_tree_depth_info)),
                ("activeFeatureList", B(&self.device_active_feature_list)),
                ("featurePartitionSplits", B(&self.device_feature_partition_splits_pool)),
                ("allPartitionSplitsOut", B(&self.device_all_partition_splits_pool)),
                ("partitionIndexes", B(&self.device_partition_index_pool)),
            ], &[])?;
            if depth == levels - 1 {
                continue; // no need to update things, we have finished
            }
            // Now we assign new partition numbers to the active partitions after the split.
            run(q, "assignPartitionNumbers", &[
                ("treeTrainingInfo", B(&self.device_tree_training_info)),
                ("treeDepthInfo", B(&self.device_tree_depth_info)),
                ("allPartitionSplits", B(&self.device_all_partition_splits_pool)),
                ("partitionIndexesOut", B(&self.device_partition_index_pool)),
                ("partitionInfoOut", B(&self.device_partition_info_pool)),
                ("smallSideIndexesOut", B(&self.small_side_indexes)),
                ("smallSideIndexToPartitionOut", B(&self.small_side_index_to_partition_numbers)),
            ], &[])?;
            // First we clear everything on the small side, ready to accumulate the new buckets there.
            run(q, "clearBuckets", &[
                ("treeTrainingInfo", B(&self.device_tree_training_info)),
                ("treeDepthInfo", B(&self.device_tree_depth_info)),
                ("bucketsOut", B(&self.device_partition_bucket_pool)),
                ("wAllOut", B(&self.device_w_all_pool)),
                ("numNonZeroDirectionIndices", B(&self.num_non_zero_direction_indices)),
                ("smallSideIndexes", B(&self.small_side_indexes)),
            ], &[nab])?;
            // Then we calculate our new partition numbers (for each row).
            run(q, "updatePartitionNumbers", &[
                ("treeTrainingInfo", B(&self.device_tree_training_info)),
                ("treeDepthInfo", B(&self.device_tree_depth_info)),
                ("partitions", B(&self.device_partitions)),
                ("directions", B(&self.directions)),
                ("nonZeroDirectionIndices", B(&self.non_zero_direction_indices)),
                ("numNonZeroDirectionIndices", B(&self.num_non_zero_direction_indices)),
                ("smallSideIndexes", B(&self.small_side_indexes)),
                ("allPartitionSplits", B(&self.device_all_partition_splits_pool)),
                ("partitionInfo", B(&self.device_partition_info_pool)),
                ("bucketData", B(&o.bucket_data)),
                ("bucketDataOffsets", B(&o.device_bucket_data_offsets)),
                ("bucketNumbers", B(&self.device_bucket_numbers)),
                ("bucketEntryBits", B(&o.device_bucket_entry_bits)),
                ("featureIsOrdinal", B(&o.device_feature_is_ordinal)),
                ("decodedRows", B(&o.expanded_row_data)),
            ], &[nr])?;
            // Now the small side buckets are clear, we can accumulate their weights.
            run(q, "updateBuckets", &[
                ("treeTrainingInfo", B(&self.device_tree_training_info)),
                ("treeDepthInfo", B(&self.device_tree_depth_info)),
                ("partitions", B(&self.device_partitions)),
                ("directions", B(&self.directions)),
                ("nonZeroDirectionIndices", B(&self.non_zero_direction_indices)),
                ("numNonZeroDirectionIndices", B(&self.num_non_zero_direction_indices)),
                ("buckets", B(&self.device_partition_bucket_pool)),
                ("wAll", B(&self.device_w_all_pool)),
                ("smallSideIndexes", B(&self.small_side_indexes)),
                ("smallSideIndexToPartition", B(&self.small_side_index_to_partition_numbers)),
                ("decodedRows", B(&o.expanded_row_data)),
                ("bucketData", B(&o.bucket_data)),
                ("bucketDataOffsets", B(&o.device_bucket_data_offsets)),
                ("bucketNumbers", B(&self.device_bucket_numbers)),
                ("bucketEntryBits", B(&o.device_bucket_entry_bits)),
                ("activeFeatureList", B(&self.device_active_feature_list)),
                ("featureIsOrdinal", B(&o.device_feature_is_ordinal)),
            ], &[nr, naf + 1 /* +1 is wAll */])?;
            // And then subtract the small sides from the big sides.
            run(q, "fixupBuckets", &[
                ("treeTrainingInfo", B(&self.device_tree_training_info)),
                ("treeDepthInfo", B(&self.device_tree_depth_info)),
                ("buckets", B(&self.device_partition_bucket_pool)),
                ("wAll", B(&self.device_w_all_pool)),
                ("partitionInfo", B(&self.device_partition_info_pool)),
                ("smallSideIndexes", B(&self.small_side_indexes)),
            ], &[nab])?;
        }
        q.enqueue_copy_between_device_regions(&self.device_tree_depth_info, &result_tree_depth_info, 0, 0, std::mem::size_of::<TreeDepthInfo>() as u64);
        q.enqueue_copy_between_device_regions(&self.device_all_partition_splits_pool, &result_partition_splits, 0, 0, self.device_all_partition_splits_pool.length());
        let continuation = (o.device_levels < o.tree_max_depth || self.can_stop).then(|| {
            let copy = |src: &Buffer| {
                let dst = alloc_uninitialized_array_sync::<u8>(&shared.device, src.length() as usize);
                q.enqueue_copy_between_device_regions(src, &dst, 0, 0, src.length());
                dst
            };
            (copy(&self.device_partitions), copy(&self.device_w_all_pool), copy(&self.device_partition_index_pool))
        });
        Ok(ScheduledTree { event: self.depth_queue.flush(), partition_splits: result_partition_splits, tree_depth_info: result_tree_depth_info, continuation })
    }

    /// Builds the tree from the finished partitions' splits (the work must have completed), continuing on the host
    /// the partitions the device left active when the tree is deeper than its levels (from their depth, to the
    /// tree's: upstream's continuation, behind its early return, started a level too deep and went a level too
    /// far, and read pools the next tree had since overwritten).
    pub fn finish(&self, tree_number: i32, debug_name: &str, data: &PartitionControlData, scheduled: &ScheduledTree) -> Result<Tree, String> {
        Ok(self.finish_with_depth(tree_number, debug_name, data, scheduled)?.0)
    }

    /// `finish`, with the depth the device reached (the host trained the levels from there).
    pub fn finish_with_depth(&self, _tree_number: i32, _debug_name: &str, data: &PartitionControlData, scheduled: &ScheduledTree) -> Result<(Tree, u32), String> {
        let o = self.owner;
        let tree_depth_info = transfer_to_host_sync::<TreeDepthInfo>(&scheduled.tree_depth_info, 1)[0];
        let stopped = tree_depth_info.status == TREE_DEPTH_STATUS_STOPPED;
        if tree_depth_info.status != 0 && !stopped {
            return Err(format!("the device stopped with status {}: {}", tree_depth_info.status as i32, print_tree_depth_info(&tree_depth_info)));
        }
        if !stopped && tree_depth_info.depth != o.device_levels || stopped && tree_depth_info.depth >= o.device_levels {
            return Err(format!("the device reached depth {} of {} (status {})", tree_depth_info.depth, o.device_levels, tree_depth_info.status));
        }
        let device_depth = tree_depth_info.depth;
        if device_depth < o.tree_max_depth && scheduled.continuation.is_none() {
            return Err(format!("the device stopped at depth {device_depth} with nothing to continue from"));
        }
        let num_finished_partitions = tree_depth_info.num_finished_partitions as usize;
        let all_partition_splits = transfer_to_host_sync::<IndexedPartitionSplit>(&scheduled.partition_splits, num_finished_partitions);
        let mut tree = Tree::new();
        let mut leaves = finished_leaves(&mut tree, &all_partition_splits);
        if let Some((partitions, w_all, indexes)) = &scheduled.continuation {
            let nap = tree_depth_info.num_active_partitions as usize;
            let partitions = transfer_to_host_sync::<RowPartitionInfo>(partitions, o.num_rows as usize);
            let w_all = transfer_to_host_sync::<W>(w_all, nap);
            let indexes = transfer_to_host_sync::<PartitionIndex>(indexes, nap);
            let decoded_rows = transfer_to_host_sync::<f32>(&o.expanded_row_data, o.num_rows as usize);
            let mut features = o.features.to_vec();
            for (i, f) in features.iter_mut().enumerate() {
                f.active = data.feature_is_active[i] != 0;
            }
            let new_leaves = split_and_recurse_partitioned(device_depth as i32, o.tree_max_depth as i32, &mut tree, None, data.num_active_buckets, &data.bucket_numbers, &features, &data.active_features_in, &decoded_rows, &partitions, &w_all, &indexes, o.fs)?;
            leaves.1.extend(new_leaves);
        }
        tree.root = extract_tree(0, o.tree_max_depth as i32, &mut tree, PartitionIndex::root(), &leaves.0, &leaves.1, o.features, o.fs)?;
        Ok((tree, device_depth))
    }
}

/// `finish`'s reading of the finished partitions: each one's split, and leaves for its sides until a deeper split
/// replaces them.
pub fn finished_leaves(tree: &mut Tree, all_partition_splits: &[IndexedPartitionSplit]) -> (BTreeMap<PartitionIndex, PartitionSplit>, BTreeMap<PartitionIndex, Ptr>) {
    let mut all_splits: BTreeMap<PartitionIndex, PartitionSplit> = BTreeMap::new();
    let mut leaves: BTreeMap<PartitionIndex, Ptr> = BTreeMap::new();
    for split in all_partition_splits {
        if split.index == PartitionIndex::none() || !split.split.valid() {
            continue;
        }
        let index = split.index;
        all_splits.insert(index, split.split);
        leaves.remove(&index);
        if split.split.left.count() == 0 && split.split.right.count() == 0 {
            continue;
        }
        leaves.insert(index.left_child(), get_leaf(tree, &split.split.left));
        leaves.insert(index.right_child(), get_leaf(tree, &split.split.right));
    }
    (all_splits, leaves)
}

/// The tree from a schedule the device finished (`tree_depth_info.depth + 1 == max_depth`).
pub fn tree_from_splits(tree_depth_info: &TreeDepthInfo, all_partition_splits: &[IndexedPartitionSplit], max_depth: u32, features: &[Feature], fs: &DatasetFeatureSpace) -> Result<Tree, String> {
    assert_eq!(tree_depth_info.depth + 1, max_depth);
    let mut tree = Tree::new();
    let (all_splits, leaves) = finished_leaves(&mut tree, &all_partition_splits[..tree_depth_info.num_finished_partitions as usize]);
    tree.root = extract_tree(0, max_depth as i32, &mut tree, PartitionIndex::root(), &all_splits, &leaves, features, fs)?;
    Ok(tree)
}

/// Trains a tree over the rows on the device, from the root.
#[allow(clippy::too_many_arguments)]
pub fn train_partitioned_end_to_end(feature_vector_sampling: i32, _feature_sampling: i32, debug_name: &str, depth: i32, max_depth: i32, tree: &mut Tree, rows: &Rows, features: &[Feature], bucket_memory: FrozenMemoryRegionT<u32>, fs: &DatasetFeatureSpace) -> Result<Ptr, String> {
    if depth != 0 {
        return crate::forest::randomforest_recursive::train_partitioned_end_to_end_cpu(depth, max_depth, tree, rows, features, fs);
    }
    let features_active: Vec<i32> = features.iter().enumerate().filter(|(_, f)| f.active).map(|(i, _)| i as i32).collect();
    let trainer = FeatureSamplingTrainerKernel::init(debug_name, max_depth, rows, features, bucket_memory, fs)?;
    *tree = trainer.train_partitioned(feature_vector_sampling, debug_name, &[features_active])?.remove(0);
    Ok(tree.root)
}

impl PartitionData {
    /// The features of the feature space: active when the column has more than one value, ordinal when its values
    /// are only numeric.
    pub fn new(fs: Arc<DatasetFeatureSpace>) -> Self {
        let mut features = vec![Feature::default(); fs.column_info.len()];
        for (hash, c) in &fs.column_info {
            let f = &mut features[c.index as usize];
            f.active = c.distinct_values > 1;
            f.buckets = c.buckets.clone();
            f.info = Some(*hash);
            f.ordinal = c.bucket_descriptions.is_only_numeric();
        }
        PartitionData { fs, rows: Rows::default(), bucket_memory: Default::default(), features }
    }

    pub fn clear(&mut self) {
        self.rows.clear();
        self.features.clear();
    }

    /// A bag: the rows reweighted by `counts` (each row's weight times its count times `scale`) and compacted to those
    /// with weight, numbered from zero, with the active features' buckets re-encoded over them in one block of
    /// memory (one device transfer). The weights are encoded as int multiples of the smallest when every one is
    /// exactly that, else as a table when there are fewer than 4096 of them, else as floats: the first format in
    /// which each weight decodes as itself.
    ///
    /// Departures, each a fault upstream: the int-multiple check was disabled (`!isInt && false`), so every bag
    /// was int multiples and weights that are not were rounded to them; the table's weight bits were those of zero
    /// weights (it was never chosen); a row with a count but no weight was left out of the rows but not of the
    /// buckets, so the two disagreed; the bucket lists were sized by the column's distinct values but written with
    /// the feature's bucket count. Inactive features carry no buckets (upstream kept the old ones, outside the
    /// bag's memory). The weights must total less than 2 (W32 accumulates them with 30 fraction bits).
    pub fn reweight_and_compact(&self, counts: &[u32], scale: f64, serializer: &mut MemorySerializer) -> Result<PartitionData, String> {
        let n = self.rows.row_count();
        if counts.len() != n {
            return Err(format!("{} counts for {n} rows", counts.len()));
        }
        let weight_of = |i: usize| (self.rows.get_weight(i) as f64 * counts[i] as f64 * scale) as f32;
        let mut uniques = Vec::new();
        let mut total = 0f64;
        let mut keep = Vec::new();
        for i in 0..n {
            let w = weight_of(i);
            if !(w >= 0.0) || !w.is_finite() {
                return Err(format!("row {i}: weight {w}"));
            }
            if w != 0.0 {
                keep.push(i);
                uniques.push(w);
                total += w as f64;
            }
        }
        if keep.is_empty() {
            return Err("no rows with weight".into());
        }
        if total >= 2.0 {
            return Err(format!("the weights total {total}: they must total less than 2"));
        }
        uniques.sort_by(f32::total_cmp);
        uniques.dedup();
        let min_weight = uniques[0];
        let int_multiple = || -> Option<WeightEncoder> {
            let max_factor = (uniques[uniques.len() - 1] / min_weight).round();
            if max_factor >= (1u64 << 32) as f32 {
                return None;
            }
            let enc = WeightEncoder { weight_format: WeightFormat::WfIntMultiple, weight_multiplier: min_weight, weight_bits: 32 - (max_factor as u32).leading_zeros() as i32, ..Default::default() };
            uniques.iter().all(|&w| enc.encode_weight(w).is_ok()).then_some(enc)
        };
        let weight_encoder = int_multiple().unwrap_or_else(|| {
            if uniques.len() < 4096 {
                // At most 16kb of table, in the cache.
                let mut t = serializer.allocate_writable_t::<f32>(uniques.len(), 4);
                t.data_mut().copy_from_slice(&uniques);
                let bits = 32 - (uniques.len() as u32 - 1).leading_zeros() as i32;
                WeightEncoder { weight_format: WeightFormat::WfTable, weight_format_table: serializer.freeze_t(&mut t), weight_bits: bits, weight_multiplier: 0.0 }
            } else {
                WeightEncoder { weight_format: WeightFormat::WfFloat, weight_bits: 32, ..Default::default() }
            }
        });
        let num_non_zero = keep.len();
        let mut src_rows = Rows { weight_encoder, ..Default::default() };
        let mut writer = src_rows.get_row_writer(num_non_zero, num_non_zero, serializer, false, 1.0);
        for (k, &i) in keep.iter().enumerate() {
            writer.add_row_values(self.rows.get_row(i).label(), weight_of(i), k as u32)?;
        }
        src_rows = writer.freeze(serializer);

        // The active features' buckets over the kept rows, each encoded on its own (in parallel), then in one block.
        use rayon::prelude::*;
        let lists: Vec<Option<(Vec<u32>, i32, i32)>> = self.features.par_iter().map(|f| {
            if !f.active {
                return None;
            }
            let mut s = MemorySerializer;
            let mut bl = WritableBucketList::new(num_non_zero, f.buckets.num_buckets as u32, &mut s);
            for &i in &keep {
                bl.write(f.buckets.get(self.rows.get_example_num(i)));
            }
            let frozen = bl.freeze(&mut s);
            Some((frozen.storage.data().to_vec(), frozen.entry_bits, frozen.num_buckets))
        }).collect();
        let words: usize = lists.iter().flatten().map(|l| l.0.len()).sum();
        let mut block = serializer.allocate_writable_t::<u32>(words.max(1), 4096 /* page aligned */);
        let mut spans = Vec::new();
        let mut at = 0;
        for l in &lists {
            spans.push(l.as_ref().map(|l| {
                block.data_mut()[at..at + l.0.len()].copy_from_slice(&l.0);
                at += l.0.len();
                (at - l.0.len(), l.0.len(), l.1, l.2)
            }));
        }
        let bucket_memory = serializer.freeze_t(&mut block);
        let features = self.features.iter().zip(spans).map(|(f, span)| {
            let buckets = match span {
                Some((start, len, entry_bits, num_buckets)) => BucketList { storage: FrozenMemoryRegionT::new(bucket_memory.raw().range(start * 4, (start + len) * 4)), entry_bits, num_buckets, num_entries: num_non_zero },
                None => BucketList::default(),
            };
            Feature { active: f.active, ordinal: f.ordinal, info: f.info, buckets }
        }).collect();
        Ok(PartitionData { fs: self.fs.clone(), rows: src_rows, bucket_memory, features })
    }

    /// Splits the rows between two datasets by the split, keeping the example numbers (so the buckets are shared).
    pub fn split_without_reindex(&self, feature_to_split_on: usize, split_value: i32, serializer: &mut MemorySerializer) -> [PartitionData; 2] {
        let f = &self.features[feature_to_split_on];
        let highest = self.rows.highest_example_num() as usize;
        let mut writers = [0, 1].map(|_| self.rows.get_row_writer(self.rows.row_count(), highest, serializer, false, 1.0));
        let mut it = self.rows.get_row_iterator();
        for _ in 0..self.rows.row_count() {
            let row = it.get_row();
            let side = side_of(f.buckets.get(row.example_num()), split_value, f.ordinal);
            writers[side as usize].add_row(&row);
        }
        let [w0, w1] = writers;
        [w0, w1].map(|w| PartitionData { fs: self.fs.clone(), rows: w.freeze(serializer), bucket_memory: self.bucket_memory.clone(), features: self.features.clone() })
    }

    /// Splits the rows between two datasets by the split, numbering each side's examples from zero and re-encoding
    /// the active features' buckets over each side's rows.
    pub fn split_and_reindex(&self, feature_to_split_on: usize, split_value: i32, serializer: &mut MemorySerializer) -> [PartitionData; 2] {
        let f = &self.features[feature_to_split_on];
        let n = self.rows.row_count();
        let mut lr = vec![false; n];
        let mut num_on_side = [0usize; 2];
        let mut writers = [0, 1].map(|_| self.rows.get_row_writer(n, n, serializer, true, 1.0));
        let mut it = self.rows.get_row_iterator();
        for side_of_row in lr.iter_mut() {
            let mut row = it.get_row();
            let side = side_of(f.buckets.get(row.example_num()), split_value, f.ordinal);
            *side_of_row = side;
            row.example_num_ = num_on_side[side as usize] as u32;
            num_on_side[side as usize] += 1;
            writers[side as usize].add_row(&row);
        }
        let mut sides = [self.features.clone(), self.features.clone()];
        for (i, feat) in self.features.iter().enumerate() {
            if !feat.active {
                continue;
            }
            let nb = feat.buckets.num_buckets as u32;
            let mut new_features = [WritableBucketList::new(num_on_side[0], nb, serializer), WritableBucketList::new(num_on_side[1], nb, serializer)];
            for (j, &side) in lr.iter().enumerate() {
                new_features[side as usize].write(feat.buckets.get(self.rows.get_example_num(j)));
            }
            for side in 0..2 {
                sides[side][i].buckets = new_features[side].freeze(serializer);
            }
        }
        let [w0, w1] = writers;
        let [f0, f1] = sides;
        [(w0, f0), (w1, f1)].map(|(w, features)| PartitionData { fs: self.fs.clone(), rows: w.freeze(serializer), bucket_memory: Default::default(), features })
    }

    /// Splits the rows by the split: re-indexed when the example numbers are sparse (fewer than a quarter used),
    /// when random access over the old ones would kill the cache.
    pub fn split(&self, feature_to_split_on: i32, split_value: i32, serializer: &mut MemorySerializer) -> Result<[PartitionData; 2], String> {
        if feature_to_split_on < 0 || feature_to_split_on as usize >= self.features.len() {
            return Err(format!("split on feature {feature_to_split_on} of {}", self.features.len()));
        }
        let use_ratio = self.rows.row_count() as f64 / self.rows.highest_example_num() as f64;
        Ok(if use_ratio < 0.25 { self.split_and_reindex(feature_to_split_on as usize, split_value, serializer) } else { self.split_without_reindex(feature_to_split_on as usize, split_value, serializer) })
    }

    /// Trains the tree under this data: partitioned (from the root, on the device), or recursively (the best split
    /// over all features, the rows split, each side trained), or both, compared.
    #[allow(clippy::too_many_arguments)]
    pub fn train(&self, feature_vector_sampling: i32, feature_sampling: i32, debug_name: &str, depth: i32, max_depth: i32, tree: &mut Tree, training_scheme: TrainingScheme) -> Result<Ptr, String> {
        if self.rows.row_count() == 0 {
            return Ok(Ptr::Null);
        }
        if self.rows.row_count() < 2 || depth >= max_depth {
            return Ok(get_leaf(tree, &self.rows.w_all));
        }
        let mut partitioned = None;
        match training_scheme {
            TrainingScheme::Partitioned => return self.train_partitioned(feature_vector_sampling, feature_sampling, debug_name, depth, max_depth, tree),
            TrainingScheme::BothAndCompare if depth == 0 => {
                let mut t = Tree::new();
                let p = self.train_partitioned(feature_vector_sampling, feature_sampling, debug_name, depth, max_depth, &mut t)?;
                partitioned = Some((t, p));
            }
            _ => {}
        }
        let (best_score, best_feature, best_split, w_left, w_right, _new_active) = test_all(depth, &self.features, &self.rows);
        if best_feature == -1 {
            return Ok(get_leaf(tree, &self.rows.w_all));
        }
        let mut serializer = MemorySerializer;
        let [mut left_data, mut right_data] = self.split(best_feature, best_split, &mut serializer)?;
        let (left_rows, right_rows) = (left_data.rows.row_count(), right_data.rows.row_count());
        if left_rows == 0 || right_rows == 0 {
            return Err(format!("Invalid split in random forest: leftRows {left_rows} rightRows {right_rows} bestFeature {best_feature} bestSplit {best_split} wLeft {:?} wRight {:?}", w_left, w_right));
        }
        let scheme = if training_scheme == TrainingScheme::BothAndCompare { TrainingScheme::Recursive } else { training_scheme };
        let left = left_data.train(feature_vector_sampling, feature_sampling, debug_name, depth + 1, max_depth, tree, scheme)?;
        left_data.clear();
        let right = right_data.train(feature_vector_sampling, feature_sampling, debug_name, depth + 1, max_depth, tree, scheme)?;
        right_data.clear();
        let result = if left.is_some() && right.is_some() {
            get_node(tree, best_score as f32, best_feature, best_split, left, right, w_left, w_right, &self.features, &self.fs)?
        } else {
            get_leaf(tree, &(w_left + w_right))
        };
        if let Some((t, p)) = partitioned {
            let mut why = String::new();
            if !compare_trees(tree, result, &t, p, PartitionIndex::root(), &self.fs, &mut why) {
                return Err(format!("the recursive and partitioned trees differ:\n{why}"));
            }
        }
        Ok(result)
    }

    pub fn train_partitioned(&self, feature_vector_sampling: i32, feature_sampling: i32, debug_name: &str, depth: i32, max_depth: i32, tree: &mut Tree) -> Result<Ptr, String> {
        train_partitioned_end_to_end(feature_vector_sampling, feature_sampling, debug_name, depth, max_depth, tree, &self.rows, &self.features, self.bucket_memory.clone(), &self.fs)
    }

    /// Trains one tree per set of active features (`featuresActive`): partitioned, on the device in one trainer;
    /// otherwise each tree on its own copy of the data with only its features active (in parallel). A tree whose
    /// features give no split at the root is one leaf, the rows' distribution (upstream: no tree from the
    /// partitioned trainer, which a committee reads as 0/0, and an error from the recursive one).
    pub fn train_multiple_samplings(&self, feature_vector_sampling: i32, debug_name: &str, max_depth: i32, features_active: &[Vec<i32>], training_scheme: TrainingScheme) -> Result<Vec<Tree>, String> {
        let mut trees = self.train_multiple_samplings_raw(feature_vector_sampling, debug_name, max_depth, features_active, training_scheme)?;
        for t in &mut trees {
            if !t.root.is_some() {
                t.root = get_leaf(t, &self.rows.w_all);
            }
        }
        Ok(trees)
    }

    fn train_multiple_samplings_raw(&self, feature_vector_sampling: i32, debug_name: &str, max_depth: i32, features_active: &[Vec<i32>], training_scheme: TrainingScheme) -> Result<Vec<Tree>, String> {
        if training_scheme == TrainingScheme::Partitioned {
            let trainer = FeatureSamplingTrainerKernel::init(debug_name, max_depth, &self.rows, &self.features, self.bucket_memory.clone(), &self.fs).map_err(|e| format!("Error training partition {debug_name}: {e}"))?;
            return trainer.train_partitioned(feature_vector_sampling, debug_name, features_active).map_err(|e| format!("Error training partition {debug_name}: {e}"));
        }
        use rayon::prelude::*;
        features_active.par_iter().enumerate().map(|(i, af)| {
            let mut my_data = self.clone();
            for f in my_data.features.iter_mut() {
                f.active = false;
            }
            for &f in af {
                my_data.features[f as usize].active = true;
            }
            let mut tree = Tree::new();
            tree.root = my_data.train(feature_vector_sampling, i as i32, debug_name, 0, max_depth, &mut tree, training_scheme).map_err(|e| format!("Error training tree {i}: {e}"))?;
            Ok(tree)
        }).collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::forest::dataset_feature_space::{ColumnInfo, FeatureType};
    use crate::forest::memory_region::FrozenMemoryRegion;
    use crate::forest::randomforest_kernels as host;

    /// Rows with int-multiple weights (as the device decodes them) and the features given (bucket count, ordinal,
    /// the bucket of each row), their buckets in one blob; example numbers are row numbers.
    pub(crate) fn dataset_from(nr: u32, specs: &[(u32, bool)], bucket_of: &dyn Fn(usize, u32) -> u32, label: &dyn Fn(u32) -> bool, weight_multiplier: f32) -> PartitionData {
        let mut s = MemorySerializer;
        let enc = WeightEncoder { weight_format: WeightFormat::WfIntMultiple, weight_bits: 5, weight_multiplier, ..Default::default() };
        let mut w = RowWriter::new(&enc, nr as usize, nr as usize, &mut s, true, 1.0);
        for r in 0..nr {
            w.add_row_values(label(r), (1 + r % 3) as f32 * weight_multiplier, r).unwrap();
        }
        let rows = w.freeze(&mut s);
        let mut blob: Vec<u32> = Vec::new();
        let mut spans = Vec::new();
        for (f, &(nb, _)) in specs.iter().enumerate() {
            let mut bl = WritableBucketList::new(nr as usize, nb, &mut s);
            for r in 0..nr {
                bl.write(bucket_of(f, r));
            }
            let frozen = bl.freeze(&mut s);
            spans.push((blob.len(), frozen.storage.length(), frozen.entry_bits));
            blob.extend_from_slice(frozen.storage.data());
        }
        let mut whole = s.allocate_writable_t::<u32>(blob.len(), 4096);
        whole.data_mut().copy_from_slice(&blob);
        let bucket_memory = s.freeze_t(&mut whole);
        let mut fs = DatasetFeatureSpace::default();
        let mut features = Vec::new();
        for (f, &(start, len, entry_bits)) in spans.iter().enumerate() {
            let (nb, ordinal) = specs[f];
            let raw: FrozenMemoryRegion = bucket_memory.raw().range(start * 4, (start + len) * 4);
            let buckets = BucketList { storage: FrozenMemoryRegionT::new(raw), entry_bits, num_buckets: nb as i32, num_entries: nr as usize };
            let mut bucket_descriptions = crate::forest::bucket::BucketDescriptions::default();
            bucket_descriptions.initialize_from(false, (0..nb).map(|v| v as f64).collect(), vec![], nb as i32).unwrap();
            let info = fs.add_column(ColumnInfo { column_name: format!("f{f}"), info: if ordinal { FeatureType::Real } else { FeatureType::Categorical }, index: -1, distinct_values: nb as i32, buckets: buckets.clone(), bucket_descriptions, ..Default::default() });
            features.push(Feature { active: true, ordinal, info: Some(info), buckets });
        }
        PartitionData { fs: Arc::new(fs), rows, bucket_memory, features }
    }

    /// Four features (8, 3 categorical, 50 and 16 buckets) over a label that is a function of them and a little
    /// noise.
    pub(crate) fn dataset(nr: u32) -> PartitionData {
        let bucket_of = move |f: usize, r: u32| match f {
            0 => r % 8,
            1 => r % 3,
            2 => (r * 7) % 50,
            _ => r * 16 / nr,
        };
        dataset_from(nr, &[(8, true), (3, false), (50, true), (16, true)], &bucket_of, &|r| ((r % 8 >= 3) ^ ((r * 7) % 50 < 10)) || r % 13 == 0, 1.0 / 4096.0)
    }

    fn hash(r: u32, f: u32) -> u32 {
        let mut x = r.wrapping_mul(0x9E37_79B9) ^ f.wrapping_mul(0x85EB_CA6B);
        x ^= x >> 15;
        x = x.wrapping_mul(0x2C1B_3C6D);
        x ^= x >> 12;
        x = x.wrapping_mul(0x297A_2D39);
        x ^ (x >> 15)
    }

    /// Fourteen coarse hashed features (4 to 8 buckets; two categorical with 3) over a label that depends on
    /// three of them and on heavy noise (3 rows in 8 flipped). Every partition stays impure and no split can peel
    /// off a few pure rows, so splits stay balanced and the width doubles level after level.
    pub(crate) fn deep_dataset(nr: u32) -> PartitionData {
        let mut specs: Vec<(u32, bool)> = (0..12).map(|f| (4 + f % 5, true)).collect();
        specs.extend([(3, false), (3, false)]);
        let nbs: Vec<u32> = specs.iter().map(|s| s.0).collect();
        let bucket_of = move |f: usize, r: u32| hash(r, f as u32) % nbs[f];
        let label = move |r: u32| {
            let (a, b, c) = (hash(r, 0) % 4, hash(r, 3) % 7, hash(r, 12) % 3);
            let noise = (r as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93).rotate_right(29).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 61;
            (a < 2) ^ (b >= 4) ^ (c == 1) ^ (noise < 3)
        };
        // Totals stay below 2 (W32): weights 1..3 multiples, nr * 2 * m < 2.
        let m = 1.0 / (1u32 << (32 - nr.leading_zeros() + 1)) as f32;
        dataset_from(nr, &specs, &bucket_of, &label, m)
    }

    /// The same schedule as `schedule_training`, over the kernels' host forms; its tree.
    fn train_on_host(d: &PartitionData, max_depth_in: i32) -> Tree {
        let nr = d.rows.row_count() as u32;
        let nf = d.features.len();
        let max_depth = max_depth_in as u32 + 1;
        let mpc = 65535u32.min(1 << 16u32.min(max_depth_in as u32)) as usize;
        let decoded = host::decode_rows(&d.rows);
        let base = d.bucket_memory.data().as_ptr() as usize;
        let all: &[u32] = d.bucket_memory.data();
        let offsets: Vec<u32> = d.features.iter().map(|f| ((f.buckets.storage.data().as_ptr() as usize - base) / 4) as u32).chain([all.len() as u32 * 2]).collect();
        let entry_bits: Vec<u32> = d.features.iter().map(|f| f.buckets.entry_bits as u32).collect();
        let ordinal: Vec<u32> = d.features.iter().map(|f| f.ordinal as u32).collect();
        let active: Vec<u32> = (0..nf as u32).collect();
        let mut numbers = vec![0u32];
        for f in &d.features {
            numbers.push(numbers.last().unwrap() + f.buckets.num_buckets as u32);
        }
        let nab = *numbers.last().unwrap();
        let tti = [TreeTrainingInfo { num_rows: nr, max_depth: max_depth as u16, num_active_features: nf as u16, num_features: nf as u16, max_num_active_partitions: mpc as u16, num_active_buckets: nab, max_num_finished_partitions: u32::MAX, ..Default::default() }];
        let mut tdi = [TreeDepthInfo { num_active_partitions: 1, ..Default::default() }];
        let mut buckets = vec![W::default(); mpc * nab as usize];
        let mut w_all = vec![W::default(); mpc];
        w_all[0] = d.rows.w_all;
        for fidx in 0..nf as u32 {
            host::test_feature_kernel_host(fidx, nf as u32, 0..nr, &decoded, nr, all, &offsets, &numbers, &entry_bits, &active, &mut buckets);
        }
        let mut splits = vec![PartitionSplit::default(); mpc * nf];
        let mut all_splits = vec![IndexedPartitionSplit::default(); 4096];
        let mut indexes = vec![PartitionIndex::none(); mpc];
        let mut info = vec![PartitionInfo::default(); mpc];
        let mut ssi = vec![0u8; mpc];
        let mut ssi2p = vec![0u16; 256];
        let mut partitions = vec![RowPartitionInfo::default(); nr as usize];
        let mut work = vec![UpdateWorkEntry::default(); nr as usize / 2 + 2];
        let mut nndi = [0u32];
        for depth in 0..max_depth {
            for fidx in 0..nf as u32 {
                host::get_partition_splits_kernel(fidx, nf as u32, &tti, &numbers, &active, &ordinal, &buckets, &w_all, &mut splits, &tdi);
            }
            host::best_partition_split_kernel(&tti, &tdi, &active, &splits, &indexes, &mut all_splits);
            if depth == max_depth - 1 {
                continue;
            }
            host::assign_partition_numbers_kernel(&tti, &mut tdi, &all_splits, &mut indexes, &mut info, &mut ssi, &mut ssi2p);
            host::clear_buckets_kernel(0..nab, &tti, &tdi, &mut buckets, &mut w_all, &mut nndi, &ssi);
            host::update_partition_numbers_kernel(0..nr, &tti, &tdi, &mut partitions, &mut [], &mut nndi, &mut work, &ssi, &all_splits, &info, all, &offsets, &numbers, &entry_bits, &ordinal, &decoded);
            for fidxp1 in 0..=nf as u32 {
                host::update_buckets_kernel(0..nr, fidxp1, nf as u32 + 1, &tti, &tdi, &partitions, &[], &nndi, &work, &mut buckets, &mut w_all, &ssi, &ssi2p, &decoded, all, &offsets, &numbers, &entry_bits, &active, &ordinal);
            }
            host::fixup_buckets_kernel(0..nab, &tti, &tdi, &mut buckets, &mut w_all, &info, &ssi);
        }
        tree_from_splits(&tdi[0], &all_splits, max_depth, &d.features, &d.fs).unwrap()
    }

    fn depth_of(t: &Tree, p: Ptr) -> usize {
        match t.node(p) {
            Some(n) => 1 + depth_of(t, n.child_true).max(depth_of(t, n.child_false)),
            None => 0,
        }
    }

    /// Trees trained on Metal, every depth scheduled with no wait between them, are the trees the kernels' host forms
    /// train over the same schedule; several trees (feature samplings) scheduled before any is awaited each come
    /// out the same as trained alone.
    #[test]
    fn trees_trained_on_metal_match_the_host_schedule() {
        if crate::gpu::shared::get().is_none() {
            eprintln!("no Metal device: skipped");
            return;
        }
        let d = dataset(2000);
        for max_depth in [1, 3, 5] {
            let mut tree = Tree::new();
            d.train_partitioned(0, 0, "test", 0, max_depth, &mut tree).unwrap_or_else(|e| panic!("{e}"));
            let reference = train_on_host(&d, max_depth);
            let mut why = String::new();
            assert!(compare_trees(&reference, reference.root, &tree, tree.root, PartitionIndex::root(), &d.fs, &mut why), "max depth {max_depth}:\n{why}");
            assert!(tree.node(tree.root).is_some(), "max depth {max_depth}: a root split");
            assert_eq!(depth_of(&tree, tree.root), max_depth as usize, "max depth {max_depth}");
        }
        let samplings = vec![vec![0, 1, 2, 3], vec![1, 2], vec![0, 3]];
        let trees = d.train_multiple_samplings(0, "test", 3, &samplings, TrainingScheme::Partitioned).unwrap();
        for (i, af) in samplings.iter().enumerate() {
            let mut one = d.clone();
            for (f, feat) in one.features.iter_mut().enumerate() {
                feat.active = af.contains(&(f as i32));
            }
            let mut alone = Tree::new();
            one.train_partitioned(0, 0, "alone", 0, 3, &mut alone).unwrap();
            let mut why = String::new();
            assert!(compare_trees(&alone, alone.root, &trees[i], trees[i].root, PartitionIndex::root(), &d.fs, &mut why), "sampling {i}:\n{why}");
            let used = |t: &Tree| {
                let mut fs = BTreeSet::new();
                let mut stack = vec![t.root];
                while let Some(p) = stack.pop() {
                    if let Some(n) = t.node(p) {
                        fs.insert(d.fs.print_feature(n.split.feature()));
                        stack.extend([n.child_true, n.child_false]);
                    }
                }
                fs
            };
            for f in used(&trees[i]) {
                assert!(af.iter().any(|&a| format!("f{a}") == f), "sampling {i} split on {f}");
            }
        }
    }

    fn train_with(d: &PartitionData, max_depth: i32, how: &str) -> Tree {
        let mut tree = Tree::new();
        match how {
            "device" => {
                d.train_partitioned(0, 0, how, 0, max_depth, &mut tree).unwrap_or_else(|e| panic!("{e}"));
            }
            "cpu" => {
                tree.root = crate::forest::randomforest_recursive::train_partitioned_end_to_end_cpu(0, max_depth, &mut tree, &d.rows, &d.features, &d.fs).unwrap_or_else(|e| panic!("{e}"));
            }
            _ => {
                tree.root = d.train(0, 0, how, 0, max_depth, &mut tree, TrainingScheme::Recursive).unwrap_or_else(|e| panic!("{e}"));
            }
        }
        tree
    }

    fn same(a: &Tree, b: &Tree, fs: &DatasetFeatureSpace, what: &str) {
        let mut why = String::new();
        assert!(compare_trees(a, a.root, b, b.root, PartitionIndex::root(), fs, &mut why), "{what}:\n{why}");
    }

    /// The CPU's partitioned trainer (breadth first, small partitions depth first, then recursing per partition) and
    /// the recursive trainer (split the rows, train each side) give the tree the device gives. The deep data takes
    /// the CPU through the depth-first descent (partitions smaller than the bucket count) and the device past 254
    /// small sides at a level.
    #[test]
    fn cpu_partitioned_and_recursive_trees_match_the_device() {
        if crate::gpu::shared::get().is_none() {
            eprintln!("no Metal device: skipped");
            return;
        }
        let d = dataset(2000);
        for max_depth in [1, 3, 6] {
            let device = train_with(&d, max_depth, "device");
            same(&device, &train_with(&d, max_depth, "cpu"), &d.fs, &format!("cpu, max depth {max_depth}"));
            same(&device, &train_with(&d, max_depth, "recursive"), &d.fs, &format!("recursive, max depth {max_depth}"));
        }
        let d = deep_dataset(20000);
        let device = train_with(&d, 10, "device");
        same(&device, &train_with(&d, 10, "cpu"), &d.fs, "deep, cpu");
        same(&device, &train_with(&d, 10, "recursive"), &d.fs, "deep, recursive");
        let mut widths = vec![0usize; 12];
        let mut stack = vec![(device.root, 0usize)];
        while let Some((p, k)) = stack.pop() {
            if let Some(n) = device.node(p) {
                widths[k] += 1;
                stack.extend([(n.child_true, k + 1), (n.child_false, k + 1)]);
            }
        }
        assert!(widths.iter().any(|&w| w > 254), "a level wider than 254 partitions: {widths:?}");
    }

    fn train_device_levels(d: &PartitionData, max_depth: i32, levels: u32, samplings: &[Vec<i32>]) -> Vec<Tree> {
        let trainer = FeatureSamplingTrainerKernel::init_with_device_levels("levels", max_depth, &d.rows, &d.features, d.bucket_memory.clone(), &d.fs, levels).unwrap();
        assert_eq!(trainer.device_levels, levels.min(max_depth as u32));
        trainer.train_partitioned(0, "levels", samplings).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Spike measurement for deep trees on the device: per tree depth, the nodes (the device's active partitions
    /// at that level), and the time of the device's 14 levels against the tree to depth 20 (the rest on the host).
    #[test]
    #[ignore = "spike measurement (deep trees on the device); run by name with --nocapture"]
    fn spike_deep_tree_widths_and_times() {
        if crate::gpu::shared::get().is_none() {
            return;
        }
        fn widths(t: &Tree, p: Ptr, d: usize, w: &mut Vec<usize>) {
            if let Some(n) = t.node(p) {
                if w.len() <= d {
                    w.resize(d + 1, 0);
                }
                w[d] += 1;
                let (a, b) = (n.child_true, n.child_false);
                widths(t, a, d + 1, w);
                widths(t, b, d + 1, w);
            }
        }
        let n: u32 = std::env::var("SPIKE_ROWS").ok().and_then(|v| v.parse().ok()).unwrap_or(20000);
        let d = deep_dataset(n);
        let all: Vec<Vec<i32>> = vec![(0..d.features.len() as i32).collect()];
        let _warm = train_device_levels(&d, 4, 4, &all);
        let t0 = std::time::Instant::now();
        let t14 = train_device_levels(&d, 14, 14, &all).remove(0);
        let device14 = t0.elapsed();
        let t0 = std::time::Instant::now();
        let t20 = train_device_levels(&d, 20, 14, &all).remove(0);
        let device14_host6 = t0.elapsed();
        let t0 = std::time::Instant::now();
        let trainer = FeatureSamplingTrainerKernel::init("spike", 20, &d.rows, &d.features, d.bucket_memory.clone(), &d.fs).unwrap();
        let (t20d, reached) = trainer.train_partitioned_with_depths(0, "spike", &all).unwrap().remove(0);
        let device20 = t0.elapsed();
        same(&t20, &t20d, &d.fs, "depth 20: 14 device levels and all on the device");
        eprintln!("rows {n}: depth 20 all on the device {device20:?} (reached {reached})");
        let mut w = Vec::new();
        widths(&t20, t20.root, 0, &mut w);
        let mut w14 = Vec::new();
        widths(&t14, t14.root, 0, &mut w14);
        eprintln!("rows {n}: depth 14 on the device {device14:?}; depth 20 (14 device + host) {device14_host6:?}");
        eprintln!("nodes per depth (depth 20): {w:?}");
        eprintln!("max width {}; widest level {}", w.iter().max().unwrap(), w.iter().enumerate().max_by_key(|x| x.1).unwrap().0);
    }

    /// A tree deeper than the device's levels continues on the host from the partitions the device left (the
    /// upstream fault: an early return gave the tree unfinished, and the code behind it was wrong). Whatever level
    /// the device stops at, the tree is the one the device gives alone; trees scheduled together continue each from
    /// its own partitions (they share the device's pools); and a tree deeper than the device's 14 levels is the one
    /// the CPU's partitioned trainer gives.
    #[test]
    fn deeper_trees_continue_on_the_host_where_the_device_stops() {
        if crate::gpu::shared::get().is_none() {
            eprintln!("no Metal device: skipped");
            return;
        }
        let all: Vec<i32> = (0..4).collect();
        let d = dataset(2000);
        let full = train_device_levels(&d, 6, 14, std::slice::from_ref(&all)).remove(0);
        for levels in [1, 2, 4, 5] {
            same(&full, &train_device_levels(&d, 6, levels, std::slice::from_ref(&all))[0], &d.fs, &format!("max depth 6, {levels} device levels"));
        }

        let d = deep_dataset(20000);
        let samplings: Vec<Vec<i32>> = vec![(0..14).collect(), vec![0, 3, 12, 5, 7], vec![1, 2, 3, 4, 13]];
        let alone: Vec<Tree> = samplings.iter().map(|s| train_device_levels(&d, 10, 14, std::slice::from_ref(s)).remove(0)).collect();
        for levels in [3, 7] {
            let together = train_device_levels(&d, 10, levels, &samplings);
            for (i, t) in together.iter().enumerate() {
                same(&alone[i], t, &d.fs, &format!("sampling {i}, {levels} device levels"));
            }
        }

        // Past the 14 levels that are never wider than the device's partitions: the device trains all 17 (this
        // tree is at most 735 wide), and the tree is the CPU's.
        let device = train_with(&d, 17, "device");
        let mut trainer = FeatureSamplingTrainerKernel::init("deep", 17, &d.rows, &d.features, d.bucket_memory.clone(), &d.fs).unwrap();
        assert_eq!(trainer.device_levels, 17);
        let cpu = train_with(&d, 17, "cpu");
        same(&device, &cpu, &d.fs, "max depth 17");
        assert!(depth_of(&device, device.root) > 14, "deeper than 14 levels: {}", depth_of(&device, device.root));
        let all14: Vec<Vec<i32>> = vec![(0..14).collect()];
        let (t, reached) = trainer.train_partitioned_with_depths(0, "deep", &all14).unwrap().remove(0);
        assert_eq!(reached, 17, "the device trains every level");
        same(&t, &cpu, &d.fs, "max depth 17, on the device");

        // A narrower device stops before the first level wider than its partitions, and the host continues from
        // there: the same tree.
        for width in [16, 100, 500] {
            trainer.max_partition_count = width;
            let (t, reached) = trainer.train_partitioned_with_depths(0, "narrow", &all14).unwrap().remove(0);
            assert!(reached < 17, "width {width}: the device stopped at {reached}");
            assert!(reached as usize >= (width as f64).log2() as usize, "width {width}: the device stopped at {reached}");
            same(&t, &cpu, &d.fs, &format!("max depth 17, width {width}"));
        }
    }
}
