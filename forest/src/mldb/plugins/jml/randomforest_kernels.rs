//! randomforest_kernels.h and randomforest_kernels.cc, in Rust.
//!
//! Jeremy Barnes, 13 October 2018. Copyright (c) 2018 mldb.ai inc. This file is part of MLDB. Copyright 2016 mldb.ai
//! inc. Apache-2.0 (see the LICENSE at `forest/src/mldb/`).
//!
//! Kernels for the random forest algorithm: the split score, and the host (CPU) forms of the trainer's kernels,
//! which the device forms (randomforest_kernels_common.h, through the Metal shim) are checked against.
//!
//! A kernel's grid dimension (`ComputeKernelGridRange`) is a `Range<u32>`; the `ComputeContext` argument, unused by
//! every host form, is dropped. The registrations (`RegisterKernels`: each kernel's dimensions, parameters and
//! constraints, for the compute kernel runtime) are `HOST_KERNELS`, as data. The `if (false)` dumps to
//! `tree-*.txt` files and the trace lines are not brought over.
//!
//! The functions the header declares and randomforest_recursive.cc defines are re-exported from
//! randomforest_recursive.rs as they are translated.

use crate::bucket::BucketListRef;
use crate::fixed_point_accum::FixedPointAccum32;
use crate::memory_region::FrozenMemoryRegionT;
use crate::randomforest_types::{
    IndexedPartitionSplit, PartitionIndex, PartitionInfo, PartitionSplit, PartitionSplitDirection, RowPartitionInfo, Rows, TreeDepthInfo, TreeTrainingInfo, UpdateWorkEntry, WeightFormat, W, RowWeights, SplitCriterion, in_category_set, FEATURE_KIND_ORDINAL, FEATURE_KIND_SUBSETS,
};
use std::ops::Range;

pub use crate::randomforest_recursive::{choose_split_kernel, decode_rows, test_feature_kernel, test_feature_kernel_cpu, test_feature_number, BucketSource, RowSource};

pub const SQRT3_MULT_BEFORE: f32 = 65536.0;
pub const SQRT3_MULT_AFTER: f32 = 1.0 / 256.0;

/// A square root that is exactly the largest float whose square is at most the (scaled) value, so the score is
/// the same wherever it is computed.
#[inline]
pub fn sqrt3(mut val: f32) -> f32 {
    val *= SQRT3_MULT_BEFORE;
    let mut approx = val.sqrt().next_up();
    while approx * approx > val {
        approx = approx.next_down();
    }
    approx * SQRT3_MULT_AFTER
}

#[inline]
/// A split's score; infinite (no split) when a side has fewer rows than the criterion's leaves may, or for the
/// variance when the split does not lower the node's own error (`scoreSplit` on the device).
pub fn score_split(w_false: &W, w_true: &W, criterion: SplitCriterion) -> f32 {
    let m = criterion.min_leaf();
    if w_false.count() < m || w_true.count() < m {
        return f32::INFINITY;
    }
    if criterion.kind() == SplitCriterion::Variance {
        let q = variance_fixed(w_false) + variance_fixed(w_true);
        if q >= variance_fixed(&(*w_false + *w_true)) {
            return f32::INFINITY;
        }
        return q as f32 * (1.0 / (1u64 << 30) as f32);
    }
    let f = |w: &W, i: usize| FixedPointAccum32::to_f32(w.v[i]);
    2.0 * (sqrt3(f(w_false, 0) * f(w_false, 1)) + sqrt3(f(w_true, 0) * f(w_true, 1)))
}

/// A side's squared error about its mean target, less the part every split shares: `f t / (f + t)` in the
/// accumulators' fixed point (2^-30), by integer division, so that the device's (`varianceFixed`) is the same.
#[inline]
pub fn variance_fixed(w: &W) -> i64 {
    let (f, t) = (w.v[0].rep as i64, w.v[1].rep as i64);
    if f + t <= 0 { 0 } else { f * t / (f + t) }
}

/// A feature's bucket list, out of all the features' bucket data.
fn feature_buckets<'a>(f: usize, all_bucket_data: &'a [u32], bucket_data_offsets: &[u32], bucket_numbers: &[u32], bucket_entry_bits: &[u32]) -> BucketListRef<'a> {
    BucketListRef { storage: &all_bucket_data[bucket_data_offsets[f] as usize..], entry_bits: bucket_entry_bits[f], num_entries: (bucket_numbers[f + 1] - bucket_numbers[f]) as usize }
}

#[allow(clippy::too_many_arguments)]
pub fn decode_rows_kernel_cpu(row_range: Range<u32>, row_data: FrozenMemoryRegionT<u64>, _row_data_length: u32, weight_bits: u16, example_num_bits: u16, num_rows: u32, weight_format: WeightFormat, weight_multiplier: f32, weight_data: FrozenMemoryRegionT<f32>, targets: &[f32], decoded_rows_out: &mut [RowWeights]) {
    let mut rows = Rows { row_data, num_row_entries: num_rows as usize, example_num_bits: example_num_bits as i32, ..Default::default() };
    rows.example_num_mask = (1u64 << example_num_bits) - 1;
    rows.weight_mask = (1u64 << weight_bits) - 1;
    rows.total_bits = weight_bits as i32 + example_num_bits as i32 + 1;
    rows.weight_encoder.weight_bits = weight_bits as i32;
    rows.weight_encoder.weight_format = weight_format;
    rows.weight_encoder.weight_multiplier = weight_multiplier;
    rows.weight_encoder.weight_format_table = weight_data;
    rows.targets = (!targets.is_empty()).then(|| std::sync::Arc::new(targets.to_vec()));

    let mut it = rows.get_row_iterator();
    assert_eq!(decoded_rows_out.len(), num_rows as usize);
    for i in row_range {
        it.skip_to(i as usize);
        let row = it.get_decoded_row();
        assert_eq!(i, row.example_num);
        decoded_rows_out[i as usize] = row.weights();
    }
}

