//! randomforest_recursive.cc, in Rust.
//!
//! Jeremy Barnes, 13 October 2018. Copyright (c) 2018 mldb.ai inc. This file is part of MLDB. Copyright 2016 mldb.ai
//! inc. Apache-2.0 (see the LICENSE at `accel/src/forest/mldb/`).
//!
//! Recursive (CPU) kernels for the random forest algorithm: the core of the split search
//! (`testFeatureKernel`, `chooseSplitKernel`, `testFeatureNumber`) and `decodeRows`, which the kernels' host forms
//! (randomforest_kernels.rs) call, `extractTree`, which builds a tree from the partitioned trainer's splits, and the
//! partitioned trainer on the CPU: breadth first a level at a time (`getPartitionSplits`, `updateBuckets`), small
//! partitions depth first (`trainSmallPartitions`, `descendSmallPartition`), and the recursion per partition
//! (`splitPartitions`, `splitAndRecursePartitioned`, `trainPartitionedRecursive`); `trainPartitionedEndToEndCpu`
//! from the root, and `testAll` for the recursive trainer. The partitioned trainer is also where the device trainer
//! continues a tree deeper than its levels.
//!
//! Departures from upstream, each a fault there: rows in no partition are skipped by `splitPartitions` (upstream
//! indexed its output with the "no partition" number); the bucket matrix is the width of the level (upstream doubled
//! it every level, to 2^20 partitions' worth); the partitions beyond the maximum width are the smallest and are
//! split off whole (upstream cut sides before sorting them, and could split a partition's two sides); subtrees
//! trained in parallel each have their own tree, grafted after (upstream shared one between threads). The unused
//! `bucketMemory` argument is not carried.
//!
//! The C++ template over row iterators and bucket lists is two traits: `RowSource` (a row iterator, or decoded rows)
//! and `BucketSource` (a bucket list, a view of one, or a plain `u16`/`u32` array). The `debug` traces are not
//! brought over.

use crate::forest::bucket::{BucketList, BucketListRef, WritableBucketList};
use crate::forest::memory_region::MemorySerializer;
use crate::forest::randomforest_kernels::score_split;
use crate::forest::dataset_feature_space::DatasetFeatureSpace;
use crate::forest::randomforest_types::{decode_row_arr, decode_row_iter, get_leaf, get_node, Feature, IndexedPartitionSplit, PartitionEntry, PartitionIndex, PartitionSplit, PartitionSplitDirection, RowIterator, RowPartitionInfo, RowWeights, Rows, SplitCriterion, W, CategorySet, MAX_CATEGORICAL_BUCKETS, in_category_set};
use crate::forest::tree::{Ptr, Tree};
use std::collections::BTreeMap;

/// `decodeRow` over what the split search reads its rows from.
pub trait RowSource {
    /// The row's example number and weights.
    fn decode_row(&mut self, row_number: usize) -> (u32, RowWeights);
}

impl RowSource for RowIterator<'_> {
    #[inline(always)]
    fn decode_row(&mut self, row_number: usize) -> (u32, RowWeights) {
        decode_row_iter(self, row_number)
    }
}

impl RowSource for &[RowWeights] {
    #[inline(always)]
    fn decode_row(&mut self, row_number: usize) -> (u32, RowWeights) {
        decode_row_arr(self, row_number)
    }
}

/// `buckets[i]` over what the split search reads its buckets from.
pub trait BucketSource {
    fn bucket(&self, i: u32) -> u32;
}

impl BucketSource for BucketList {
    #[inline(always)]
    fn bucket(&self, i: u32) -> u32 {
        self.get(i)
    }
}

impl BucketSource for BucketListRef<'_> {
    #[inline(always)]
    fn bucket(&self, i: u32) -> u32 {
        self.get(i)
    }
}

impl BucketSource for [u16] {
    #[inline(always)]
    fn bucket(&self, i: u32) -> u32 {
        self[i as usize] as u32
    }
}

impl BucketSource for [u32] {
    #[inline(always)]
    fn bucket(&self, i: u32) -> u32 {
        self[i as usize]
    }
}

/// Core kernel of the decision tree search algorithm. Transfer the example weight into the appropriate
/// (bucket, label) accumulator. Returns whether or not the feature is still active, and the highest bucket seen.
pub fn test_feature_kernel_cpu<R: RowSource, B: BucketSource + ?Sized>(mut row_iterator: R, num_rows: usize, buckets: &B, w: &mut [W]) -> (bool, i32) {
    // Minimum bucket number we've seen.
    let mut min_bucket = i32::MAX;
    // Maximum bucket number we've seen. Can significantly reduce the work required to search the buckets later on,
    // as those without an example have no possible split point.
    let mut max_bucket = i32::MIN;
    for j in 0..num_rows {
        let (example_num, weights) = row_iterator.decode_row(j);
        let bucket = buckets.bucket(example_num) as i32;
        w[bucket as usize].add_weights(weights);
        max_bucket = max_bucket.max(bucket);
        min_bucket = min_bucket.min(bucket);
    }
    (min_bucket < max_bucket, max_bucket)
}

/// `testFeatureKernel`: upstream's overloads all run the CPU form.
pub fn test_feature_kernel<R: RowSource, B: BucketSource + ?Sized>(row_iterator: R, num_rows: usize, buckets: &B, w: &mut [W]) -> (bool, i32) {
    test_feature_kernel_cpu(row_iterator, num_rows, buckets, w)
}

