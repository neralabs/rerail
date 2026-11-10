//! randomforest_recursive.cc, in Rust (in progress: what is here has been cut from the .cc, the rest is still
//! there).
//!
//! Jeremy Barnes, 13 October 2018. Copyright (c) 2018 mldb.ai inc. This file is part of MLDB. Copyright 2016 mldb.ai
//! inc. Apache-2.0 (see the LICENSE at `accel/src/forest/mldb/`).
//!
//! Recursive (CPU) kernels for the random forest algorithm. Here so far: the core of the split search
//! (`testFeatureKernel`, `chooseSplitKernel`, `testFeatureNumber`) and `decodeRows`, which the kernels' host forms
//! (randomforest_kernels.rs) call, and `extractTree`, which builds a tree from the partitioned trainer's splits.
//!
//! The C++ template over row iterators and bucket lists is two traits: `RowSource` (a row iterator, or decoded rows)
//! and `BucketSource` (a bucket list, a view of one, or a plain `u16`/`u32` array). The `debug` traces are not
//! brought over.

use crate::forest::bucket::{BucketList, BucketListRef};
use crate::forest::randomforest_kernels::score_split;
use crate::forest::dataset_feature_space::DatasetFeatureSpace;
use crate::forest::randomforest_types::{decode_row_arr, decode_row_iter, get_leaf, get_node, DecodedRow, Feature, PartitionIndex, PartitionSplit, RowIterator, Rows, W};
use crate::forest::tree::{Ptr, Tree};
use std::collections::BTreeMap;

/// `decodeRow` over what the split search reads its rows from.
pub trait RowSource {
    fn decode_row(&mut self, row_number: usize) -> DecodedRow;
}

impl RowSource for RowIterator<'_> {
    #[inline(always)]
    fn decode_row(&mut self, row_number: usize) -> DecodedRow {
        decode_row_iter(self, row_number)
    }
}

impl RowSource for &[f32] {
    #[inline(always)]
    fn decode_row(&mut self, row_number: usize) -> DecodedRow {
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
        let r = row_iterator.decode_row(j);
        let bucket = buckets.bucket(r.example_num) as i32;
        w[bucket as usize].add_weight(r.label, r.weight);
        max_bucket = max_bucket.max(bucket);
        min_bucket = min_bucket.min(bucket);
    }
    (min_bucket < max_bucket, max_bucket)
}

/// `testFeatureKernel`: upstream's overloads all run the CPU form.
pub fn test_feature_kernel<R: RowSource, B: BucketSource + ?Sized>(row_iterator: R, num_rows: usize, buckets: &B, w: &mut [W]) -> (bool, i32) {
    test_feature_kernel_cpu(row_iterator, num_rows, buckets, w)
}

/// Chooses which is the best split for a given feature: (best score, best split, best left, best right).
pub fn choose_split_kernel(w: &[W] /* at least max_bucket + 1 entries */, max_bucket: i32, ordinal: bool, w_all: &W) -> (f64, i32, W, W) {
    let mut best_score = f64::INFINITY;
    let mut best_split = -1;
    let mut best_left = W::default();
    let mut best_right = W::default();

    if ordinal {
        // Calculate best split point for ordered values.
        let mut w_false = *w_all;
        let mut w_true = W::default();
        // Now test split points one by one.
        for j in 0..=max_bucket.max(-1) {
            if w_false.count() > 0 && w_true.count() > 0 {
                let s = score_split(&w_false, &w_true) as f64;
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
    } else {
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
            let s = score_split(&w_false, &wj) as f64;
            if s < best_score {
                best_score = s;
                best_split = j;
                best_right = w_false;
                best_left = wj;
            }
        }
    }
    (best_score, best_split, best_left, best_right)
}

/// Tests one feature over the rows: (best score, best split, best left, best right, feature is still active).
pub fn test_feature_number(feature_num: usize, features: &[Feature], row_iterator: RowIterator<'_>, num_rows: usize, w_all: &W) -> (f64, i32, W, W, bool) {
    let feature = &features[feature_num];
    let buckets = &feature.buckets;
    let nb = buckets.num_buckets.max(0) as usize;
    let mut w = vec![W::default(); nb];
    if !feature.active {
        return (f64::INFINITY, -1, W::default(), W::default(), false);
    }
    // Is the feature still active?
    let (is_active, max_bucket) = test_feature_kernel(row_iterator, num_rows, buckets, &mut w);
    if is_active {
        let (s, b, l, r) = choose_split_kernel(&w, max_bucket, feature.ordinal, w_all);
        return (s, b, l, r, true);
    }
    (f64::INFINITY, -1, W::default(), W::default(), false)
}

/// Decodes the rows to one float each: the weight, negated when the label is true.
pub fn decode_rows(rows: &Rows) -> Vec<f32> {
    let mut decoded_rows = vec![0.0; rows.row_count()];
    let mut it = rows.get_row_iterator();
    for (i, d) in decoded_rows.iter_mut().enumerate() {
        let row = it.get_decoded_row();
        assert_eq!(i as u32, row.example_num);
        *d = row.weight * (1 - 2 * row.label as i32) as f32;
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
    get_node(tree, s.score, s.feature as i32, s.value as i32, left, right, s.left, s.right, features, fs)
}
