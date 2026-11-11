//! randomforest_types.h and randomforest_types.cc, in Rust.
//!
//! Jeremy Barnes, 13 October 2018. Copyright (c) 2018 mldb.ai inc. This file is part of MLDB. Copyright 2016 mldb.ai
//! inc. Apache-2.0 (see the LICENSE at `accel/src/forest/mldb/`).
//!
//! Types for the random forest algorithm: the weights, rows and partitions the host and the kernels share. Every
//! type the kernels read is `#[repr(C)]` with the size and offsets randomforest_types.cc asserts (and
//! randomforest_kernels_common.h declares on the device side); `forest_types_match_the_kernels` checks them.
//!
//! The `ValueDescription`s of the .cc (MLDB's JSON reflection) are `Debug`; its `print*` helpers print that. A
//! `Feature`'s column is named by its hash in the feature space, where upstream held a pointer to its `ColumnInfo`.

use crate::forest::bit_range_ops::{BitExtractor, BitWriter};
use crate::forest::bucket::{BucketList, CellValue};
use crate::forest::dataset_feature_space::{ColumnHash, DatasetFeatureSpace};
use crate::forest::fixed_point_accum::{FixedPointAccum32, FixedPointAccum64};
use crate::forest::memory_region::{FrozenMemoryRegionT, MappedSerializer, MutableMemoryRegionT, Pod};
use crate::forest::split::{Op, Split};
use crate::forest::tree::{Ptr, Tree};
use std::collections::BTreeSet;
use std::fmt;
use std::ops::{Add, AddAssign, Sub, SubAssign};

/// What a `WT` accumulates in: the fixed-point accumulators (or a float).
pub trait Accum: Copy + Default + PartialEq + fmt::Debug + AddAssign + SubAssign + 'static {
    fn from_f32(v: f32) -> Self;
    fn to_f32(self) -> f32;
}

impl Accum for FixedPointAccum32 {
    fn from_f32(v: f32) -> Self {
        FixedPointAccum32::new(v)
    }
    fn to_f32(self) -> f32 {
        FixedPointAccum32::to_f32(self)
    }
}

impl Accum for FixedPointAccum64 {
    fn from_f32(v: f32) -> Self {
        FixedPointAccum64::new(v)
    }
    fn to_f32(self) -> f32 {
        FixedPointAccum64::to_f32(self)
    }
}

impl Accum for f32 {
    fn from_f32(v: f32) -> Self {
        v
    }
    fn to_f32(self) -> f32 {
        self
    }
}

impl Accum for f64 {
    fn from_f32(v: f32) -> Self {
        v as f64
    }
    fn to_f32(self) -> f32 {
        self as f32
    }
}

/// A row's contribution to the weights: (to false, to true). A row with label `l` and weight `w` gives `w` to its
/// label's side; a regression row with target `t` (in [0, 1]) is a soft label, giving `w (1 - t)` and `w t`.
/// Classification is the case of a target of 0 or 1, so the trainer is the same for both: the buckets, the splits'
/// sides and the leaves (whose true side is the rows' weighted mean target) need nothing more. Only the split's
/// score differs (`SplitCriterion`).
pub type RowWeights = [f32; 2];

/// A categorical split's buckets on its left side (CART's subset split), a bit per bucket: a categorical feature has
/// at most 512 buckets.
pub type CategorySet = [u32; 16];

/// The most buckets a categorical feature may have (`CategorySet`).
pub const MAX_CATEGORICAL_BUCKETS: usize = 512;

/// Whether bucket `b` is in a set (none past the set's 512).
#[inline(always)]
pub fn in_category_set(set: &CategorySet, b: u32) -> bool {
    (b as usize) < MAX_CATEGORICAL_BUCKETS && (set[(b >> 5) as usize] >> (b & 31)) & 1 != 0
}

/// A feature's kind as the device reads it (`featureIsOrdinal`): categorical with upstream's one-vs-rest splits,
/// ordinal, or categorical with CART's subset splits.
pub const FEATURE_KIND_ONE_VS_REST: u32 = 0;
pub const FEATURE_KIND_ORDINAL: u32 = 1;
pub const FEATURE_KIND_SUBSETS: u32 = 2;

impl Feature {
    /// The feature's kind (`FEATURE_KIND_*`).
    pub fn kind(&self) -> u32 {
        if self.ordinal { FEATURE_KIND_ORDINAL } else if self.subsets { FEATURE_KIND_SUBSETS } else { FEATURE_KIND_ONE_VS_REST }
    }
}

/// How a split is scored (lower is better), from its sides' weights, and which splits may be made at all. The
/// device reads the same word (`TreeTrainingInfo::split_criterion`): the score's kind in the low 8 bits, the
/// fewest rows a side may have above them.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SplitCriterion(pub u32);

#[allow(non_upper_case_globals)]
impl SplitCriterion {
    /// upstream's: `2 (sqrt(f0 t0) + sqrt(f1 t1))` over the sides' false and true weights, for classification.
    pub const Sqrt: SplitCriterion = SplitCriterion(0);
    /// The rows' squared error about their side's mean target: `sum over sides of f t / (f + t)`, in the
    /// accumulators' fixed point and exact (the sum of `w t^2` is the same for every split, and leaves out). For
    /// regression; the Gini impurity for classification. A split must lower the node's own error (exactly).
    pub const Variance: SplitCriterion = SplitCriterion(1);