/// Chooses which is the best split for a given feature: (best score, best split, best left, best right, the
/// categorical split's subset).
///
/// A categorical feature's split is upstream's (each bucket alone against the rest; the split's value is the
/// bucket), or with `subsets` CART's (rerail's extension): its buckets ranked by their mean target
/// (`category_order`), the best prefix of that order found as an ordinal split is, and the prefix is the subset on
/// the left (the split's value is its number of buckets). With two labels or a variance, and a concave impurity
/// (both criteria), the best subset is such a prefix (Breiman et al. 1984, 9.4).
pub fn choose_split_kernel(w: &[W] /* at least max_bucket + 1 entries */, max_bucket: i32, ordinal: bool, subsets: bool, w_all: &W, criterion: SplitCriterion) -> (f64, i32, W, W, CategorySet) {
    if ordinal {
        let (s, b, l, r) = choose_ordinal_split(w, max_bucket, w_all, criterion);
        return (s, b, l, r, [0; 16]);
    }
    if !subsets {
        let (s, b, l, r) = choose_one_vs_rest_split(w, max_bucket, w_all, criterion);
        return (s, b, l, r, [0; 16]);
    }
    let n = (max_bucket + 1).max(0) as usize;
    assert!(n <= MAX_CATEGORICAL_BUCKETS, "a categorical feature with {n} buckets");
    let order = category_order(&w[..n]);
    let ranked: Vec<W> = order.iter().map(|&b| w[b as usize]).collect();
    let (s, k, l, r) = choose_ordinal_split(&ranked, max_bucket, w_all, criterion);
    let mut subset = [0u32; 16];
    for &b in &order[..k.max(0) as usize] {
        subset[(b >> 5) as usize] |= 1 << (b & 31);
    }
    (s, k, l, r, subset)
}

/// A categorical feature's buckets in the order of their mean target (the true side's share of the weight), then
/// the empty buckets; ties by bucket number. Shares compare exactly, in the accumulators' fixed point
/// (`t_a (f_b + t_b) < t_b (f_a + t_a)` in i64), as the device's `chooseSplit` ranks them.
pub fn category_order(w: &[W]) -> Vec<u32> {
    let mut order: Vec<u32> = (0..w.len() as u32).collect();
    order.sort_by(|&a, &b| category_before(&w[a as usize], a, &w[b as usize], b));
    order
}

/// `category_order`'s comparison.
#[inline]
pub fn category_before(wa: &W, a: u32, wb: &W, b: u32) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (wa.count() == 0, wb.count() == 0) {
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        (true, true) => return a.cmp(&b),
        _ => {}
    }
    let (fa, ta) = (wa.v[0].rep as i64, wa.v[1].rep as i64);
    let (fb, tb) = (wb.v[0].rep as i64, wb.v[1].rep as i64);
    (ta * (fb + tb)).cmp(&(tb * (fa + ta))).then(a.cmp(&b))
}

/// Upstream's categorical split: the best bucket alone (on the left) against the rest: (best score, the bucket,
/// left, right).
fn choose_one_vs_rest_split(w: &[W], max_bucket: i32, w_all: &W, criterion: SplitCriterion) -> (f64, i32, W, W) {
    let mut best_score = f64::INFINITY;
    let mut best_split = -1;
    let mut best_left = W::default();
    let mut best_right = W::default();
    // Calculate best split point for non-ordered values. Now test split points one by one.
    for j in 0..=max_bucket.max(-1) {
        let wj = w[j as usize];
        if wj.empty() {
            continue;
        }
        let w_false = *w_all - wj;
        if w_false.count() == 0 || wj.count() == 0 {
            continue;
        }
        let s = score_split(&w_false, &wj, criterion) as f64;
        if s < best_score {
            best_score = s;
            best_split = j;
            best_right = w_false;
            best_left = wj;
        }
    }
    (best_score, best_split, best_left, best_right)
}

/// The best split of buckets in order: (best score, the number of buckets on the left, left, right).
fn choose_ordinal_split(w: &[W], max_bucket: i32, w_all: &W, criterion: SplitCriterion) -> (f64, i32, W, W) {
    let mut best_score = f64::INFINITY;
    let mut best_split = -1;
    let mut best_left = W::default();
    let mut best_right = W::default();

    {
        // Calculate best split point for ordered values.
        let mut w_false = *w_all;
        let mut w_true = W::default();
        // Now test split points one by one.
        for j in 0..=max_bucket.max(-1) {
            if w_false.count() > 0 && w_true.count() > 0 {
                let s = score_split(&w_false, &w_true, criterion) as f64;
                if s < best_score {
                    best_score = s;
                    best_split = j;
                    best_right = w_false;
                    best_left = w_true;
                }
            }
            if j < max_bucket && w[j as usize].count() != 0 {
                w_false -= w[j as usize];
                w_true += w[j as usize];
            }
        }
    }
    (best_score, best_split, best_left, best_right)
}

/// Tests one feature over the rows: (best score, best split, best left, best right, feature is still active, the
/// categorical split's subset).
pub fn test_feature_number(feature_num: usize, features: &[Feature], row_iterator: RowIterator<'_>, num_rows: usize, w_all: &W, criterion: SplitCriterion) -> (f64, i32, W, W, bool, CategorySet) {
    let feature = &features[feature_num];
    let buckets = &feature.buckets;
    let nb = buckets.num_buckets.max(0) as usize;
    let mut w = vec![W::default(); nb];
    if !feature.active {
        return (f64::INFINITY, -1, W::default(), W::default(), false, [0; 16]);
    }
    // Is the feature still active?
    let (is_active, max_bucket) = test_feature_kernel(row_iterator, num_rows, buckets, &mut w);
    if is_active {
        let (s, b, l, r, subset) = choose_split_kernel(&w, max_bucket, feature.ordinal, feature.subsets, w_all, criterion);
        return (s, b, l, r, true, subset);
    }
    (f64::INFINITY, -1, W::default(), W::default(), false, [0; 16])
}

/// Decodes the rows to their weights to false and true (`RowWeights`; upstream had one float, the weight negated
/// for a true label, which has no room for a regression target).
pub fn decode_rows(rows: &Rows) -> Vec<RowWeights> {
    let mut decoded_rows = vec![[0.0; 2]; rows.row_count()];
    let mut it = rows.get_row_iterator();
    for (i, d) in decoded_rows.iter_mut().enumerate() {
        let row = it.get_decoded_row();
        assert_eq!(i as u32, row.example_num);
        *d = row.weights();
    }
    decoded_rows
}

