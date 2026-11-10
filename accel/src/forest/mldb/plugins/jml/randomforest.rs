//! randomforest.h and randomforest.cc, in Rust (in progress: what is here has been cut from the .cc, the rest is
//! still there).
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
//! `trainPartitionedEndToEnd`, `PartitionData::trainPartitioned` and `trainMultipleSamplings`. Still in the .cc:
//! `reweightAndCompact`, the bag's rows and buckets.
//!
//! The device is Metal (the shared device); upstream's choice among OpenCL, Metal, the CPU and the comparing `MULTI`
//! runtime is not kept, nor the `DEBUG_RF_KERNELS` checks of each kernel against the host (the grid kernels' test
//! makes them), nor the timings. Training below the root (`depth != 0`) goes to the CPU's partitioned trainer, as
//! upstream. The recursive trainer's sides are trained one after the other, as upstream (`singleThreadOnly`); the
//! trees of `trainMultipleSamplings` in parallel.

use crate::forest::compute_kernel_grid::ComputeKernelArgument::{self, Buffer as B, Float, Int};
use crate::forest::compute_kernel_metal::{alloc_uninitialized_array_sync, transfer_to_device_immutable, transfer_to_host_sync, MetalComputeEvent, MetalComputeQueue};
use crate::forest::dataset_feature_space::DatasetFeatureSpace;
use crate::forest::memory_region::FrozenMemoryRegionT;
use crate::forest::randomforest_kernels_grid::kernel;
use crate::forest::bucket::WritableBucketList;
use crate::forest::memory_region::MemorySerializer;
use crate::forest::randomforest_recursive::{extract_tree, side_of, test_all};
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
    pub max_depth: u32,
    pub num_iterations: u32,
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
    pub const MAX_ITERATIONS: u32 = 16;

    pub fn init(_debug_name: &str, max_depth: i32, rows: &Rows, features: &'a [Feature], bucket_memory: FrozenMemoryRegionT<u32>, fs: &'a DatasetFeatureSpace) -> Result<Self, String> {
        let shared = shared()?;
        let device = &shared.device;
        let num_rows = rows.row_count() as u32;
        let nf = features.len() as u32;
        let num_iterations = Self::MAX_ITERATIONS.min(max_depth as u32);
        let max_partition_count = 65535u32.min(1 << num_iterations);

        let (mut bucket_entry_bits, mut feature_is_ordinal, mut bucket_memory_offsets) = (Vec::new(), Vec::new(), Vec::new());
        let mut last_bucket_data_offset = 0u32;
        let base = bucket_memory.data().as_ptr() as usize;
        for f in features {
            let buckets = &f.buckets;
            bucket_entry_bits.push(buckets.entry_bits as u32);
            feature_is_ordinal.push(f.ordinal as u32);
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
            max_depth: max_depth as u32 + 1,
            num_iterations,
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
        let debug_name = format!("{debug_name_in}-{feature_vector_sampling}");
        let mut partition = FeatureSamplingTrainerPartition::allocate(self, all_active_features_in, &debug_name)?;
        let mut outputs = Vec::new();
        for (i, af) in all_active_features_in.iter().enumerate() {
            let bag_debug_name = format!("{debug_name}-{i}");
            let data = partition.init(i as i32, feature_vector_sampling, &bag_debug_name, af).map_err(|e| format!("Error training {debug_name} partition {i}: {e}"))?;
            let (event, splits, depth_info) = partition.schedule_training(&bag_debug_name, &data).map_err(|e| format!("Error training {debug_name} partition {i}: {e}"))?;
            outputs.push((event, splits, depth_info, data));
        }
        let mut result = Vec::new();
        for (i, (event, splits, info, data)) in outputs.into_iter().enumerate() {
            event.await_()?;
            result.push(partition.finish(i as i32, &format!("{debug_name}-{i}"), &data, &splits, &info)?);
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

/// Entries in the pool of every partition split of a tree (`deviceAllPartitionSplitsPool`).
pub const ALL_PARTITION_SPLITS_POOL: usize = 256000;

impl<'o, 'a> FeatureSamplingTrainerPartition<'o, 'a> {
    pub fn allocate(owner: &'o FeatureSamplingTrainerKernel<'a>, active_features_in: &[Vec<i32>], _debug_name: &str) -> Result<Self, String> {
        let shared = shared()?;
        let d = &shared.device;
        let (nf, mpc, nr) = (owner.nf as usize, owner.max_partition_count as usize, owner.num_rows as usize);
        let mut max_num_active_buckets = 0usize;
        let mut max_num_active_features = 0usize;
        for af in active_features_in {
            max_num_active_features = max_num_active_features.max(af.len());
            let n: usize = af.iter().map(|&f| owner.features[f as usize].buckets.num_buckets as usize).sum();
            max_num_active_buckets = max_num_active_buckets.max(n);
        }
        Ok(FeatureSamplingTrainerPartition {
            owner,
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
            device_all_partition_splits_pool: alloc_uninitialized_array_sync::<IndexedPartitionSplit>(d, ALL_PARTITION_SPLITS_POOL),
            device_feature_partition_splits_pool: alloc_uninitialized_array_sync::<PartitionSplit>(d, mpc * nf),
        })
    }

    /// Sets up one tree: its training info, active features and bucket numbering, the root's wAll and buckets, and
    /// the root's buckets accumulated (`testFeature`), all enqueued.
    pub fn init(&mut self, feature_sampling: i32, feature_vector_sampling: i32, _debug_name: &str, active_features_in: &[i32]) -> Result<PartitionControlData, String> {
        let o = self.owner;
        let nf = o.nf as usize;
        let mut data = PartitionControlData { active_features_in: active_features_in.to_vec(), bucket_numbers: vec![0], feature_is_active: vec![0; nf], active_feature_list: active_features_in.iter().map(|&f| f as u32).collect(), ..Default::default() };
        let active_feature_set: BTreeSet<i32> = active_features_in.iter().copied().collect();
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
            max_num_active_partitions: o.max_partition_count as u16,
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

    /// Enqueues every depth of the tree's training (no waiting between them), then the copies of the results out of
    /// the pools; commits, and gives the event and the results' memory.
    pub fn schedule_training(&mut self, _debug_name: &str, data: &PartitionControlData) -> Result<(MetalComputeEvent, Buffer, Buffer), String> {
        let o = self.owner;
        let shared = shared()?;
        let (naf, nab, nr) = (data.num_active_features, data.num_active_buckets, o.num_rows);
        let result_partition_splits = alloc_uninitialized_array_sync::<IndexedPartitionSplit>(&shared.device, ALL_PARTITION_SPLITS_POOL);
        let result_tree_depth_info = alloc_uninitialized_array_sync::<TreeDepthInfo>(&shared.device, 1);
        let q = &self.depth_queue;
        for depth in 0..o.max_depth {
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
            if depth == o.max_depth - 1 {
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
        Ok((self.depth_queue.flush(), result_partition_splits, result_tree_depth_info))
    }

    /// Builds the tree from the finished partitions' splits (the work must have completed).
    pub fn finish(&self, _tree_number: i32, _debug_name: &str, _data: &PartitionControlData, result_partition_splits: &Buffer, result_tree_depth_info: &Buffer) -> Result<Tree, String> {
        let o = self.owner;
        let tree_depth_info = transfer_to_host_sync::<TreeDepthInfo>(result_tree_depth_info, 1)[0];
        let num_finished_partitions = tree_depth_info.num_finished_partitions as usize;
        let all_partition_splits = transfer_to_host_sync::<IndexedPartitionSplit>(result_partition_splits, num_finished_partitions);
        tree_from_splits(&tree_depth_info, &all_partition_splits, o.max_depth, o.features, o.fs)
    }
}

/// `finish`'s building of the tree: each finished partition's split, and leaves for its sides until a deeper split
/// replaces them; the tree extracted from the root.
pub fn tree_from_splits(tree_depth_info: &TreeDepthInfo, all_partition_splits: &[IndexedPartitionSplit], max_depth: u32, features: &[Feature], fs: &DatasetFeatureSpace) -> Result<Tree, String> {
    let depth = tree_depth_info.depth + 1;
    let mut all_splits: BTreeMap<PartitionIndex, PartitionSplit> = BTreeMap::new();
    let mut leaves: BTreeMap<PartitionIndex, Ptr> = BTreeMap::new();
    let mut tree = Tree::new();
    for split in &all_partition_splits[..tree_depth_info.num_finished_partitions as usize] {
        if split.index == PartitionIndex::none() || !split.split.valid() {
            continue;
        }
        let index = split.index;
        all_splits.insert(index, split.split);
        leaves.remove(&index);
        if split.split.left.count() == 0 && split.split.right.count() == 0 {
            continue;
        }
        leaves.insert(index.left_child(), get_leaf(&mut tree, &split.split.left));
        leaves.insert(index.right_child(), get_leaf(&mut tree, &split.split.right));
    }
    if depth < max_depth {
        return Err(format!("the tree stopped at depth {depth} of {max_depth}: deeper trees need splitAndRecursePartitioned (not yet translated)"));
    }
    let result = extract_tree(0, max_depth as i32, &mut tree, PartitionIndex::root(), &all_splits, &leaves, features, fs)?;
    tree.root = result;
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
    /// otherwise each tree on its own copy of the data with only its features active (in parallel).
    pub fn train_multiple_samplings(&self, feature_vector_sampling: i32, debug_name: &str, max_depth: i32, features_active: &[Vec<i32>], training_scheme: TrainingScheme) -> Result<Vec<Tree>, String> {
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
            if !tree.root.is_some() {
                return Err(format!("Error training tree {i}: no tree"));
            }
            Ok(tree)
        }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forest::bucket::{BucketList, WritableBucketList};
    use crate::forest::dataset_feature_space::{ColumnInfo, FeatureType};
    use crate::forest::memory_region::{FrozenMemoryRegion, MappedSerializer, MemorySerializer};
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
        let tti = [TreeTrainingInfo { num_rows: nr, max_depth: max_depth as u16, num_active_features: nf as u16, num_features: nf as u16, max_num_active_partitions: mpc as u16, num_active_buckets: nab, ..Default::default() }];
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
}