    /// The criterion for a feature space's label (the variance for a real label, else upstream's), with its
    /// fewest rows a leaf may have (`DatasetFeatureSpace::min_leaf_examples`).
    pub fn of(fs: &DatasetFeatureSpace) -> SplitCriterion {
        let kind = if fs.label_info == crate::forest::dataset_feature_space::FeatureType::Real { SplitCriterion::Variance } else { SplitCriterion::Sqrt };
        kind.with_min_leaf(fs.min_leaf_examples)
    }

    /// The score's kind (`Sqrt` or `Variance`).
    pub fn kind(self) -> SplitCriterion {
        SplitCriterion(self.0 & 0xff)
    }

    /// The fewest rows a split's side may have (at least 1).
    pub fn min_leaf(self) -> u32 {
        (self.0 >> 8).max(1)
    }

    pub fn with_min_leaf(self, m: u32) -> SplitCriterion {
        SplitCriterion((self.0 & 0xff) | (m.min(0xff_ffff) << 8))
    }
}

/// This structure holds the weights (false and true) for any particular split.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WT<F: Accum> {
    pub v: [F; 2],
    pub c: i32,
}

impl<F: Accum> WT<F> {
    /// `add(i, val)` upstream (named apart from `Add::add`).
    pub fn add_weight(&mut self, i: bool, val: f32) {
        self.v[i as usize] += F::from_f32(val);
        self.c += 1;
    }

    /// Adds a row's weights (`RowWeights`).
    pub fn add_weights(&mut self, r: RowWeights) {
        self.v[0] += F::from_f32(r[0]);
        self.v[1] += F::from_f32(r[1]);
        self.c += 1;
    }

    /// Removes a row's weights (`RowWeights`).
    pub fn sub_weights(&mut self, r: RowWeights) {
        self.v[0] -= F::from_f32(r[0]);
        self.v[1] -= F::from_f32(r[1]);
        self.c -= 1;
    }

    /// `sub(i, val)` upstream.
    pub fn sub_weight(&mut self, i: bool, val: f32) {
        self.v[i as usize] -= F::from_f32(val);
        self.c -= 1;
    }

    pub fn get(&self, i: bool) -> F {
        self.v[i as usize]
    }

    pub fn empty(&self) -> bool {
        self.total() == 0.0
    }

    /// Is the label uniform, meaning we can learn nothing more from this region of the tree?
    pub fn uniform(&self) -> bool {
        self.v[0].to_f32() == 0.0 || self.v[1].to_f32() == 0.0
    }

    pub fn total(&self) -> f32 {
        self.v[0].to_f32() + self.v[1].to_f32()
    }

    pub fn count(&self) -> u32 {
        self.c as u32
    }
}

impl<F: Accum> AddAssign for WT<F> {
    fn add_assign(&mut self, other: Self) {
        self.v[0] += other.v[0];
        self.v[1] += other.v[1];
        self.c += other.c;
    }
}

impl<F: Accum> Add for WT<F> {
    type Output = Self;
    fn add(mut self, other: Self) -> Self {
        self += other;
        self
    }
}

impl<F: Accum> SubAssign for WT<F> {
    fn sub_assign(&mut self, other: Self) {
        self.v[0] -= other.v[0];
        self.v[1] -= other.v[1];
        self.c -= other.c;
    }
}

impl<F: Accum> Sub for WT<F> {
    type Output = Self;
    fn sub(mut self, other: Self) -> Self {
        self -= other;
        self
    }
}

pub type W64 = WT<FixedPointAccum64>;
pub type W32 = WT<FixedPointAccum32>;
pub type W = W32;

unsafe impl Pod for FixedPointAccum32 {}
unsafe impl Pod for FixedPointAccum64 {}
unsafe impl<F: Accum + Pod> Pod for WT<F> {}

/// Version of W with an index field; index is used in local GPU memory for metadata.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WIndexed {
    pub w: W,
    pub index: i32,
}

/// Entry for an individual feature.
#[derive(Clone, Debug)]
pub struct Feature {
    /// If true, the feature can be split on.
    pub active: bool,
    /// If true, it's continuous valued; else categorical.
    pub ordinal: bool,
    /// A categorical feature's splits are CART's subsets (`choose_split_kernel`), not upstream's one bucket against
    /// the rest (rerail's extension; `RandomForestProcedureConfig::categorical_subsets`).
    pub subsets: bool,
    /// The feature's column in the feature space.
    pub info: Option<ColumnHash>,
    /// List of bucket numbers, per example.
    pub buckets: BucketList,
}

impl Default for Feature {
    fn default() -> Self {
        Feature { active: false, ordinal: true, subsets: false, info: None, buckets: BucketList::default() }
    }
}

/// Defines how weights are stored in rows.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WeightFormat {
    /// Stored as an int multiplied by a constant.
    WfIntMultiple = 0,
    /// Stored as an index into a fixed table.
    WfTable = 1,
    /// Stored directly as a 32 bit float.
    #[default]
    WfFloat = 2,
}