/// Builds the tree under `root` from the splits and leaves of a partitioned training: a leaf where there is one, else
/// a node from the split (its children recursively, or leaves from the split's sides where they cannot recurse).
/// There is no calculating going on here, just creation of the data structure.
#[allow(clippy::too_many_arguments)]
pub fn extract_tree(depth: i32, max_depth: i32, tree: &mut Tree, root: PartitionIndex, splits: &BTreeMap<PartitionIndex, PartitionSplit>, leaves: &BTreeMap<PartitionIndex, Ptr>, features: &[Feature], fs: &DatasetFeatureSpace) -> Result<Ptr, String> {
    assert_eq!(root.depth(), depth);
    // First look for a leaf.
    if let Some(&l) = leaves.get(&root) {
        return Ok(l);
    }
    // Secondly look for a split.
    let Some(s) = splits.get(&root) else {
        return Ok(Ptr::Null);
    };
    if s.left.count() + s.right.count() == 0 {
        return Ok(Ptr::Null);
    } else if s.feature == -1 {
        return Ok(get_leaf(tree, &(s.left + s.right)));
    }
    let total = s.left + s.right;
    // No impurity.
    if total.v[0].to_f32() == 0.0 || total.v[1].to_f32() == 0.0 {
        return Ok(Ptr::Null);
    }
    let mut left = extract_tree(depth + 1, max_depth, tree, root.left_child(), splits, leaves, features, fs)?;
    let mut right = extract_tree(depth + 1, max_depth, tree, root.right_child(), splits, leaves, features, fs)?;
    // Fill in from the information in the split if we can't recurse.
    if !left.is_some() {
        left = get_leaf(tree, &s.left);
    }
    if !right.is_some() {
        right = get_leaf(tree, &s.right);
    }
    get_node(tree, s.score, s.feature as i32, s.value as i32, &s.subset, left, right, s.left, s.right, features, fs)
}

/// The best split over every active feature, the rows read from the row iterator: (score, feature, split, left,
/// right, which features are still active). Features are searched in parallel near the root, and reduced in order
/// so that the choice is deterministic (the first feature with the lowest score).
pub fn test_all_cpu(depth: i32, features: &[Feature], rows: &Rows, criterion: SplitCriterion) -> (f64, i32, i32, W, W, Vec<u8>, CategorySet) {
    let mut new_active: Vec<u8> = features.iter().map(|f| f.active as u8).collect();
    // We have no impurity in our bucket. Time to stop.
    if rows.w_all.v[0].to_f32() == 0.0 || rows.w_all.v[1].to_f32() == 0.0 {
        return (1.0, -1, -1, rows.w_all, W::default(), vec![0; features.len()], [0; 16]);
    }
    let nf = features.len();
    let do_feature = |i: usize| test_feature_number(i, features, rows.get_row_iterator(), rows.row_count(), &rows.w_all, criterion);
    let results: Vec<(f64, i32, W, W, bool, CategorySet)> = if depth < 4 || rows.row_count() as u64 * nf as u64 > 20000 {
        use rayon::prelude::*;
        (0..nf).into_par_iter().map(do_feature).collect()
    } else {
        (0..nf).map(do_feature).collect()
    };
    let (mut best_score, mut best_feature, mut best_split, mut best_left, mut best_right, mut best_subset) = (f64::INFINITY, -1, -1, W::default(), W::default(), [0; 16]);
    for (i, (score, split, left, right, still_active, subset)) in results.into_iter().enumerate() {
        new_active[i] = still_active as u8;
        if score < best_score {
            (best_score, best_feature, best_split, best_left, best_right, best_subset) = (score, i as i32, split, left, right, subset);
        }
    }
    (best_score, best_feature, best_split, best_left, best_right, new_active, best_subset)
}

/// `testAll`: upstream's only form is the CPU one.
pub fn test_all(depth: i32, features: &[Feature], rows: &Rows, criterion: SplitCriterion) -> (f64, i32, i32, W, W, Vec<u8>, CategorySet) {
    test_all_cpu(depth, features, rows, criterion)
}

/// Which side of a split a feature's bucket is on: `true` is right (ordinal: at or above the split value;
/// categorical: not the split value, or with subsets not in the split's subset).
#[inline(always)]
pub fn side_of(bucket: u32, split_value: i32, subset: &CategorySet, f: &Feature) -> bool {
    if f.ordinal {
        bucket as i32 >= split_value
    } else if f.subsets {
        !in_category_set(subset, bucket)
    } else {
        bucket as i32 != split_value
    }
}