/// Accumulates one active feature's buckets over all the rows (the root partition).
#[allow(clippy::too_many_arguments)]
pub fn test_feature_kernel_host(fidx: u32, _naf: u32, rows: Range<u32>, decoded_rows: &[RowWeights], num_rows: u32, all_bucket_data: &[u32], bucket_data_offsets: &[u32], bucket_numbers: &[u32], bucket_entry_bits: &[u32], active_feature_list: &[u32], all_w_out: &mut [W]) {
    let f = active_feature_list[fidx as usize] as usize;
    assert!(bucket_numbers[f + 1] as usize <= all_w_out.len());
    assert_eq!(rows.len() as u32, num_rows); // We fake having this argument...
    let buckets = feature_buckets(f, all_bucket_data, bucket_data_offsets, bucket_numbers, bucket_entry_bits);
    test_feature_kernel(decoded_rows, num_rows as usize, &buckets, &mut all_w_out[bucket_numbers[f] as usize..]);
}

/// For each feature and partition, find the split that gives the best score and record it with one row per
/// partition and one column per feature in the output matrix. This involves testing each split point for the given
/// feature and partition.
#[allow(clippy::too_many_arguments)]
pub fn get_partition_splits_kernel(fidx: u32, naf: u32, tree_training_info: &[TreeTrainingInfo], bucket_numbers: &[u32], active_feature_list: &[u32], feature_is_ordinal: &[u32], buckets: &[W], w_all: &[W], splits_out: &mut [PartitionSplit], tree_depth_info: &[TreeDepthInfo]) {
    let num_active_partitions = tree_depth_info[0].num_active_partitions;
    let total_buckets = tree_training_info[0].num_active_buckets as usize;
    for p in 0..num_active_partitions as usize {
        let f = active_feature_list[fidx as usize] as usize;
        let result = &mut splits_out[p * naf as usize + fidx as usize];
        if w_all[p].empty() || w_all[p].uniform() {
            *result = PartitionSplit::default();
            continue;
        }
        let start_bucket = bucket_numbers[f] as usize;
        let end_bucket = bucket_numbers[f + 1] as usize;
        let max_bucket = end_bucket as i32 - start_bucket as i32 - 1;
        let w_feature = &buckets[p * total_buckets + start_bucket..];
        let criterion = SplitCriterion(tree_training_info[0].split_criterion);
        let (score, value, left, right, subset) = choose_split_kernel(w_feature, max_bucket, feature_is_ordinal[f] == FEATURE_KIND_ORDINAL, feature_is_ordinal[f] == FEATURE_KIND_SUBSETS, &w_all[p], criterion);
        result.score = score as f32;
        result.value = value as i16;
        result.subset = subset;
        result.left = left;
        result.right = right;
        result.feature = if result.score == f32::INFINITY { -1 } else { f as i16 };
    }
}