#[derive(Clone, Debug)]
pub struct WeightEncoder {
    pub weight_format: WeightFormat,
    pub weight_bits: i32,
    pub weight_format_table: FrozenMemoryRegionT<f32>,
    pub weight_multiplier: f32,
}

impl Default for WeightEncoder {
    fn default() -> Self {
        WeightEncoder { weight_format: WeightFormat::WfFloat, weight_bits: 32, weight_format_table: Default::default(), weight_multiplier: 0.0 }
    }
}

impl WeightEncoder {
    #[inline]
    pub fn decode_weight(&self, weight_bits: u32) -> f32 {
        match self.weight_format {
            WeightFormat::WfIntMultiple => self.weight_multiplier * weight_bits as f32,
            WeightFormat::WfTable => self.weight_format_table.data()[weight_bits as usize],
            WeightFormat::WfFloat => f32::from_bits(weight_bits),
        }
    }

    /// The bits for `weight`. An int multiple is rounded to the nearest multiple (upstream truncated, so a weight
    /// that is 3 multiples in exact arithmetic but 2.9999998 in float was encoded as 2); a table weight is its
    /// place in the (sorted) table (upstream threw: never encoded).
    fn encode_weight_(&self, weight: f32) -> Result<u32, String> {
        match self.weight_format {
            WeightFormat::WfIntMultiple => {
                let m = (weight / self.weight_multiplier).round();
                if !(0.0..=u32::MAX as f32).contains(&m) {
                    return Err(format!("weight {weight} is {m} multiples of {}", self.weight_multiplier));
                }
                Ok(m as u32)
            }
            WeightFormat::WfTable => {
                let table = self.weight_format_table.data();
                table.binary_search_by(|t| t.total_cmp(&weight)).map(|i| i as u32).map_err(|_| format!("weight {weight} is not in the weight table"))
            }
            WeightFormat::WfFloat => Ok(weight.to_bits()),
        }
    }

    /// The bits for `weight`: an error unless they fit the weight bits and decode to `weight` exactly (the check
    /// upstream had commented out).
    pub fn encode_weight(&self, weight: f32) -> Result<u32, String> {
        let result = self.encode_weight_(weight)?;
        if self.weight_bits < 32 && result >> self.weight_bits != 0 {
            return Err(format!("weight {weight} encodes as {result}, wider than {} bits", self.weight_bits));
        }
        if self.decode_weight(result) != weight {
            return Err(format!("weight {weight} encodes as {result}, which decodes as {}", self.decode_weight(result)));
        }
        Ok(result)
    }
}

/// Entry for an individual row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    /// Label associated with example.
    pub label_: bool,
    /// Weight of the example.
    pub encoded_weight_: u32,
    /// Index into feature array.
    pub example_num_: u32,
}

impl Row {
    pub fn label(&self) -> bool {
        self.label_
    }
    pub fn example_num(&self) -> u32 {
        self.example_num_
    }
    pub fn encoded_weight(&self) -> u32 {
        self.encoded_weight_
    }
}

/// Version of Row that has the weight decoded.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DecodedRow {
    pub label: bool,
    pub weight: f32,
    pub example_num: u32,
    /// The row's target in [0, 1]: its label (0 or 1), or its regression target (`Rows::targets`).
    pub target: f32,
}

impl DecodedRow {
    /// The row's weights to false and true (`row_weights`).
    pub fn weights(&self) -> RowWeights {
        row_weights(self.weight, self.target)
    }
}

/// A row's weights to false and true from its weight and target: `w (1 - t)` (one rounding, a fused multiply-add,
/// as the device's `decompressRowsImpl`, so that the two agree to the bit) and `w t`; exact for a label.
#[inline(always)]
pub fn row_weights(weight: f32, target: f32) -> RowWeights {
    [(-weight).mul_add(target, weight), weight * target]
}

/// Writes rows, bit-packed: label, then weight, then (unless sequential) example number.
#[derive(Debug)]
pub struct RowWriter {
    weight_encoder: WeightEncoder,
    weight_bits: i32,
    example_num_bits: i32,
    total_bits: i32,
    to_allocate: usize,
    data: MutableMemoryRegionT<u64>,
    /// Bits written so far (the writer's position).
    written: usize,
    w_all: W,
    weight_scaling_factor: f32,
    /// The rows' regression targets by example number (`add_row_target`); none for a classification.
    targets: Option<Vec<f32>>,
}

/// `MLDB::highest_bit(x, none)` over 64 bits.
fn highest_bit64(x: u64, none: i32) -> i32 {
    if x == 0 { none } else { 63 - x.leading_zeros() as i32 }
}