/// Moves each partition's buckets and wAll to its new place, then gives each row its new partition and moves the
/// weight of rows on the small side of their split (the transfer direction) into the small side's buckets, and
/// finally subtracts the small sides from the big sides. Only the rows on the wrong side are touched, which is
/// less work for unbalanced splits (which are typically most of them, especially for discrete buckets).
///
/// `new_partition_numbers[i]` is the (left, right) partition of partition `i`'s children, -1 for none. On return
/// `buckets` holds `new_num_partitions` partitions (upstream doubled the matrix every level whatever the width, so
/// that it reached 2^20 partitions' worth before the recursion limit).
#[allow(clippy::too_many_arguments)]
pub fn update_buckets(features: &[Feature], partitions: &mut [RowPartitionInfo], buckets: &mut Vec<W>, num_active_buckets: u32, w_all: &mut Vec<W>, bucket_offsets: &[u32], partition_splits: &[IndexedPartitionSplit], new_partition_numbers: &[(i32, i32)], new_num_partitions: usize, decoded_rows: &[RowWeights], active_features: &[i32]) {
    let num_partitions = w_all.len();
    if num_partitions == 0 {
        return;
    }
    assert_eq!(num_partitions, partition_splits.len());
    assert_eq!(num_partitions, new_partition_numbers.len());
    let nab = num_active_buckets as usize;
    let rl = |s: &IndexedPartitionSplit| s.split.transfer_direction() == PartitionSplitDirection::RL;

    // Distribute old ones to their new place.
    let mut new_buckets = vec![W::default(); new_num_partitions * nab];
    let mut new_w_all = vec![W::default(); new_num_partitions];
    for i in 0..num_partitions {
        let (l, r) = new_partition_numbers[i];
        let s = &partition_splits[i];
        if l == -1 || s.split.left.count() == 0 || s.split.right.count() == 0 {
            continue;
        }
        let to = if rl(s) { r } else { l } as usize;
        new_buckets[to * nab..(to + 1) * nab].copy_from_slice(&buckets[i * nab..(i + 1) * nab]);
        new_w_all[to] = w_all[i];
    }
    *buckets = new_buckets;
    *w_all = new_w_all;

    for (i, p) in partitions.iter_mut().enumerate() {
        let partition = p.partition();
        // Example is not in a partition.
        if partition == RowPartitionInfo::max() {
            continue;
        }
        let s = &partition_splits[partition as usize];
        let split_feature = s.split.feature;
        if split_feature == -1 {
            // Reached a leaf here, nothing to split.
            *p = RowPartitionInfo::max().into();
            continue;
        }
        let (left_partition, right_partition) = new_partition_numbers[partition as usize];
        if left_partition == -1 && right_partition == -1 {
            *p = RowPartitionInfo::max().into();
            continue;
        }
        assert!(left_partition != -1 && right_partition != -1);
        let weights = decoded_rows[i];
        let f = &features[split_feature as usize];
        let side = side_of(f.buckets.get(i as u32), s.split.value as i32, &s.split.subset, f);
        *p = (if side { right_partition } else { left_partition } as u16).into();
        // false = left to right, true = right to left.
        let direction = rl(s);
        if direction != side {
            let to = if direction { left_partition } else { right_partition } as usize;
            w_all[to].add_weights(weights);
            for &af in active_features {
                let start = bucket_offsets[af as usize] as usize;
                let bucket = features[af as usize].buckets.get(i as u32) as usize;
                buckets[to * nab + start + bucket].add_weights(weights);
            }
        }
    }

    // Fix up all of the "from" buckets by subtracting the accumulated counts from the "to" buckets. This saves lots
    // of work since we only need to transfer the aggregate weight, not update row by row.
    for (i, s) in partition_splits.iter().enumerate() {
        let (l, r) = new_partition_numbers[i];
        if l == -1 || r == -1 {
            continue;
        }
        let (from, to) = if rl(s) { (r as usize, l as usize) } else { (l as usize, r as usize) };
        let t = w_all[to];
        w_all[from] -= t;
        for &af in active_features {
            for b in bucket_offsets[af as usize] as usize..bucket_offsets[af as usize + 1] as usize {
                let tb = buckets[to * nab + b];
                if tb.count() == 0 {
                    continue;
                }
                buckets[from * nab + b] -= tb;
            }
        }
    }
}

/// Splits the rows into one dataset per partition (`w[i]` its weight, `indexes[i]` its place in the tree): each
/// partition's decoded rows, in order, and its active features' buckets re-encoded over just those rows. Rows in no
/// partition, and partitions with no rows or no index, are left out (upstream indexed its output with the "no
/// partition" number for rows a leaf had claimed).
pub fn split_partitions(features: &[Feature], active_features: &[i32], decoded_rows: &[RowWeights], partitions: &[RowPartitionInfo], w: &[W], indexes: &[PartitionIndex]) -> Vec<PartitionEntry> {
    let num_partitions = w.len();
    let mut s = MemorySerializer;
    let keep = |p: usize| p < num_partitions && w[p].count() > 0 && indexes[p] != PartitionIndex::none();
    let mut out: Vec<PartitionEntry> = (0..num_partitions).map(|_| PartitionEntry::default()).collect();
    for (i, e) in out.iter_mut().enumerate() {
        if !keep(i) {
            continue;
        }
        e.decoded_rows.reserve(w[i].count() as usize);
        e.active_features = active_features.to_vec();
        e.active_feature_set = active_features.iter().copied().collect();
        e.index = indexes[i];
        e.features = features.iter().enumerate().map(|(f, feat)| Feature { active: e.active_feature_set.contains(&(f as i32)), ordinal: feat.ordinal, subsets: feat.subsets, info: feat.info, buckets: BucketList::default() }).collect();
    }
    for (i, p) in partitions.iter().enumerate() {
        let p = p.partition() as usize;
        if !keep(p) {
            continue;
        }
        out[p].decoded_rows.push(decoded_rows[i]);
        let weight = decoded_rows[i][0] + decoded_rows[i][1];
        assert!(weight <= 1.0, "row {i} weight {weight}");
        out[p].w_all.add_weights(decoded_rows[i]);
    }
    for &f in active_features {
        let f = f as usize;
        let mut writers: Vec<Option<WritableBucketList>> = (0..num_partitions).map(|i| keep(i).then(|| WritableBucketList::new(w[i].count() as usize, features[f].buckets.num_buckets as u32, &mut s))).collect();
        for (j, p) in partitions.iter().enumerate() {
            if let Some(Some(wr)) = writers.get_mut(p.partition() as usize) {
                wr.write(features[f].buckets.get(j as u32));
            }
        }
        for (i, wr) in writers.iter_mut().enumerate() {
            if let Some(wr) = wr {
                out[i].features[f].buckets = wr.freeze(&mut s);
            }
        }
    }
    for i in 0..num_partitions {
        if keep(i) {
            assert_eq!(w[i], out[i].w_all, "partition {i}: its weight is not its rows' weight");
        }
    }
    out
}