/// Reduction over the feature columns from `get_partition_splits_kernel`, where we find which feature gives the
/// best split for each partition and record that in the output.
pub fn best_partition_split_kernel(tree_training_info: &[TreeTrainingInfo], tree_depth_info: &[TreeDepthInfo], _feature_active: &[u32], feature_partition_splits: &[PartitionSplit], partition_indexes: &[PartitionIndex], partition_splits_out: &mut [IndexedPartitionSplit]) {
    let partition_splits_offset = tree_depth_info[0].num_finished_partitions as usize;
    let naf = tree_training_info[0].num_active_features as usize;
    let nap = tree_depth_info[0].num_active_partitions as usize;
    let depth = tree_depth_info[0].depth as u16;
    for p in 0..nap {
        assert!(partition_splits_offset + p < partition_splits_out.len(), "overflow: p = {p}, nap = {nap}, partitionSplitsOffset = {partition_splits_offset}, partitionSplitsOut.size() = {}, treeDepthInfo = {:?}", partition_splits_out.len(), tree_depth_info[0]);
        let result = &mut partition_splits_out[partition_splits_offset + p];
        *result = IndexedPartitionSplit::default();
        result.index = if depth == 0 { PartitionIndex::root() } else { partition_indexes[p] };
        if result.index == PartitionIndex::none() {
            continue;
        }
        for fidx in 0..naf {
            let fp = &feature_partition_splits[p * naf + fidx];
            if fp.score == f32::INFINITY {
                continue;
            }
            if fp.score < result.split.score {
                result.split = *fp;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn assign_partition_numbers_kernel(tree_training_info: &[TreeTrainingInfo], tree_depth_info: &mut [TreeDepthInfo], partition_splits: &[IndexedPartitionSplit], partition_indexes_out: &mut [PartitionIndex], partition_info_out: &mut [PartitionInfo], small_side_indexes_out: &mut [u8], small_side_index_to_partition_out: &mut [u16]) {
    let depth = tree_depth_info[0].depth as u16;
    let partition_splits_offset = tree_depth_info[0].num_finished_partitions as usize;
    let num_active_partitions = tree_depth_info[0].num_active_partitions;
    let max_num_active_partitions = tree_training_info[0].max_num_active_partitions as u32;

    assert!(partition_indexes_out.len() >= num_active_partitions as usize);
    assert!(partition_info_out.len() >= num_active_partitions as usize);

    let partition_splits = &partition_splits[partition_splits_offset..];
    small_side_index_to_partition_out.fill(0);

    // First, accumulate a list of inactive partitions.
    let mut inactive_partitions: Vec<u32> = Vec::new();
    let mut num_small_side_rows = 0u32;
    for p in 0..num_active_partitions {
        let split = &partition_splits[p as usize].split;
        if !split.valid() || (split.left.uniform() && split.right.uniform()) {
            inactive_partitions.push(p);
            small_side_indexes_out[p as usize] = 255;
            continue;
        }
        num_small_side_rows += split.left.count().min(split.right.count());
        small_side_indexes_out[p as usize] = 0;
    }

    // Stop before writing this level when the next is wider than the partitions or its splits do not fit the pool
    // (as the Metal kernel).
    if depth != 0 && tree_depth_info[0].status != 0 {
        return;
    }
    let num_continuing = num_active_partitions - inactive_partitions.len() as u32;
    let next_num_active = num_active_partitions + num_continuing.saturating_sub(inactive_partitions.len() as u32);
    let next_num_finished = if depth == 0 { 0 } else { tree_depth_info[0].num_finished_partitions } + num_active_partitions;
    if next_num_active > max_num_active_partitions || next_num_finished as u64 + next_num_active as u64 > tree_training_info[0].max_num_finished_partitions as u64 {
        tree_depth_info[0].status = crate::randomforest_types::TREE_DEPTH_STATUS_STOPPED;
        return;
    }

    let (mut n, mut n2) = (0usize, num_active_partitions);
    let mut ssi: u16 = 0;

    for p in 0..num_active_partitions {
        let split = &partition_splits[p as usize];
        let info = &mut partition_info_out[p as usize];
        if !split.split.valid() || (split.split.left.uniform() && split.split.right.uniform()) {
            *info = PartitionInfo::default();
            continue;
        }
        let direction = split.split.transfer_direction() == PartitionSplitDirection::RL;

        // Both still valid. One needs a new partition number.
        let minor_partition_number = if n < inactive_partitions.len() {
            // Re-use an unused partition.
            n += 1;
            inactive_partitions[n - 1]
        } else if n2 < max_num_active_partitions {
            // Extend the list of partitions.
            n2 += 1;
            n2 - 1
        } else {
            // Max width reached; ignore this partition.
            info.left = -1;
            info.right = -1;
            continue;
        };

        // Attempt to allocate a small side number, and if it's possible record the mapping.
        if ssi < 254 {
            ssi += 1;
            small_side_indexes_out[minor_partition_number as usize] = ssi as u8;
            small_side_index_to_partition_out[ssi as usize] = minor_partition_number as u16;
        } else {
            small_side_indexes_out[minor_partition_number as usize] = 255;
        }

        if !direction {
            info.left = p as i32;
            info.right = minor_partition_number as i32;
        } else {
            info.left = minor_partition_number as i32;
            info.right = p as i32;
        }
        let (l, r) = (info.left as usize, info.right as usize);
        partition_indexes_out[l] = split.index.left_child();
        partition_indexes_out[r] = split.index.right_child();
    }

    let tdi = &mut tree_depth_info[0];
    tdi.depth = depth as u32 + 1;
    tdi.prev_num_active_partitions = tdi.num_active_partitions;
    tdi.num_active_partitions = n2;
    tdi.num_small_side_rows = num_small_side_rows;
    tdi.prev_num_finished_partitions = tdi.num_finished_partitions;
    tdi.num_finished_partitions = tdi.prev_num_finished_partitions + num_active_partitions;
    tdi.status = 0; // for now

    while n < inactive_partitions.len() {
        let p = inactive_partitions[n] as usize;
        n += 1;
        partition_indexes_out[p] = PartitionIndex::none();
        small_side_indexes_out[p] = 0;
    }
}

/// After doubling the number of buckets, this clears the wAll and allPartitionBuckets entries corresponding to
/// active but non-initialized post-split buckets.
pub fn clear_buckets_kernel(bucket_range: Range<u32>, tree_training_info: &[TreeTrainingInfo], tree_depth_info: &[TreeDepthInfo], all_partition_buckets: &mut [W], w_all: &mut [W], num_non_zero_direction_indices: &mut [u32], small_side_indexes: &[u8]) {
    let num_active_partitions = tree_depth_info[0].num_active_partitions as usize;
    let num_active_buckets = tree_training_info[0].num_active_buckets as usize;
    assert_eq!(num_non_zero_direction_indices.len(), 1);
    num_non_zero_direction_indices[0] = 0;
    for partition in 0..num_active_partitions {
        if small_side_indexes[partition] == 0 {
            continue;
        }
        w_all[partition] = W::default();
        assert_eq!(num_active_buckets, bucket_range.len());
        for bucket in bucket_range.clone() {
            all_partition_buckets[partition * num_active_buckets + bucket as usize] = W::default();
        }
    }
}

/// Applies each partition's split to each row (the tree's evaluation on the rows): every row reads its split's
/// feature's bucket, goes to a side, and the rows that change partition are listed for `update_buckets_kernel`.
#[allow(clippy::too_many_arguments)]
pub fn update_partition_numbers_kernel(row_range: Range<u32>, _tree_training_info: &[TreeTrainingInfo], tree_depth_info: &[TreeDepthInfo], partitions: &mut [RowPartitionInfo], _directions: &mut [u32], num_non_zero_direction_indices: &mut [u32], non_zero_direction_indices: &mut [UpdateWorkEntry], small_side_indexes: &[u8], partition_splits: &[IndexedPartitionSplit], partition_info: &[PartitionInfo], all_bucket_data: &[u32], bucket_data_offsets: &[u32], bucket_numbers: &[u32], bucket_entry_bits: &[u32], feature_is_ordinal: &[u32], decoded_rows: &[RowWeights]) {
    let partition_splits_offset = tree_depth_info[0].prev_num_finished_partitions as usize;
    let partition_splits = &partition_splits[partition_splits_offset..];
    let depth = tree_depth_info[0].depth as u16 - 1;
    let nf = bucket_entry_bits.len();
    let mut feature_buckets_cache: Vec<Option<BucketListRef>> = vec![None; nf];
    let mut num_non_zero = 0usize;
    const NONE: u16 = u16::MAX;

    for r in row_range {
        let r_ = r as usize;
        let partition = if depth == 0 { 0 } else { partitions[r_].partition() };

        // Row is not in any partition.
        if partition == NONE {
            if depth == 0 {
                partitions[r_] = NONE.into();
            }
            continue;
        }

        let info = &partition_info[partition as usize];
        if info.ignore() {
            partitions[r_] = NONE.into();
            continue;
        }

        let split_feature = partition_splits[partition as usize].split.feature;
        if split_feature == -1 {
            // Reached a leaf here, nothing to split.
            partitions[r_] = NONE.into();
            continue;
        }
        let split_feature = split_feature as usize;
        let split_value = partition_splits[partition as usize].split.value as u32;
        let subset = partition_splits[partition as usize].split.subset;
        let kind = feature_is_ordinal[split_feature];
        let ordinal = kind == FEATURE_KIND_ORDINAL;

        // Get buckets for the split feature.
        let buckets = *feature_buckets_cache[split_feature].get_or_insert_with(|| feature_buckets(split_feature, all_bucket_data, bucket_data_offsets, bucket_numbers, bucket_entry_bits));
        let bucket = buckets.get(r);
        let side = if ordinal { bucket >= split_value } else if kind == FEATURE_KIND_SUBSETS { !in_category_set(&subset, bucket) } else { bucket != split_value }; // false = left, true = right

        // Set the new partition number.
        let new_partition_number = (if side { info.right } else { info.left }) as u16;
        if depth == 0 {
            partitions[r_] = new_partition_number.into();
        }
        if new_partition_number != partition {
            if depth != 0 {
                partitions[r_] = new_partition_number.into();
            }
            let small_side_index = small_side_indexes[new_partition_number as usize] as u16;
            non_zero_direction_indices[num_non_zero] = UpdateWorkEntry { row: r, partition: new_partition_number, small_side_index, decoded_row: decoded_rows[r_] };
            num_non_zero += 1;
        }
    }
    num_non_zero_direction_indices[0] = num_non_zero as u32;
}

/// Transfers the weight of the rows that changed partition into their new partition's buckets (`fidxp1` 0 is wAll,
/// otherwise it is the active feature `fidxp1 - 1`). Only the rows on the side that moved are touched, as the
/// weight is transferred rather than summed from the beginning: less work for unbalanced splits, which most are.
#[allow(clippy::too_many_arguments)]
pub fn update_buckets_kernel(_row_range: Range<u32>, fidxp1: u32, _nafp1: u32, tree_training_info: &[TreeTrainingInfo], _tree_depth_info: &[TreeDepthInfo], _partitions: &[RowPartitionInfo], _directions: &[u32], num_non_zero_direction_indices: &[u32], non_zero_direction_indices: &[UpdateWorkEntry], partition_buckets: &mut [W], w_all: &mut [W], _small_side_indexes: &[u8], _small_side_index_to_partition: &[u16], _decoded_rows: &[RowWeights], all_bucket_data: &[u32], bucket_data_offsets: &[u32], bucket_numbers: &[u32], bucket_entry_bits: &[u32], feature_active: &[u32], _feature_is_ordinal: &[u32]) {
    let fidx = fidxp1 as i32 - 1; // -1 means wAll, otherwise it's the feature number
    let f = if fidx == -1 { None } else { Some(feature_active[fidx as usize] as usize) };
    let num_active_buckets = tree_training_info[0].num_active_buckets as usize;

    // Set up access to the buckets of the feature we're updating for the split.
    let buckets = f.map(|f| feature_buckets(f, all_bucket_data, bucket_data_offsets, bucket_numbers, bucket_entry_bits));
    let (w_global, start_bucket) = match f {
        None => (w_all, 0),
        Some(f) => (partition_buckets, bucket_numbers[f] as usize),
    };

    let num_work_items = num_non_zero_direction_indices[0] as usize;
    for work_item in &non_zero_direction_indices[..num_work_items] {
        let partition = work_item.partition as usize;
        let to_bucket = match &buckets {
            None => partition,
            Some(b) => partition * num_active_buckets + start_bucket + b.get(work_item.row) as usize,
        };
        w_global[to_bucket].add_weights(work_item.decoded_row);
    }
}

/// For each partition and each bucket, we up to now accumulated just the weight that needs to be transferred in
/// the small side of the partition splits. We need to fix this up by subtracting this weight from the big side.
pub fn fixup_buckets_kernel(bucket_range: Range<u32>, tree_training_info: &[TreeTrainingInfo], tree_depth_info: &[TreeDepthInfo], all_partition_buckets: &mut [W], w_all: &mut [W], partition_info: &[PartitionInfo], small_side_indexes: &[u8]) {
    let num_active_buckets = tree_training_info[0].num_active_buckets as usize;
    let num_active_partitions = tree_depth_info[0].prev_num_active_partitions as usize;
    assert_eq!(num_active_buckets, bucket_range.len());

    for info in &partition_info[..num_active_partitions] {
        if info.ignore() {
            continue;
        }
        assert_ne!(info.left, info.right);
        // The small side always gets subtracted from the big side.
        let (to, from) = if small_side_indexes[info.left as usize] != 0 {
            assert_eq!(small_side_indexes[info.right as usize], 0);
            (info.right as usize, info.left as usize)
        } else {
            assert_eq!(small_side_indexes[info.left as usize], 0);
            (info.left as usize, info.right as usize)
        };
        assert!(w_all[to].count() >= w_all[from].count());

        let mut has_zero = false;
        for bucket in bucket_range.clone() {
            has_zero = has_zero || bucket == 0;
            let from_w = all_partition_buckets[from * num_active_buckets + bucket as usize];
            all_partition_buckets[to * num_active_buckets + bucket as usize] -= from_w;
        }
        // Bucket zero also updates wAll.
        if has_zero {
            let from_w = w_all[from];
            w_all[to] -= from_w;
        }
    }
}

/// When a constraint of a kernel's registration holds: before the call, after it, or both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    Pre,
    Post,
    Both,
}

/// A constraint between a kernel's sizes and its arguments.
#[derive(Clone, Copy, Debug)]
pub struct Constraint {
    pub when: When,
    pub lhs: &'static str,
    pub op: &'static str,
    pub rhs: &'static str,
    pub description: &'static str,
}

/// A kernel's registration: its grid dimensions (name, range), and its parameters (name, access, type).
#[derive(Clone, Copy, Debug)]
pub struct KernelSpec {
    pub name: &'static str,
    pub dimensions: &'static [(&'static str, &'static str)],
    pub parameters: &'static [(&'static str, &'static str, &'static str)],
    pub constraints: &'static [Constraint],
}

const fn c(when: When, lhs: &'static str, op: &'static str, rhs: &'static str) -> Constraint {
    Constraint { when, lhs, op, rhs, description: "" }
}

const fn cd(when: When, lhs: &'static str, op: &'static str, rhs: &'static str, description: &'static str) -> Constraint {
    Constraint { when, lhs, op, rhs, description }
}

use When::{Both, Post, Pre};

/// The host kernels' registrations, as randomforest_kernels.cc made them (`doNothing` included). `testFeature`'s
/// constraint names a `numFuckingBuckets` field that `TreeDepthInfo` does not have, and no `treeDepthInfo`
/// parameter: as upstream.
pub const HOST_KERNELS: &[KernelSpec] = &[
    KernelSpec { name: "doNothing", dimensions: &[], parameters: &[], constraints: &[] },
    KernelSpec {
        name: "decodeRows",
        dimensions: &[("r", "numRows")],
        parameters: &[
            ("rowData", "r", "u64[rowDataLength]"),
            ("rowDataLength", "r", "u32"),
            ("weightBits", "r", "u16"),
            ("exampleNumBits", "r", "u16"),
            ("numRows", "r", "u32"),
            ("weightFormat", "r", "WeightFormat"),
            ("weightMultiplier", "r", "f32"),
            ("weightData", "r", "f32[weightDataLength]"),
            ("decodedRowsOut", "w", "f32[numRows]"),
        ],
        constraints: &[],
    },
    KernelSpec {
        name: "testFeature",
        dimensions: &[("featureIdx", "naf"), ("rowNum", "nr")],
        parameters: &[
            ("decodedRows", "r", "f32[numRows]"),
            ("numRows", "r", "u32"),
            ("bucketData", "r", "u32[bucketDataLength]"),
            ("bucketDataOffsets", "r", "u32[nf + 1]"),
            ("bucketNumbers", "r", "u32[nf]"),
            ("bucketEntryBits", "r", "u32[nf]"),
            ("activeFeatureList", "r", "u32[naf]"),
            ("partitionBuckets", "rw", "W32[numBuckets]"),
        ],
        constraints: &[c(Both, "numBuckets", "==", "readArrayElement(treeDepthInfo, 0).numFuckingBuckets")],
    },
    KernelSpec {
        name: "getPartitionSplits",
        dimensions: &[("f", "nf")],
        parameters: &[
            ("treeTrainingInfo", "r", "TreeTrainingInfo[=1]"),
            ("bucketNumbers", "r", "u32[nf]"),
            ("activeFeatureList", "r", "u32[nf]"),
            ("featureIsOrdinal", "r", "u32[nf]"),
            ("buckets", "r", "W32[numActiveBuckets * nap]"),
            ("wAll", "r", "W32[nap]"),
            ("featurePartitionSplitsOut", "w", "PartitionSplit[np * nf]"),
            ("treeDepthInfo", "r", "TreeDepthInfo[=1]"),
        ],
        constraints: &[c(Pre, "nap", "==", "readArrayElement(treeDepthInfo, 0).numActivePartitions"), c(Post, "nap", "==", "readArrayElement(treeDepthInfo, 0).numActivePartitions")],
    },
    KernelSpec {
        name: "bestPartitionSplit",
        dimensions: &[],
        parameters: &[
            ("treeTrainingInfo", "r", "TreeTrainingInfo[1]"),
            ("treeDepthInfo", "r", "TreeDepthInfo[1]"),
            ("activeFeatureList", "r", "u32[numFeatures]"),
            ("featurePartitionSplits", "r", "PartitionSplit[np * numActiveFeatures]"),
            ("partitionIndexes", "r", "PartitionIndex[nap]"),
            ("allPartitionSplitsOut", "w", "IndexedPartitionSplit[nap]"),
        ],
        constraints: &[c(Pre, "nap", "==", "readArrayElement(treeDepthInfo, 0).numActivePartitions"), c(Post, "nap", "==", "readArrayElement(treeDepthInfo, 0).numActivePartitions")],
    },
    KernelSpec {
        name: "assignPartitionNumbers",
        dimensions: &[],
        parameters: &[
            ("treeTrainingInfo", "r", "TreeTrainingInfo[1]"),
            ("treeDepthInfo", "rw", "TreeDepthInfo[1]"),
            ("allPartitionSplits", "r", "IndexedPartitionSplit[numActivePartitions]"),
            ("partitionIndexesOut", "w", "PartitionIndex[newNumActivePartitions]"),
            ("partitionInfoOut", "w", "PartitionInfo[numActivePartitions]"),
            ("smallSideIndexesOut", "w", "u8[newNumActivePartitions]"),
            ("smallSideIndexToPartitionOut", "w", "u16[nssi2p]"),
        ],
        constraints: &[
            c(Pre, "numActivePartitions", "==", "readArrayElement(treeDepthInfo, 0).numActivePartitions"),
            c(Post, "numActivePartitions", "==", "readArrayElement(treeDepthInfo, 0).prevNumActivePartitions"),
            c(Post, "newNumActivePartitions", "==", "readArrayElement(treeDepthInfo, 0).numActivePartitions"),
            c(Pre, "nssi2p", "==", "256"),
            c(Post, "nssi2p", "==", "min(256, newNumActivePartitions/2)"),
        ],
    },
    KernelSpec {
        name: "clearBuckets",
        dimensions: &[("b", "numActiveBuckets")],
        parameters: &[
            ("treeTrainingInfo", "r", "TreeTrainingInfo[=1]"),
            ("treeDepthInfo", "rw", "TreeDepthInfo[=1]"),
            ("bucketsOut", "w", "W32[numActiveBuckets * numActivePartitions]"),
            ("wAllOut", "rw", "W32[numActivePartitions]"),
            ("numNonZeroDirectionIndices", "w", "u32[=1]"),
            ("smallSideIndexes", "r", "u8[newNumActivePartitions]"),
        ],
        constraints: &[c(Pre, "numActivePartitions", "==", "readArrayElement(treeDepthInfo, 0).prevNumActivePartitions"), c(Post, "numActivePartitions", "==", "readArrayElement(treeDepthInfo, 0).prevNumActivePartitions")],
    },
    KernelSpec {
        name: "updatePartitionNumbers",
        dimensions: &[("r", "numRows")],
        parameters: &[
            ("treeTrainingInfo", "r", "TreeTrainingInfo[1]"),
            ("treeDepthInfo", "rw", "TreeDepthInfo[1]"),
            ("partitions", "r", "RowPartitionInfo[numRows]"),
            ("directions", "w", "u32[(numRows+31)/32]"),
            ("numNonZeroDirectionIndices", "rw", "u32[1]"),
            ("nonZeroDirectionIndices", "w", "UpdateWorkEntry[nndi:unordered]"),
            ("smallSideIndexes", "r", "u8[numActivePartitions]"),
            ("allPartitionSplits", "r", "IndexedPartitionSplit[numActivePartitions]"),
            ("partitionInfo", "r", "PartitionInfo[numActivePartitions]"),
            ("bucketData", "r", "u32[bucketDataLength]"),
            ("bucketDataOffsets", "r", "u32[nf + 1]"),
            ("bucketNumbers", "r", "u32[nf]"),
            ("bucketEntryBits", "r", "u32[nf]"),
            ("featureIsOrdinal", "r", "u32[nf]"),
            ("decodedRows", "r", "f32[nr]"),
        ],
        constraints: &[
            c(Pre, "numActivePartitions", "==", "readArrayElement(treeDepthInfo, 0).prevNumActivePartitions"),
            c(Post, "numActivePartitions", "==", "readArrayElement(treeDepthInfo, 0).prevNumActivePartitions"),
            cd(Pre, "nndi", "==", "numRows/2 + 2", "Loose sizing pre-call"),
            cd(Post, "nndi", "==", "readArrayElement(numNonZeroDirectionIndices, 0)", "Tight sizing post-call"),
        ],
    },
    KernelSpec {
        name: "updateBuckets",
        dimensions: &[("r", "numRows"), ("fidx", "naf")],
        parameters: &[
            ("treeTrainingInfo", "r", "TreeTrainingInfo[1]"),
            ("treeDepthInfo", "rw", "TreeDepthInfo[1]"),
            ("partitions", "r", "RowPartitionInfo[numRows]"),
            ("directions", "r", "u8[(numRows+31)/32]"),
            ("numNonZeroDirectionIndices", "rw", "u32[1]"),
            ("nonZeroDirectionIndices", "r", "UpdateWorkEntry[nndi:unordered]"),
            ("buckets", "w", "W32[numActiveBuckets * newNumActivePartitions]"),
            ("wAll", "rw", "W32[newNumActivePartitions]"),
            ("smallSideIndexes", "r", "u8[numActivePartitions]"),
            ("smallSideIndexToPartition", "r", "u16[min(256,newNumActivePartitions/2)]"),
            ("decodedRows", "r", "f32[nr]"),
            ("bucketData", "r", "u32[bucketDataLength]"),
            ("bucketDataOffsets", "r", "u32[nf + 1]"),
            ("bucketNumbers", "r", "u32[nf]"),
            ("bucketEntryBits", "r", "u32[nf]"),
            ("activeFeatureList", "r", "u32[numActiveFeatures]"),
            ("featureIsOrdinal", "r", "u32[nf]"),
        ],
        constraints: &[
            c(Both, "nndi", "==", "readArrayElement(numNonZeroDirectionIndices, 0)"),
            c(Both, "numActivePartitions", "==", "readArrayElement(treeDepthInfo, 0).prevNumActivePartitions"),
            c(Both, "newNumActivePartitions", "==", "readArrayElement(treeDepthInfo, 0).numActivePartitions"),
        ],
    },
    KernelSpec {
        name: "fixupBuckets",
        dimensions: &[("bucket", "numActiveBuckets")],
        parameters: &[
            ("treeTrainingInfo", "r", "TreeTrainingInfo[=1]"),
            ("treeDepthInfo", "rw", "TreeDepthInfo[=1]"),
            ("buckets", "rw", "W32[numActiveBuckets * numActivePartitions]"),
            ("wAll", "rw", "W32[newNumActivePartitions]"),
            ("partitionInfo", "r", "PartitionInfo[oldNumActivePartitions]"),
            ("smallSideIndexes", "r", "u8[oldNumActivePartitions]"),
        ],
        constraints: &[
            c(Both, "oldNumActivePartitions", "==", "readArrayElement(treeDepthInfo, 0).prevNumActivePartitions"),
            c(Both, "newNumActivePartitions", "==", "readArrayElement(treeDepthInfo, 0).numActivePartitions"),
            c(Pre, "numActivePartitions", "==", "oldNumActivePartitions"),
            c(Post, "numActivePartitions", "==", "newNumActivePartitions"),
        ],
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bucket::WritableBucketList;
    use crate::memory_region::MemorySerializer;
    use crate::randomforest_types::{RowWriter, WeightEncoder};

    #[test]
    fn sqrt3_is_exact_and_scores_pure_splits_zero() {
        for v in [0.0f32, 1e-6, 0.25, 0.3, 1.0, 1.5] {
            let s = sqrt3(v) / SQRT3_MULT_AFTER;
            let scaled = v * SQRT3_MULT_BEFORE;
            assert!(s * s <= scaled && (v == 0.0 || s.next_up() * s.next_up() > scaled), "{v}");
        }
        let mut pure = W::default();
        pure.add_weight(true, 0.5);
        assert_eq!(score_split(&pure, &pure, SplitCriterion::Sqrt), 0.0);
        // A pure node's split doesn't lower its error: none.
        assert_eq!(score_split(&pure, &pure, SplitCriterion::Variance), f32::INFINITY);
        let mut other = W::default();
        other.add_weight(false, 0.5);
        assert_eq!(score_split(&pure, &other, SplitCriterion::Variance), 0.0);
        assert_eq!(score_split(&pure, &other, SplitCriterion::Sqrt.with_min_leaf(2)), f32::INFINITY);
        assert_eq!(HOST_KERNELS.len(), 10);
    }

    /// One level of training through the nine kernels' host forms: the root's best split is the feature that
    /// separates the labels, and the children's buckets (after the small side's transfer and the big side's
    /// fixup) are what summing each child's rows from the beginning gives.
    #[test]
    fn one_level_through_the_host_kernels() {
        let mut s = MemorySerializer;
        let nr = 64u32;
        // Feature 0 (ordinal, 8 buckets): row % 8; the label is mostly bucket >= 3 (rows 0 and 26 are true too), so
        // the best split is feature 0 at 3, with a mixed left side (a split pure on both sides finishes its
        // partition instead). Feature 1 (categorical, 3 buckets): row % 3, unrelated to the label.
        let label = |r: u32| r % 8 >= 3 || r % 13 == 0;
        let feature_nb = [8u32, 3];
        let bucket_of = |f: usize, r: u32| if f == 0 { r % 8 } else { r % 3 };

        // Rows: label, weight 1/128 (W32 holds totals below 2), sequential example numbers.
        let enc = WeightEncoder::default();
        let mut w = RowWriter::new(&enc, nr as usize, nr as usize, &mut s, true, 1.0);
        for r in 0..nr {
            w.add_row_values(label(r), 1.0 / 128.0, r).unwrap();
        }
        let rows = w.freeze(&mut s);
        let decoded = decode_rows(&rows);
        let mut via_kernel = vec![[0.0; 2]; nr as usize];
        decode_rows_kernel_cpu(0..nr, rows.row_data.clone(), rows.row_data.length() as u32, 32, 0, nr, WeightFormat::WfFloat, 0.0, Default::default(), &[], &mut via_kernel);
        assert_eq!(decoded, via_kernel);

        // The features' bucket data, back to back.
        let mut all_bucket_data: Vec<u32> = Vec::new();
        let (mut offsets, mut numbers, mut entry_bits) = (vec![0u32], vec![0u32], vec![]);
        for (f, &nb) in feature_nb.iter().enumerate() {
            let mut bl = WritableBucketList::new(nr as usize, nb, &mut s);
            for r in 0..nr {
                bl.write(bucket_of(f, r));
            }
            let frozen = bl.freeze(&mut s);
            entry_bits.push(frozen.entry_bits as u32);
            all_bucket_data.extend_from_slice(frozen.storage.data());
            offsets.push(all_bucket_data.len() as u32);
            numbers.push(numbers.last().unwrap() + nb);
        }
        let total_buckets = *numbers.last().unwrap();
        let active = [0u32, 1];
        let ordinal = [1u32, 0];
        let max_parts = 4usize;
        let tti = [TreeTrainingInfo { num_rows: nr, max_depth: 4, num_active_features: 2, num_features: 2, max_num_active_partitions: max_parts as u16, num_active_buckets: total_buckets, max_num_finished_partitions: u32::MAX, ..Default::default() }];
        let mut tdi = [TreeDepthInfo::default()];

        // Depth 0: the root's buckets and wAll.
        let mut buckets = vec![W::default(); max_parts * total_buckets as usize];
        for fidx in 0..2 {
            test_feature_kernel_host(fidx, 2, 0..nr, &decoded, nr, &all_bucket_data, &offsets, &numbers, &entry_bits, &active, &mut buckets);
        }
        let mut w_all = vec![W::default(); max_parts];
        for &d in &decoded {
            w_all[0].add_weights(d);
        }

        let mut splits = vec![PartitionSplit::default(); max_parts * 2];
        for fidx in 0..2 {
            get_partition_splits_kernel(fidx, 2, &tti, &numbers, &active, &ordinal, &buckets, &w_all, &mut splits, &tdi);
        }
        let mut all_splits = vec![IndexedPartitionSplit::default(); 16];
        best_partition_split_kernel(&tti, &tdi, &active, &splits, &[PartitionIndex::root()], &mut all_splits);
        let best = all_splits[0];
        assert_eq!(best.index, PartitionIndex::root());
        assert_eq!((best.split.feature, best.split.value), (0, 3));
        assert!(best.split.right.uniform() && !best.split.left.uniform());
        assert_eq!(best.split.left.count() + best.split.right.count(), nr);

        let mut indexes = vec![PartitionIndex::none(); max_parts];
        let mut info = vec![PartitionInfo::default(); max_parts];
        let mut ssi = vec![0u8; max_parts];
        let mut ssi2p = vec![0u16; 256];
        assign_partition_numbers_kernel(&tti, &mut tdi, &all_splits, &mut indexes, &mut info, &mut ssi, &mut ssi2p);
        assert_eq!((tdi[0].depth, tdi[0].num_active_partitions, tdi[0].prev_num_active_partitions), (1, 2, 1));
        let mut children = [indexes[0], indexes[1]];
        children.sort();
        assert_eq!(children, [PartitionIndex::root().left_child(), PartitionIndex::root().right_child()]);

        let mut nndi = [0u32];
        clear_buckets_kernel(0..total_buckets, &tti, &tdi, &mut buckets, &mut w_all, &mut nndi, &ssi);
        let mut partitions = vec![RowPartitionInfo::default(); nr as usize];
        let mut work = vec![UpdateWorkEntry::default(); nr as usize / 2 + 2];
        update_partition_numbers_kernel(0..nr, &tti, &tdi, &mut partitions, &mut [], &mut nndi, &mut work, &ssi, &all_splits, &info, &all_bucket_data, &offsets, &numbers, &entry_bits, &ordinal, &decoded);
        assert_eq!(nndi[0], best.split.left.count().min(best.split.right.count()));
        for fidxp1 in 0..=2 {
            update_buckets_kernel(0..nr, fidxp1, 3, &tti, &tdi, &partitions, &[], &nndi, &work, &mut buckets, &mut w_all, &ssi, &ssi2p, &decoded, &all_bucket_data, &offsets, &numbers, &entry_bits, &active, &ordinal);
        }
        fixup_buckets_kernel(0..total_buckets, &tti, &tdi, &mut buckets, &mut w_all, &info, &ssi);

        // Each child's buckets and wAll, summed from the beginning.
        for p in 0..2usize {
            let mut expect = vec![W::default(); total_buckets as usize];
            let mut expect_all = W::default();
            for r in 0..nr {
                if partitions[r as usize].partition() as usize != p {
                    continue;
                }
                let d = decoded[r as usize];
                expect_all.add_weights(d);
                for f in 0..2 {
                    expect[(numbers[f] + bucket_of(f, r)) as usize].add_weights(d);
                }
            }
            assert_eq!(w_all[p], expect_all, "partition {p}: wAll");
            assert_eq!(&buckets[p * total_buckets as usize..(p + 1) * total_buckets as usize], &expect[..], "partition {p}: buckets");
        }
    }
}