impl RowWriter {
    pub fn new(weight_encoder: &WeightEncoder, max_rows: usize, max_example_count: usize, serializer: &mut impl MappedSerializer, sequential_example_nums: bool, weight_scaling_factor: f32) -> Self {
        let weight_bits = weight_encoder.weight_bits;
        let example_num_bits = if sequential_example_nums { 0 } else { highest_bit64(max_example_count as u64, -1) + 1 };
        let total_bits = weight_bits + example_num_bits + 1; // for label
        // +1 allows extractFast
        let to_allocate = (total_bits as usize * max_rows).div_ceil(64) + 1;
        let data = serializer.allocate_writable_t::<u64>(to_allocate, 4096); // page aligned
        RowWriter { weight_encoder: weight_encoder.clone(), weight_bits, example_num_bits, total_bits, to_allocate, data, written: 0, w_all: W::default(), weight_scaling_factor, targets: None }
    }

    fn write_bits(&mut self, val: u64, bits: u32) {
        let mut w = BitWriter::new(self.data.data_mut());
        w.skip(self.written as u32);
        w.write(val, bits);
        self.written += bits as usize;
    }

    /// A classification row (its target is its label).
    pub fn add_row(&mut self, row: &Row) {
        assert!(self.targets.is_none(), "a classification row among regression rows");
        self.add_row_with(row, row.label() as u8 as f32);
    }

    /// A row with its target (in [0, 1]; its label for a classification): a regression writer keeps the target by
    /// the example number the rows will decode with (the row's, or its place when they are sequential).
    pub fn add_row_with(&mut self, row: &Row, target: f32) {
        if let Some(t) = &mut self.targets {
            let ex = if self.example_num_bits == 0 { self.written / self.total_bits as usize } else { row.example_num() as usize };
            if t.len() <= ex {
                t.resize(ex + 1, f32::NAN);
            }
            t[ex] = target;
        }
        let mut to_write = row.label() as u64;
        to_write = (to_write << self.weight_bits) | row.encoded_weight_ as u64;
        if self.example_num_bits > 0 {
            to_write = (to_write << self.example_num_bits) | row.example_num() as u64;
        }
        self.write_bits(to_write, self.total_bits as u32);
        let weight = self.weight_encoder.decode_weight(row.encoded_weight_) * self.weight_scaling_factor;
        self.w_all.add_weights(row_weights(weight, target));
    }

    /// A regression row: its target, in [0, 1], kept by example number (its label is the nearer of 0 and 1). A
    /// writer's rows are all classification rows or all regression rows.
    pub fn add_row_target(&mut self, target: f32, weight: f32, example_num: u32) -> Result<(), String> {
        if !(0.0..=1.0).contains(&target) {
            return Err(format!("example {example_num}: target {target} is outside [0, 1]"));
        }
        if self.written > 0 && self.targets.is_none() {
            return Err("a regression row after classification rows".into());
        }
        let row = Row { label_: target >= 0.5, encoded_weight_: self.weight_encoder.encode_weight(weight)?, example_num_: example_num };
        self.targets.get_or_insert_with(Vec::new);
        self.add_row_with(&row, target);
        Ok(())
    }

    pub fn add_row_values(&mut self, label: bool, weight: f32, example_num: u32) -> Result<(), String> {
        let to_add = Row { label_: label, encoded_weight_: self.weight_encoder.encode_weight(weight)?, example_num_: example_num };
        self.add_row(&to_add);
        Ok(())
    }