/// Takes the W values of each bucket of a number of partitions, and calculates the optimal split feature and value
/// for each of the partitions. It's the "search" part of the decision tree algorithm, but across a full set of rows
/// split into multiple partitions.
#[allow(clippy::too_many_arguments)]
pub fn get_partition_splits(buckets: &[W] /* [np][nab] */, num_active_buckets: u32, active_feature_list: &[i32], bucket_offsets: &[u32] /* [nf + 1] */, features: &[Feature], w_all: &[W] /* [np] */, indexes: &[PartitionIndex], parallel: bool, criterion: SplitCriterion) -> Vec<IndexedPartitionSplit> {
    let num_partitions = w_all.len();
    assert_eq!(*bucket_offsets.last().unwrap(), num_active_buckets);
    assert_eq!(indexes.len(), num_partitions);
    assert!(buckets.len() >= num_partitions * num_active_buckets as usize);
    let mut partition_splits = vec![IndexedPartitionSplit::default(); num_partitions];
    for partition in 0..num_partitions {
        let wa = w_all[partition];
        if wa.empty() {
            continue;
        }
        let do_feature = |af: usize| -> (i32, f64, i32, W, W, CategorySet) {
            let f = active_feature_list[af] as usize;
            if wa.v[0].to_f32() != 0.0 && wa.v[1].to_f32() != 0.0 {
                let start = bucket_offsets[f] as usize;
                let max_bucket = (bucket_offsets[f + 1] as usize - start) as i32 - 1;
                let base = partition * num_active_buckets as usize + start;
                let (s, b, l, r, subset) = choose_split_kernel(&buckets[base..base + max_bucket as usize + 1], max_bucket, features[f].ordinal, features[f].subsets, &wa, criterion);
                return (f as i32, s, b, l, r, subset);
            }
            (f as i32, f64::INFINITY, -1, W::default(), W::default(), [0; 16])
        };
        let results: Vec<_> = if parallel {
            use rayon::prelude::*;
            (0..active_feature_list.len()).into_par_iter().map(do_feature).collect()
        } else {
            (0..active_feature_list.len()).map(do_feature).collect()
        };
        let (mut best_feature, mut best_score, mut best_split, mut best_left, mut best_right, mut best_subset) = (-1, f64::INFINITY, -1, W::default(), W::default(), [0; 16]);
        for (f, score, split, left, right, subset) in results {
            if score == f64::INFINITY {
                continue;
            }
            if score < best_score {
                (best_feature, best_score, best_split, best_left, best_right, best_subset) = (f, score, split, left, right, subset);
            }
        }
        partition_splits[partition] = IndexedPartitionSplit { split: PartitionSplit { score: best_score as f32, feature: best_feature as i16, value: best_split as i16, left: best_left, right: best_right, subset: best_subset }, index: indexes[partition] };
    }
    partition_splits
}

/// Checks that the partition counts match the W counts.
pub fn verify_partition_buckets(partitions: &[RowPartitionInfo], w_all: &[W]) {
    let mut counts = vec![0u32; w_all.len()];
    for p in partitions {
        if p.partition() != RowPartitionInfo::max() {
            counts[p.partition() as usize] += 1;
        }
    }
    for (i, w) in w_all.iter().enumerate() {
        assert_eq!(counts[i], w.count(), "partition {i}: row count vs wAll count");
    }
}

/// The subtrees trained separately, each in its own tree, grafted into `tree`.
fn graft(tree: &mut Tree, trained: Vec<(PartitionIndex, Result<(Tree, Ptr), String>)>) -> Result<BTreeMap<PartitionIndex, Ptr>, String> {
    let mut result = BTreeMap::new();
    for (index, t) in trained {
        let (sub, root) = t?;
        let p = if root.is_some() { tree.copy_recursive(&sub, root) } else { Ptr::Null };
        result.insert(index, p);
    }
    Ok(result)
}

/// Splits the dataset into a separate dataset for each partition, and recurses to create a subtree for each (its
/// partition's buckets, when they are given, else they are accumulated from its rows). Mutually recursive with
/// `train_partitioned_recursive`. The subtrees are trained in parallel near the root, each in a tree of its own, and
/// grafted into `tree` (upstream shared one tree between the threads).
#[allow(clippy::too_many_arguments)]
pub fn split_and_recurse_partitioned(depth: i32, max_depth: i32, tree: &mut Tree, buckets: Option<&[W]>, num_active_buckets: u32, bucket_offsets: &[u32], features: &[Feature], active_features: &[i32], decoded_rows: &[RowWeights], partitions: &[RowPartitionInfo], w_all: &[W], indexes: &[PartitionIndex], fs: &DatasetFeatureSpace) -> Result<BTreeMap<PartitionIndex, Ptr>, String> {
    assert_eq!(partitions.len(), decoded_rows.len());
    assert_eq!(bucket_offsets.len(), features.len() + 1);
    assert_eq!(indexes.len(), w_all.len());
    if depth == max_depth {
        return Ok(BTreeMap::new());
    }
    let nab = num_active_buckets as usize;
    let new_data = split_partitions(features, active_features, decoded_rows, partitions, w_all, indexes);
    let do_entry = |(i, e): (usize, &PartitionEntry)| -> Option<(PartitionIndex, Result<(Tree, Ptr), String>)> {
        if e.index == PartitionIndex::none() {
            return None;
        }
        assert_eq!(e.index.depth(), depth, "partition {i} index {} at depth {depth}", e.index);
        let buckets_in = match buckets {
            Some(b) => b[i * nab..(i + 1) * nab].to_vec(),
            None => initial_buckets(&e.features, &e.active_features, bucket_offsets, num_active_buckets, &e.decoded_rows),
        };
        let mut sub = Tree::new();
        let r = train_partitioned_recursive(depth, max_depth, &mut sub, bucket_offsets, &e.active_features, buckets_in, &e.decoded_rows, &e.w_all, e.index, fs, &e.features);
        Some((e.index, r.map(|p| (sub, p))))
    };
    let trained: Vec<_> = if depth <= 8 {
        use rayon::prelude::*;
        new_data.par_iter().enumerate().filter_map(do_entry).collect()
    } else {
        new_data.iter().enumerate().filter_map(do_entry).collect()
    };
    graft(tree, trained)
}