    pub fn freeze(mut self, serializer: &mut impl MappedSerializer) -> Rows {
        let num_row_entries = self.written / self.total_bits as usize;
        // fill in extra guard word at end, required for extractFast to work
        self.write_bits(0, 64);
        {
            let mut w = BitWriter::new(self.data.data_mut());
            w.skip(self.written as u32);
            w.zero_extra_bits();
        }
        let _ = self.to_allocate;
        Rows {
            row_data: serializer.freeze_t(&mut self.data),
            num_row_entries,
            example_num_bits: self.example_num_bits,
            example_num_mask: (1u64 << self.example_num_bits) - 1,
            weight_mask: if self.weight_bits >= 64 { u64::MAX } else { (1u64 << self.weight_bits) - 1 },
            total_bits: self.total_bits,
            weight_encoder: self.weight_encoder,
            w_all: self.w_all,
            targets: self.targets.map(std::sync::Arc::new),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Rows {
    /// Encoded data for our rows.
    pub row_data: FrozenMemoryRegionT<u64>,
    /// Number of entries in rowData.
    pub num_row_entries: usize,
    pub example_num_bits: i32,
    pub example_num_mask: u64,
    pub weight_mask: u64,
    pub total_bits: i32,
    pub weight_encoder: WeightEncoder,
    pub w_all: W,
    /// Regression targets in [0, 1] by example number; none for a classification (the target is the label).
    pub targets: Option<std::sync::Arc<Vec<f32>>>,
}

impl Rows {
    /// A row's target: its regression target, else its label.
    #[inline(always)]
    pub fn target_of(&self, label: bool, example_num: u32) -> f32 {
        match &self.targets {
            Some(t) => t[example_num as usize],
            None => label as u8 as f32,
        }
    }
}

/// Iterates the rows in order.
#[derive(Clone, Copy, Debug)]
pub struct RowIterator<'a> {
    pub owner: &'a Rows,
    pub extractor: BitExtractor<'a, u64>,
    pub total_bits: i32,
    pub row_number: u32,
}

impl<'a> RowIterator<'a> {
    #[inline(always)]
    pub fn get_row_bits(&mut self) -> u64 {
        self.extractor.extract_fast_unmasked(self.total_bits as u32)
    }

    #[inline(always)]
    pub fn get_row(&mut self) -> Row {
        let bits = self.get_row_bits();
        let r = self.owner.decode_row(bits, self.row_number);
        self.row_number += 1;
        r
    }

    #[inline(always)]
    pub fn get_decoded_row(&mut self) -> DecodedRow {
        let row = self.get_row();
        DecodedRow { label: row.label(), weight: self.owner.weight_encoder.decode_weight(row.encoded_weight_), example_num: row.example_num(), target: self.owner.target_of(row.label(), row.example_num()) }
    }

    #[inline(always)]
    pub fn skip_row(&mut self) {
        self.extractor.advance(self.total_bits as isize);
        self.row_number += 1;
    }

    pub fn skip_to(&mut self, new_row_number: usize) {
        assert!(new_row_number >= self.row_number as usize);
        self.extractor.advance(self.total_bits as isize * (new_row_number as isize - self.row_number as isize));
        // Upstream leaves the row number behind, so a second skip (or a row with sequential example numbers) after
        // a skip from a fresh iterator reads the wrong row; it is moved here.
        self.row_number = new_row_number as u32;
    }
}

impl Rows {
    pub fn get_row_iterator(&self) -> RowIterator<'_> {
        RowIterator { owner: self, extractor: BitExtractor::new(self.row_data.data()), total_bits: self.total_bits, row_number: 0 }
    }

    #[inline(always)]
    pub fn decode_row(&self, mut all_bits: u64, row_number: u32) -> Row {
        let mut result = Row::default();
        if self.example_num_bits == 0 {
            result.example_num_ = row_number;
        } else {
            result.example_num_ = (all_bits & self.example_num_mask) as u32;
            all_bits >>= self.example_num_bits;
        }
        result.encoded_weight_ = (all_bits & self.weight_mask) as u32;
        all_bits = if self.weight_encoder.weight_bits >= 64 { 0 } else { all_bits >> self.weight_encoder.weight_bits };
        result.label_ = all_bits & 1 != 0;
        result
    }

    #[inline(always)]
    pub fn get_row_bits(&self, i: usize) -> u64 {
        let mut extractor = BitExtractor::new(self.row_data.data());
        extractor.advance(self.total_bits as isize * i as isize);
        extractor.extract_fast(self.total_bits as u32)
    }

    #[inline(always)]
    pub fn get_row(&self, i: usize) -> Row {
        self.decode_row(self.get_row_bits(i), i as u32)
    }

    #[inline(always)]
    pub fn get_decoded_row(&self, i: usize) -> DecodedRow {
        let row = self.get_row(i);
        DecodedRow { label: row.label(), weight: self.weight_encoder.decode_weight(row.encoded_weight_), example_num: row.example_num(), target: self.target_of(row.label(), row.example_num()) }
    }

    pub fn get_weight(&self, i: usize) -> f32 {
        let all_bits = self.get_row_bits(i) >> self.example_num_bits;
        self.weight_encoder.decode_weight((all_bits & self.weight_mask) as u32)
    }

    pub fn get_example_num(&self, i: usize) -> u32 {
        if self.example_num_bits == 0 {
            return i as u32;
        }
        (self.get_row_bits(i) & self.example_num_mask) as u32
    }

    pub fn row_count(&self) -> usize {
        self.num_row_entries
    }

    pub fn highest_example_num(&self) -> u32 {
        self.get_example_num(self.num_row_entries - 1)
    }

    pub fn clear(&mut self) {
        self.row_data = FrozenMemoryRegionT::default();
    }

    pub fn get_row_writer(&self, max_rows: usize, max_example_count: usize, serializer: &mut impl MappedSerializer, sequential_example_nums: bool, weight_scaling_factor: f32) -> RowWriter {
        let mut w = RowWriter::new(&self.weight_encoder, max_rows, max_example_count, serializer, sequential_example_nums, weight_scaling_factor);
        // A regression's rows give regression rows.
        w.targets = self.targets.as_ref().map(|_| Vec::new());
        w
    }
}

/// Partition indexes indicate where a tree fragment exists in the tree. They cover the tree space up to a depth of
/// 32.
///
/// Index 0 is null; no partition has that index. Index 1 is the root (depth 0). Indexes 2 and 3 are the left and
/// right of the root (depth 1). Indexes 4 and 5 are the left children of 2 and 3; indexes 6 and 7 are the right
/// children of 2 and 3.
///
/// ```text
/// index 0: 0000: left =  0, right =  0, parent = 0, depth = -1
/// index 1: 0001: left =  2, right =  3, parent = 0, depth =  0
/// index 2: 0010: left =  4, right =  6, parent = 1, depth =  1
/// index 3: 0011: left =  5, right =  7, parent = 1, depth =  1
/// index 4: 0100: left =  8, right = 12, parent = 2, depth =  2
/// index 5: 0101: left =  9, right = 13, parent = 3, depth =  2
/// index 6: 0110: left = 10, right = 14, parent = 2, depth =  2
/// index 7: 0111: left = 11, right = 15, parent = 3, depth =  2
/// ```
///
/// And so on down. A static array of 2^depth elements is sufficient to hold a tree of that depth, with each index
/// uniquely identifying a place in that array.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PartitionIndex {
    pub index: u32,
}