/// Each active feature's buckets accumulated over the rows (`testFeatureKernel` per feature).
pub fn initial_buckets(features: &[Feature], active_features: &[i32], bucket_offsets: &[u32], num_active_buckets: u32, decoded_rows: &[RowWeights]) -> Vec<W> {
    let mut buckets = vec![W::default(); num_active_buckets as usize];
    for &f in active_features {
        let f = f as usize;
        let (start, end) = (bucket_offsets[f] as usize, bucket_offsets[f + 1] as usize);
        test_feature_kernel(decoded_rows, decoded_rows.len(), &features[f].buckets, &mut buckets[start..end]);
    }
    buckets
}

/// One row in a depth-first descent: its number among the partition's rows, and its decoded value.
#[derive(Clone, Copy, Debug, Default)]
pub struct PartitionExample {
    pub ex: i32,
    pub row: RowWeights,
}

/// A partition put off for later in a depth-first descent: its rows are `start..start + num_examples` of the
/// examples, and its buckets (empty: accumulated when its turn comes).
#[derive(Clone, Debug, Default)]
pub struct PartitionWorkEntry {
    pub index: PartitionIndex,
    pub start: usize,
    pub num_examples: usize,
    pub w_all: W,
    pub buckets: Vec<W>,
}

/// Goes depth-first down a small partition until the recursion limit, giving the splits made on the way. At each
/// split it keeps the bigger side, subtracting the smaller side's rows from its buckets, and puts off the smaller
/// side (whose buckets are accumulated again when it is taken up). Constant memory for the bucket matrix whatever
/// the depth, with up to 2^d splits and O(d^2) work items: much less memory than breadth-first (O(2^d)) when the
/// width is so high that the buckets are very sparse.
pub fn descend_small_partition(examples: &mut [PartitionExample], entry: PartitionWorkEntry, max_depth: i32, num_active_buckets: u32, features: &[Feature], active_features: &[i32], bucket_offsets: &[u32], criterion: SplitCriterion) -> BTreeMap<PartitionIndex, PartitionSplit> {
    let mut splits = BTreeMap::new();
    if entry.index.depth() >= max_depth {
        return splits;
    }
    let mut work = vec![entry];
    while let Some(entry) = work.pop() {
        let PartitionWorkEntry { mut index, mut start, mut num_examples, mut w_all, mut buckets } = entry;
        if buckets.is_empty() {
            buckets = vec![W::default(); num_active_buckets as usize];
            for &f in active_features {
                let off = bucket_offsets[f as usize] as usize;
                for e in &examples[start..start + num_examples] {
                    let bucket = features[f as usize].buckets.get(e.ex as u32) as usize;
                    buckets[off + bucket].add_weights(e.row);
                }
            }
        }
        // Loop over all depths, keeping to the widest at each point.
        loop {
            let mut split = PartitionSplit::default();
            for &f in active_features {
                let (off, end) = (bucket_offsets[f as usize] as usize, bucket_offsets[f as usize + 1] as usize);
                let (score, value, left, right, subset) = choose_split_kernel(&buckets[off..end], (end - off) as i32 - 1, features[f as usize].ordinal, features[f as usize].subsets, &w_all, criterion);
                let score = score as f32;
                if score < split.score {
                    split = PartitionSplit { score, feature: f as i16, value: value as i16, left, right, subset };
                }
            }
            if split.feature == -1 {
                break;
            }
            splits.insert(index, split);
            if index.depth() == max_depth - 1 || split.left.count() == 0 || split.right.count() == 0 {
                break; // we've finished
            }
            // Partition the rows into right (first) and left.
            let sf = &features[split.feature as usize];
            let rows = &mut examples[start..start + num_examples];
            let mut mid = 0;
            for k in 0..rows.len() {
                if side_of(sf.buckets.get(rows[k].ex as u32), split.value as i32, &split.subset, sf) {
                    rows.swap(mid, k);
                    mid += 1;
                }
            }
            let (big_start, big_len, small_start, small_len, big_index, small_index, big_w, small_w);
            if split.transfer_direction() == PartitionSplitDirection::RL {
                // Biggest on right.
                (big_start, big_len, small_start, small_len) = (start, mid, start + mid, num_examples - mid);
                (big_index, small_index, big_w, small_w) = (index.right_child(), index.left_child(), split.right, split.left);
                assert_eq!(big_len as u32, split.right.count());
                assert_eq!(small_len as u32, split.left.count());
            } else {
                // Smallest on right.
                (small_start, small_len, big_start, big_len) = (start, mid, start + mid, num_examples - mid);
                (small_index, big_index, small_w, big_w) = (index.right_child(), index.left_child(), split.right, split.left);
                assert_eq!(small_len as u32, split.right.count());
                assert_eq!(big_len as u32, split.left.count());
            }
            assert!(big_len >= small_len);
            // Update the big side's buckets by subtracting the rows moved out to the small side.
            for &f in active_features {
                let off = bucket_offsets[f as usize] as usize;
                for e in &examples[small_start..small_start + small_len] {
                    let bucket = features[f as usize].buckets.get(e.ex as u32) as usize;
                    buckets[off + bucket].sub_weights(e.row);
                }
            }
            work.push(PartitionWorkEntry { index: small_index, start: small_start, num_examples: small_len, w_all: small_w, buckets: Vec::new() });
            (index, w_all, start, num_examples) = (big_index, big_w, big_start, big_len);
        }
    }
    splits
}