impl PartitionIndex {
    pub const fn new(index: u32) -> Self {
        PartitionIndex { index }
    }

    pub const fn none() -> Self {
        PartitionIndex { index: 0 }
    }

    pub const fn root() -> Self {
        PartitionIndex { index: 1 }
    }

    pub fn depth(&self) -> i32 {
        if self.index == 0 { -1 } else { 31 - self.index.leading_zeros() as i32 }
    }

    /// Left child is in the same position but bumped up.
    pub fn left_child(&self) -> PartitionIndex {
        assert_ne!(self.index, 0, "left child of none");
        PartitionIndex::new(self.index + (1 << self.depth()))
    }

    pub fn right_child(&self) -> PartitionIndex {
        assert_ne!(self.index, 0, "right child of none");
        PartitionIndex::new(self.index + (2 << self.depth()))
    }

    /// The parent; none and the root have none (an error).
    pub fn parent(&self) -> Result<PartitionIndex, String> {
        if self.index <= 1 {
            return Err(format!("partition {}: no parent", self.path()));
        }
        let depth_bit = 1u32 << (self.depth() - 1);
        let mask = depth_bit - 1;
        Ok(PartitionIndex::new(depth_bit + (self.index & mask)))
    }

    pub fn path(&self) -> String {
        match self.index {
            0 => "none".into(),
            1 => "root".into(),
            _ => (0..self.depth()).map(|d| if self.index & (1 << d) != 0 { 'r' } else { 'l' }).collect(),
        }
    }
}

impl fmt::Display for PartitionIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.path())
    }
}

/// Which way weight is best transferred for a split.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartitionSplitDirection {
    LR = 0,
    RL = 1,
}

/// Holds the split for a partition.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PartitionSplit {
    pub score: f32,
    pub feature: i16,
    /// An ordinal split's bucket (the left is below it); a categorical split's number of buckets on the left.
    pub value: i16,
    /// 12 bytes: true, false, count.
    pub left: W,
    pub right: W,
    /// A categorical split's buckets on the left (`CategorySet`; rerail's addition, CART's subset split).
    pub subset: CategorySet,
}

impl Default for PartitionSplit {
    fn default() -> Self {
        PartitionSplit { score: f32::INFINITY, feature: -1, value: -1, left: W::default(), right: W::default(), subset: [0; 16] }
    }
}

impl PartitionSplit {
    /// Tells us which is the most efficient way to transfer weight: from left to right, or from right to left. This
    /// doesn't affect how things are laid out. If on the right we have a single row and on the left 1,000, we can
    /// accumulate the single row on the right, and infer the left by subtracting the right from wAll, doing no work
    /// for the 999 rows on the left. This is a big optimization and leads to a major speedup. The direction does not
    /// swap the buckets: the left is on the left, and the right is on the right. It just affects which one we
    /// accumulate.
    pub fn transfer_direction(&self) -> PartitionSplitDirection {
        if self.left.count() < self.right.count() { PartitionSplitDirection::RL } else { PartitionSplitDirection::LR }
    }

    pub fn valid(&self) -> bool {
        self.feature != -1
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct IndexedPartitionSplit {
    pub split: PartitionSplit,
    pub index: PartitionIndex,
}

/// Structure giving information about a given row (which partition it's in).
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RowPartitionInfo {
    pub partition_: u16,
}

impl RowPartitionInfo {
    pub fn partition(&self) -> u16 {
        self.partition_
    }
    pub const fn max() -> u16 {
        u16::MAX
    }
}

impl From<u16> for RowPartitionInfo {
    fn from(p: u16) -> Self {
        RowPartitionInfo { partition_: p }
    }
}

#[derive(Debug, Default)]
pub struct PartitionEntry {
    pub decoded_rows: Vec<RowWeights>,
    pub features: Vec<Feature>,
    pub active_features: Vec<i32>,
    pub active_feature_set: BTreeSet<i32>,
    pub w_all: W,
    pub index: PartitionIndex,
    /// A contiguous block of memory for all of the feature blocks on each side; this enables a single GPU transfer
    /// and a single GPU argument list.
    pub bucket_memory_offsets: Vec<usize>,
    pub mutable_bucket_memory: MutableMemoryRegionT<u32>,
    pub bucket_memory: FrozenMemoryRegionT<u32>,
}

/// Information about a partition (its index in the tree and where its left and right children are found).
///
/// For each of these: 0 to 65535 means "goes in partition x"; 65536 to 131071 means "goes in no partition, but
/// subtracted from x - 65536"; -1 means "goes nowhere".
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartitionInfo {
    pub left: i32,
    pub right: i32,
}

impl Default for PartitionInfo {
    fn default() -> Self {
        PartitionInfo { left: -1, right: -1 }
    }
}

impl PartitionInfo {
    /// Ignore as neither one affects any buckets.
    pub fn ignore(&self) -> bool {
        self.left == -1 && self.right == -1
    }
}

/// Work entry for a row for which we need to update buckets.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UpdateWorkEntry {
    /// Row number to update.
    pub row: u32,
    /// Partition number of row.
    pub partition: u16,
    /// Linear index of small side bucket to update.
    pub small_side_index: u16,
    /// Decoded version of row: its weights (`RowWeights`).
    pub decoded_row: RowWeights,
}

/// Sizing information for a whole tree.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TreeTrainingInfo {
    /// Number of rows in our training set.
    pub num_rows: u32,
    /// Maximum depth we're willing to get to.
    pub max_depth: u16,
    /// Number of active features.
    pub num_active_features: u16,
    /// Number of total features.
    pub num_features: u16,
    /// Maximum width of active partition tree.
    pub max_num_active_partitions: u16,
    /// Number of buckets across all active features.
    pub num_active_buckets: u32,
    /// Which feature sampling bag we belong to.
    pub feature_sampling: u16,
    /// Which feature vector sampling bag we belong to.
    pub feature_vector_sampling: u16,
    /// Length of the pool of every level's partition splits (rerail's addition: the device stops before a level
    /// whose splits would not fit).
    pub max_num_finished_partitions: u32,
    /// How splits are scored (`SplitCriterion`, rerail's addition).
    pub split_criterion: u32,
}

/// `TreeDepthInfo::status` once the device has stopped and left the rest of the tree to the host.
pub const TREE_DEPTH_STATUS_STOPPED: u32 = 1;

/// Sizing information for a depth iteration.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeDepthInfo {
    pub depth: u32,
    /// numFinishedPartitions for previous depth.
    pub prev_num_finished_partitions: u32,
    pub prev_num_active_partitions: u32,
    pub num_finished_partitions: u32,
    pub num_active_partitions: u32,
    pub num_small_side_rows: u32,
    /// Non-zero means to stop running.
    pub status: u32,
}

impl Default for TreeDepthInfo {
    fn default() -> Self {
        TreeDepthInfo { depth: 0, prev_num_finished_partitions: 0, prev_num_active_partitions: u32::MAX, num_finished_partitions: 0, num_active_partitions: 1, num_small_side_rows: 0, status: 0 }
    }
}

unsafe impl Pod for WIndexed {}
unsafe impl Pod for PartitionIndex {}
unsafe impl Pod for PartitionSplit {}
unsafe impl Pod for IndexedPartitionSplit {}
unsafe impl Pod for RowPartitionInfo {}
unsafe impl Pod for PartitionInfo {}
unsafe impl Pod for UpdateWorkEntry {}
unsafe impl Pod for TreeTrainingInfo {}
unsafe impl Pod for TreeDepthInfo {}

pub fn print_tree_depth_info(info: &TreeDepthInfo) -> String {
    format!("{info:?}")
}

pub fn print_partition_split(split: &PartitionSplit) -> String {
    format!("{split:?}")
}

pub fn print_indexed_partition_split(split: &IndexedPartitionSplit) -> String {
    format!("{split:?}")
}

/// `printW` (and its duplicate `printWYouFucker`).
pub fn print_w(w: &W32) -> String {
    format!("{w:?}")
}

// --- Tree manipulation ---

pub fn fill_in_base(tree: &mut Tree, node: Ptr, w_all: &W) {
    let total = w_all.v[0].to_f32() + w_all.v[1].to_f32();
    let base = tree.base_mut(node).expect("fill_in_base: no node");
    base.examples = w_all.count() as f32;
    base.pred = vec![w_all.v[0].to_f32() / total, w_all.v[1].to_f32() / total];
}

pub fn get_leaf(tree: &mut Tree, w: &W) -> Ptr {
    let node = tree.new_leaf();
    fill_in_base(tree, node, w);
    node
}

#[allow(clippy::too_many_arguments)]
pub fn get_node(tree: &mut Tree, best_score: f32, best_feature: i32, best_split: i32, subset: &CategorySet, left: Ptr, right: Ptr, w_left: W, w_right: W, features: &[Feature], fs: &DatasetFeatureSpace) -> Result<Ptr, String> {
    let f = &features[best_feature as usize];
    let hash = f.info.ok_or("get_node: feature without a column")?;
    let column = fs.column_info.get(&hash).ok_or("get_node: feature's column not in the feature space")?;
    let feature = fs.get_feature(hash);
    let split_val = if f.ordinal && best_split as u32 == column.bucket_descriptions.numeric.offset && best_split > 0 {
        // The nulls (bucket 0) on the left, every number on the right: upstream's `getSplit` throws here (there is
        // no split point below the first numeric bucket), failing the training. As a value, -inf: every number is
        // false and a null is missing, which goes left.
        f32::NEG_INFINITY
    } else if f.ordinal {
        match column.bucket_descriptions.get_split(best_split as u32)? {
            c @ (CellValue::Integer(_) | CellValue::Float(_)) => c.to_double() as f32,
            _ => best_split as f32,
        }
    } else {
        best_split as f32
    };
    let split = Split::new(feature, split_val, if f.ordinal { Op::Less } else { Op::Equal })?;
    // A missing value is a null, which training put in bucket 0: on the left of an ordinal split (whose bucket is
    // at least 1), and on the left of a categorical one when bucket 0 is in its subset. Upstream gave the missing
    // branch an empty leaf (a 0/0 prediction).
    let missing = if f.ordinal || (if f.subsets { in_category_set(subset, 0) } else { best_split == 0 }) { left } else { right };
    let node = tree.new_node();
    {
        let n = tree.node_mut(node).unwrap();
        n.split = split;
        n.child_true = left;
        n.child_false = right;
        n.child_missing = missing;
        n.z = best_score;
        n.split_bucket = best_split;
        if !f.ordinal && f.subsets {
            n.split_subset = subset.to_vec();
        }
    }
    fill_in_base(tree, node, &(w_left + w_right));
    Ok(node)
}