/// Trains the children of the given (small) partitions depth-first: each child's rows gathered, then a descent per
/// child, its subtree extracted from the descent's splits. The rows of these partitions leave the breadth-first
/// pass (their partition becomes "none").
#[allow(clippy::too_many_arguments)]
pub fn train_small_partitions(depth: i32, max_depth: i32, tree: &mut Tree, features: &[Feature], partitions: &mut [RowPartitionInfo], num_partitions_now: usize, num_active_buckets: u32, bucket_offsets: &[u32], decoded_rows: &[RowWeights], active_features: &[i32], small_partitions: &[(i32, IndexedPartitionSplit)], fs: &DatasetFeatureSpace) -> Result<BTreeMap<PartitionIndex, Ptr>, String> {
    let mut small_numbers = vec![-1i32; num_partitions_now];
    let mut kept_rows = 0usize;
    for (i, (p, s)) in small_partitions.iter().enumerate() {
        kept_rows += (s.split.left.count() + s.split.right.count()) as usize;
        small_numbers[*p as usize] = i as i32;
    }
    // Twice as many partitions: one for the left and one for the right of each.
    let num_partitions = small_partitions.len() * 2;
    // (partition, example, row), sorted by partition then example.
    let mut all: Vec<(i32, i32, RowWeights)> = Vec::with_capacity(kept_rows);
    for (i, p) in partitions.iter_mut().enumerate() {
        let partition = p.partition() as usize;
        if partition >= small_numbers.len() {
            assert_eq!(partition as u16, RowPartitionInfo::max());
            continue;
        }
        let small = small_numbers[partition];
        if small == -1 {
            continue;
        }
        // No longer part of the breadth-first pass.
        *p = RowPartitionInfo::max().into();
        let s = &small_partitions[small as usize].1.split;
        if s.feature == -1 {
            continue;
        }
        let f = &features[s.feature as usize];
        let side = side_of(f.buckets.get(i as u32), s.value as i32, &s.subset, f);
        all.push((small * 2 + side as i32, i as i32, decoded_rows[i]));
    }
    assert_eq!(all.len(), kept_rows);
    all.sort_by_key(|&(part, ex, _)| (part, ex));
    let mut starts = Vec::with_capacity(num_partitions + 1);
    let mut current = -1i32;
    for (i, &(part, _, _)) in all.iter().enumerate() {
        while current < part {
            starts.push(i);
            current += 1;
        }
    }
    while starts.len() <= num_partitions {
        starts.push(all.len());
    }
    let mut examples: Vec<PartitionExample> = all.iter().map(|&(_, ex, row)| PartitionExample { ex, row }).collect();
    let mut result = BTreeMap::new();
    for i in 0..num_partitions {
        let s = &small_partitions[i / 2].1;
        let (index, w) = if i % 2 == 0 { (s.index.left_child(), s.split.left) } else { (s.index.right_child(), s.split.right) };
        let (a, b) = (starts[i], starts[i + 1]);
        let entry = PartitionWorkEntry { index, start: 0, num_examples: b - a, w_all: w, buckets: Vec::new() };
        let splits = descend_small_partition(&mut examples[a..b], entry, max_depth, num_active_buckets, features, active_features, bucket_offsets, SplitCriterion::of(fs));
        let leaf = extract_tree(depth + 1, max_depth, tree, index, &splits, &BTreeMap::new(), features, fs)?;
        result.insert(index, leaf);
    }
    Ok(result)
}

/// Maximum number of partitions to handle in parallel in the breadth first algorithm. If it's wider than this, the
/// smaller partitions are handled depth first. `MLDB_RF_CPU_PARTITION_MAX_WIDTH`, default 4096.
pub fn cpu_partition_max_width() -> usize {
    std::env::var("MLDB_RF_CPU_PARTITION_MAX_WIDTH").ok().and_then(|v| v.parse().ok()).unwrap_or(4096)
}

/// Threshold below which a partition is handled depth first, as a multiple of the number of buckets: there is no
/// point keeping large amounts of memory for buckets when there are more buckets than examples. Zero disables the
/// depth-first algorithm. `MLDB_RF_CPU_MIN_PARTITION_EXAMPLE_MULTIPLIER`, default 1.0.
pub fn cpu_min_partition_example_multiplier() -> f32 {
    std::env::var("MLDB_RF_CPU_MIN_PARTITION_EXAMPLE_MULTIPLIER").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0)
}