pub fn decode_row_iter(row_iterator: &mut RowIterator<'_>, _row_number: usize) -> (u32, RowWeights) {
    let r = row_iterator.get_decoded_row();
    (r.example_num, r.weights())
}

pub fn decode_row_arr(arr: &[RowWeights], row_number: usize) -> (u32, RowWeights) {
    (row_number as u32, arr[row_number])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forest::memory_region::MemorySerializer;
    use std::mem::{align_of, offset_of, size_of};

    /// randomforest_types.cc: "Verify that sizes of structures are matched by GPU versions".
    #[test]
    fn forest_types_match_the_kernels() {
        assert_eq!(size_of::<W32>(), 12, "GPU kernels expect that W32 should be a 12 byte structure");
        assert_eq!(align_of::<W32>(), 4, "GPU kernels expect that W32 should be 4 byte aligned");
        assert_eq!(size_of::<W64>(), 24, "GPU kernels expect that W64 should be a 24 byte structure");
        assert_eq!(align_of::<W64>(), 8, "GPU kernels expect that W64 should be 8 byte aligned");
        assert_eq!(size_of::<PartitionIndex>(), 4);
        assert_eq!(align_of::<PartitionIndex>(), 4);
        assert_eq!(size_of::<WIndexed>(), 16);
        assert_eq!(size_of::<PartitionSplit>(), 96);
        assert_eq!(size_of::<IndexedPartitionSplit>(), 100);
        assert_eq!(offset_of!(PartitionSplit, score), 0);
        assert_eq!(offset_of!(PartitionSplit, feature), 4);
        assert_eq!(offset_of!(PartitionSplit, value), 6);
        assert_eq!(offset_of!(PartitionSplit, left), 8);
        assert_eq!(offset_of!(PartitionSplit, right), 20);
        assert_eq!(offset_of!(PartitionSplit, subset), 32);
        assert_eq!(offset_of!(IndexedPartitionSplit, index), 96);
        // The kernels' own structs (randomforest_kernels_common.h).
        assert_eq!(size_of::<TreeDepthInfo>(), 28);
        assert_eq!(size_of::<TreeTrainingInfo>(), 28);
        assert_eq!(size_of::<UpdateWorkEntry>(), 16);
        assert_eq!(size_of::<PartitionInfo>(), 8);
        assert_eq!(size_of::<RowPartitionInfo>(), 2);
    }

    #[test]
    fn rows_write_freeze_decode() {
        let mut s = MemorySerializer;
        let enc = WeightEncoder { weight_format: WeightFormat::WfIntMultiple, weight_bits: 5, weight_multiplier: 0.5, ..Default::default() };
        // W32 holds values below 2 (30 fractional bits): the scaling factor keeps the total weight in range.
        let mut w = RowWriter::new(&enc, 100, 100, &mut s, false, 1.0 / 1024.0);
        for i in 0..100u32 {
            w.add_row(&Row { label_: i % 3 == 0, encoded_weight_: i % 32, example_num_: 99 - i });
        }
        let rows = w.freeze(&mut s);
        assert_eq!(rows.row_count(), 100);
        let mut it = rows.get_row_iterator();
        for i in 0..100usize {
            let r = rows.get_decoded_row(i);
            assert_eq!(r, it.get_decoded_row(), "row {i}");
            assert_eq!(r.label, i % 3 == 0);
            assert_eq!(r.weight, (i % 32) as f32 * 0.5);
            assert_eq!(r.example_num, 99 - i as u32);
        }
        let total: f32 = (0..100).map(|i| (i % 32) as f32 * 0.5).sum();
        assert!((rows.w_all.total() - total / 1024.0).abs() < 1e-5);
        assert_eq!(decode_row_arr(&[[0.0, 2.0], [3.0, 0.0]], 1), (1, [3.0, 0.0]));
        assert_eq!(DecodedRow { label: true, weight: 2.0, example_num: 0, target: 1.0 }.weights(), [0.0, 2.0]);
        assert_eq!(DecodedRow { label: false, weight: 2.0, example_num: 0, target: 0.0 }.weights(), [2.0, 0.0]);
        assert_eq!(DecodedRow { label: false, weight: 2.0, example_num: 0, target: 0.25 }.weights(), [1.5, 0.5]);
    }
}