/// Trains the tree under `root` from its rows and buckets: breadth first, a level at a time over every partition
/// wide enough to be worth it (the others go depth first, `train_small_partitions`), for up to 20 levels, then
/// recursing per partition (`split_and_recurse_partitioned`).
#[allow(clippy::too_many_arguments)]
pub fn train_partitioned_recursive_cpu(mut depth: i32, max_depth: i32, tree: &mut Tree, bucket_offsets: &[u32], active_features: &[i32], buckets_in: Vec<W>, decoded_rows: &[RowWeights], w_all_input: &W, root: PartitionIndex, fs: &DatasetFeatureSpace, features: &[Feature]) -> Result<Ptr, String> {
    let row_count = decoded_rows.len();
    let max_depth_before_recurse = (max_depth - depth).min(20);
    let num_active_buckets = buckets_in.len() as u32;
    let mut buckets = buckets_in;
    let mut partitions = vec![RowPartitionInfo::default(); row_count];
    let mut indexes = vec![root];
    let mut w_all = vec![*w_all_input];
    let mut all_splits: BTreeMap<PartitionIndex, PartitionSplit> = BTreeMap::new();
    // An even width, so that a partition's two sides stay together.
    let max_width = cpu_partition_max_width().max(2) & !1;
    let min_examples_per_bucket = (cpu_min_partition_example_multiplier() * num_active_buckets as f32) as u32;
    let original_depth = depth;
    let mut leaves: BTreeMap<PartitionIndex, Ptr> = BTreeMap::new();

    let mut my_depth = 0;
    while my_depth < max_depth_before_recurse && depth < max_depth {
        let splits = get_partition_splits(&buckets, num_active_buckets, active_features, bucket_offsets, features, &w_all, &indexes, depth < 4, SplitCriterion::of(fs));
        let mut new_partition_numbers = vec![(-1i32, -1i32); splits.len()];
        // The partitions kept breadth first, and those handled depth first.
        let mut wide: Vec<(u32, usize)> = Vec::new();
        let mut small_partitions: Vec<(i32, IndexedPartitionSplit)> = Vec::new();
        for (i, p) in splits.iter().enumerate() {
            if p.split.left.count() > 0 && p.split.right.count() > 0 {
                let count = p.split.left.count() + p.split.right.count();
                if count >= min_examples_per_bucket {
                    wide.push((count, i));
                } else {
                    small_partitions.push((i as i32, *p));
                }
            }
        }
        // Any partitions that don't make the width get handled depth first: the smallest, whole (upstream cut the
        // sides before sorting them, and could keep one side of a partition and not the other).
        wide.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        if wide.len() * 2 > max_width {
            for &(_, i) in &wide[max_width / 2..] {
                small_partitions.push((i as i32, splits[i]));
            }
            wide.truncate(max_width / 2);
        }
        small_partitions.sort_by_key(|p| p.0);
        let small_out = train_small_partitions(depth, max_depth, tree, features, &mut partitions, splits.len(), num_active_buckets, bucket_offsets, decoded_rows, active_features, &small_partitions, fs)?;
        leaves.extend(small_out);

        // Sort the sides of the partitions kept, biggest first.
        let mut partition_counts: Vec<(u32, usize)> = Vec::with_capacity(wide.len() * 2);
        for &(_, i) in &wide {
            partition_counts.push((splits[i].split.left.count(), i * 2));
            partition_counts.push((splits[i].split.right.count(), i * 2 + 1));
        }
        partition_counts.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let mut new_indexes = vec![PartitionIndex::none(); partition_counts.len()];
        for (n, &(_, idx)) in partition_counts.iter().enumerate() {
            let (part, right) = (idx >> 1, idx & 1 == 1);
            if right {
                new_partition_numbers[part].1 = n as i32;
                new_indexes[n] = splits[part].index.right_child();
            } else {
                new_partition_numbers[part].0 = n as i32;
                new_indexes[n] = splits[part].index.left_child();
            }
        }
        indexes = new_indexes;
        update_buckets(features, &mut partitions, &mut buckets, num_active_buckets, &mut w_all, bucket_offsets, &splits, &new_partition_numbers, partition_counts.len(), decoded_rows, active_features);
        for s in &splits {
            all_splits.insert(s.index, s.split);
        }
        depth += 1;
        my_depth += 1;
    }

    // If we're not at the lowest level, partition our data and recurse per partition to create our leaves.
    let new_leaves = split_and_recurse_partitioned(depth, max_depth, tree, Some(&buckets), num_active_buckets, bucket_offsets, features, active_features, decoded_rows, &partitions, &w_all, &indexes, fs)?;
    leaves.extend(new_leaves);
    // Finally, extract a tree from the splits we've been accumulating and our leaves.
    extract_tree(original_depth, max_depth, tree, root, &all_splits, &leaves, features, fs)
}

/// `trainPartitionedRecursive`: upstream's only form is the CPU one.
#[allow(clippy::too_many_arguments)]
pub fn train_partitioned_recursive(depth: i32, max_depth: i32, tree: &mut Tree, bucket_offsets: &[u32], active_features: &[i32], buckets_in: Vec<W>, decoded_rows: &[RowWeights], w_all_input: &W, root: PartitionIndex, fs: &DatasetFeatureSpace, features: &[Feature]) -> Result<Ptr, String> {
    train_partitioned_recursive_cpu(depth, max_depth, tree, bucket_offsets, active_features, buckets_in, decoded_rows, w_all_input, root, fs, features)
}

/// The bucket numbering over the active features (`bucketOffsets`, [nf + 1], an inactive feature has none) and the
/// active features.
pub fn active_bucket_offsets(features: &[Feature]) -> (Vec<u32>, Vec<i32>) {
    let mut offsets = vec![0u32];
    let mut active = Vec::new();
    let mut n = 0u32;
    for (i, f) in features.iter().enumerate() {
        if f.active {
            active.push(i as i32);
            n += f.buckets.num_buckets as u32;
        }
        offsets.push(n);
    }
    (offsets, active)
}

/// Trains the tree over the rows on the CPU, partitioned: the root's buckets accumulated per feature (in
/// parallel), then `train_partitioned_recursive` from the root. The rows' example numbers must be their row
/// numbers.
pub fn train_partitioned_end_to_end_cpu(depth: i32, max_depth: i32, tree: &mut Tree, rows: &Rows, features: &[Feature], fs: &DatasetFeatureSpace) -> Result<Ptr, String> {
    let (bucket_offsets, active_features) = active_bucket_offsets(features);
    let num_active_buckets = *bucket_offsets.last().unwrap();
    let decoded_rows = decode_rows(rows);
    use rayon::prelude::*;
    let per_feature: Vec<Vec<W>> = active_features.par_iter().map(|&f| {
        let f = f as usize;
        let mut w = vec![W::default(); (bucket_offsets[f + 1] - bucket_offsets[f]) as usize];
        test_feature_kernel(&decoded_rows[..], decoded_rows.len(), &features[f].buckets, &mut w);
        w
    }).collect();
    let mut buckets = Vec::with_capacity(num_active_buckets as usize);
    for w in per_feature {
        buckets.extend(w);
    }
    train_partitioned_recursive(depth, max_depth, tree, &bucket_offsets, &active_features, buckets, &decoded_rows, &rows.w_all, PartitionIndex::root(), fs, features)
}
