//! dataset_feature_space.h and the parts of dataset_feature_space.cc training and prediction need, in Rust.
//!
//! Jeremy Barnes, 13 March 2015. Copyright (c) 2015 mldb.ai inc. This file is part of MLDB. Apache-2.0 (see the
//! LICENSE at `forest/src/mldb/`).
//!
//! Feature space for datasets to allow training of classifiers: one feature per column, each column with its info,
//! and (when bucketized) its bit-packed bucket numbers and bucket descriptions, which the trainer reads.
//!
//! Upstream builds the space from an MLDB `Dataset`; here from the columns' values (`from_columns`, over
//! `ColumnInfo::from_values`: `getColumnInfo` bucketized, with `Dataset::getColumnBuckets` from core/dataset.cc), or
//! the caller adds its columns (`add_column`). Prediction buckets values as training did (`ColumnInfo::bucket_of`,
//! `bucketize`) rather than encoding them (`encodeFeatureValue`), which disagreed with training (see `bucket_of`).
//! The parts that serve JML's `Feature_Space` interface over MLDB's dataset (`encodeFeature`, `encodeLabel`,
//! serialization, the version two hash mapping) stay in the .cc; the unbucketized column info is not brought over
//! (the trainer needs buckets). The
//! feature to column hash mapping is the one dataset_feature_space.cc defines (type 1, then the hash's high and low
//! 32 bits), not the "type 0" its header's comment describes.

use crate::bucket::{BucketDescriptions, BucketList};
use crate::jml::feature::{Feature, MISSING_FEATURE};
use std::collections::HashMap;
use std::fmt;

/// A 64-bit hash of a column's name.
pub type ColumnHash = u64;

/// The hash of a column name: the first 8 bytes of its SHA-256 (stable across runs and machines).
pub fn column_hash(name: &str) -> ColumnHash {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(name.as_bytes());
    u64::from_le_bytes(d[..8].try_into().unwrap())
}

/// JML's `Feature_Type` (feature_info.h).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FeatureType {
    /// We have not yet determined the feature type.
    #[default]
    Unknown,
    /// Feature is present or not present; value unimportant.
    Presence,
    /// Feature is true (1.0) or false (0.0).
    Boolean,
    /// Feature is categorical; ordering makes no sense.
    Categorical,
    /// Feature is real valued.
    Real,
    /// Was PROB.
    Unused1,
    /// Feature is inutile and should be ignored.
    Inutile,
    /// Feature is an open categorical feature.
    String,
}

#[derive(Clone, Debug, Default)]
pub struct ColumnInfo {
    pub column_name: String,
    /// The feature's info (its `Feature_Type`).
    pub info: FeatureType,
    pub index: i32,
    pub distinct_values: i32,
    /// These are only filled in if bucketize is true on construction.
    pub buckets: BucketList,
    pub bucket_descriptions: BucketDescriptions,
}

impl ColumnInfo {
    pub fn print(&self) -> String {
        format!("[Column '{}'; Info: {:?}; distinctVals: {}]", self.column_name, self.info, self.distinct_values)
    }
}

impl fmt::Display for ColumnInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.print())
    }
}

/// A feature space whose information comes from a table of columns. There is one feature per column.
#[derive(Clone, Debug, Default)]
pub struct DatasetFeatureSpace {
    pub column_info: HashMap<ColumnHash, ColumnInfo>,
    pub label_info: FeatureType,
    /// The fewest rows a tree's leaf may have when training over this space (rerail's addition: upstream splits
    /// down to single rows; 0 is 1). It rides with the label because the split criterion is the label's
    /// (`SplitCriterion::of`).
    pub min_leaf_examples: u32,
}

impl DatasetFeatureSpace {
    /// Adds a column; its hash.
    pub fn add_column(&mut self, mut info: ColumnInfo) -> ColumnHash {
        let hash = column_hash(&info.column_name);
        if info.index < 0 {
            info.index = self.column_info.len() as i32;
        }
        self.column_info.insert(hash, info);
        hash
    }

    /// In JML, features are represented by 3 arbitrary 32 bit integers. JML doesn't try to interpret them; if all 3
    /// are the same, it's the same feature, otherwise it's not. Here a `ColumnHash` (a 64 bit integer) is encoded
    /// into a `Feature` as type 1, then the high 32 bits of the hash, then the low 32 bits.
    pub fn get_hash_raw(feature: Feature) -> Result<ColumnHash, String> {
        if feature == MISSING_FEATURE {
            return Ok(0);
        }
        if feature.type_() != 1 {
            return Err(format!("feature {feature} is not a column's"));
        }
        Ok((feature.arg1() as u64) << 32 | feature.arg2() as u64)
    }

    pub fn get_hash(&self, feature: Feature) -> Result<ColumnHash, String> {
        Self::get_hash_raw(feature)
    }

    /// Undo the mapping from `get_hash`.
    pub fn get_feature_raw(hash: ColumnHash) -> Feature {
        let result = Feature::new(1, (hash >> 32) as u32, hash as u32);
        debug_assert_eq!(Self::get_hash_raw(result), Ok(hash));
        result
    }

    pub fn get_feature(&self, hash: ColumnHash) -> Feature {
        Self::get_feature_raw(hash)
    }

    /// The feature of the column called `name`.
    pub fn get_feature_by_name(&self, name: &str) -> Feature {
        Self::get_feature_raw(column_hash(name))
    }

    /// `info(feature)`: the type of the feature's column.
    pub fn info(&self, feature: &Feature) -> Result<FeatureType, String> {
        let hash = self.get_hash(*feature)?;
        self.column_info.get(&hash).map(|c| c.info).ok_or_else(|| format!("feature {feature} not in the feature space"))
    }

    /// `print(feature)`: the column's name.
    pub fn print_feature(&self, feature: &Feature) -> String {
        match self.get_hash(*feature).ok().and_then(|h| self.column_info.get(&h)) {
            Some(c) => c.column_name.clone(),
            None => feature.print(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_space_columns_and_features() {
        let mut fs = DatasetFeatureSpace::default();
        let h = fs.add_column(ColumnInfo { column_name: "units".into(), info: FeatureType::Real, index: -1, ..Default::default() });
        let f = fs.get_feature(h);
        assert_eq!(f.type_(), 1);
        assert_eq!(fs.get_hash(f).unwrap(), h);
        assert_eq!(fs.get_feature_by_name("units"), f);
        assert_eq!(fs.info(&f).unwrap(), FeatureType::Real);
        assert_eq!(fs.print_feature(&f), "units");
        assert_eq!(DatasetFeatureSpace::get_hash_raw(MISSING_FEATURE).unwrap(), 0);
    }
}

/// The bucket a value outside the training's buckets is given at prediction (a category not seen in training, a
/// null in a column that had none, a value of another kind): one past the column's buckets. It is on the right of
/// every split: above every ordinal split, and equal to no categorical split.
pub fn unseen_bucket(descriptions: &BucketDescriptions) -> u32 {
    descriptions.num_buckets() as u32
}

impl ColumnInfo {
    /// A column's info from its values, bucketized (`getColumnInfo` with `bucketize`, over the column's buckets as
    /// `Dataset::getColumnBuckets` makes them, from core/dataset.cc): the distinct values described in up to
    /// `max_num_buckets` buckets (upstream asked for 255), each row's bucket, and the column real when it has
    /// numbers (an error when it also has strings), categorical when it has strings.
    pub fn from_values(column_name: &str, values: &[crate::bucket::CellValue], max_num_buckets: i32) -> Result<ColumnInfo, String> {
        use crate::bucket::{CellValue, WritableBucketList};
        use crate::memory_region::MemorySerializer;
        let mut sorted: Vec<(&CellValue, u32)> = values.iter().zip(0u32..).collect();
        sorted.sort();
        let mut distinct_values: Vec<CellValue> = Vec::new();
        let mut starts_at = Vec::new();
        for (i, (v, _)) in sorted.iter().enumerate() {
            if distinct_values.last() != Some(*v) {
                distinct_values.push((*v).clone());
                starts_at.push(i);
            }
        }
        starts_at.push(sorted.len());
        let mut descriptions = BucketDescriptions::default();
        descriptions.initialize(distinct_values.clone(), max_num_buckets)?;
        let mut bucket_numbers = vec![u32::MAX; sorted.len()];
        for (i, v) in distinct_values.iter().enumerate() {
            let bucket = descriptions.get_bucket(v)?;
            for (_, row) in &sorted[starts_at[i]..starts_at[i + 1]] {
                bucket_numbers[*row as usize] = bucket;
            }
        }
        let mut s = MemorySerializer;
        let mut buckets = WritableBucketList::new(values.len(), descriptions.num_buckets() as u32, &mut s);
        for &b in &bucket_numbers {
            buckets.write(b);
        }
        let info = if descriptions.numeric.active {
            if !descriptions.strings.buckets.is_empty() {
                let some: Vec<&String> = descriptions.strings.buckets.iter().take(100).collect();
                return Err(format!("This classifier can't train on column {column_name} which has both string and numeric values (strings {some:?}). Consider casting it to a string, or splitting it into a numeric-or-null and a string-or-null column."));
            }
            FeatureType::Real
        } else if !descriptions.strings.buckets.is_empty() {
            FeatureType::Categorical
        } else {
            // Only nulls (upstream left the type unset here).
            FeatureType::Inutile
        };
        Ok(ColumnInfo { column_name: column_name.to_string(), info, index: -1, distinct_values: descriptions.num_buckets() as i32, buckets: buckets.freeze(&mut s), bucket_descriptions: descriptions })
    }

    /// The bucket of a value at prediction: the training's bucket, or `unseen_bucket` for a value the training's
    /// buckets do not describe. Upstream encoded values instead (`encodeFeatureValue`: numbers as themselves,
    /// categories as their place among the column's strings, nulls as NaN) and compared them with the node's split
    /// value; that disagrees with the buckets training split on for categories when the column also has nulls or
    /// numbers (the strings' buckets are offset), for nulls (training put them in bucket 0, on the left; prediction
    /// took the missing branch, an empty leaf), and for values on a split point of a column with more distinct
    /// values than buckets (those split points are values, which the buckets put on the left and `<` on the right).
    pub fn bucket_of(&self, value: &crate::bucket::CellValue) -> u32 {
        self.bucket_descriptions.get_bucket(value).unwrap_or_else(|_| unseen_bucket(&self.bucket_descriptions))
    }

    /// A column of values in this column's buckets (`bucket_of`), bit-packed, with room for the unseen bucket.
    pub fn bucketize(&self, values: &[crate::bucket::CellValue]) -> BucketList {
        use crate::bucket::WritableBucketList;
        use crate::memory_region::MemorySerializer;
        let mut s = MemorySerializer;
        let mut w = WritableBucketList::new(values.len(), unseen_bucket(&self.bucket_descriptions) + 1, &mut s);
        for v in values {
            w.write(self.bucket_of(v));
        }
        w.freeze(&mut s)
    }
}

impl DatasetFeatureSpace {
    /// The feature space over the columns given, each bucketized (`ColumnInfo::from_values`, 255 buckets), in
    /// parallel; a column's index is its place in the list (upstream: the dataset's columns that were asked for).
    pub fn from_columns(columns: &[(String, Vec<crate::bucket::CellValue>)], label_info: FeatureType) -> Result<Self, String> {
        use rayon::prelude::*;
        let infos: Vec<ColumnInfo> = columns.par_iter().map(|(name, values)| ColumnInfo::from_values(name, values, 255)).collect::<Result<_, _>>()?;
        let mut fs = DatasetFeatureSpace { label_info, ..Default::default() };
        for (i, mut info) in infos.into_iter().enumerate() {
            info.index = i as i32;
            if fs.column_info.contains_key(&column_hash(&info.column_name)) {
                return Err(format!("column {} given twice", info.column_name));
            }
            fs.add_column(info);
        }
        Ok(fs)
    }

    /// The column at `index`.
    pub fn column(&self, index: usize) -> Option<&ColumnInfo> {
        self.column_info.values().find(|c| c.index == index as i32)
    }
}

/// A column's values as the device takes them: numbers (NaN for a null), or categories as codes into their names
/// (`NULL_CODE` for a null), as a column store holds them.
#[derive(Clone, Debug)]
pub enum ColumnData {
    Numeric(Vec<f64>),
    Categorical { codes: Vec<u32>, names: Vec<String> },
}

/// A null among a categorical column's codes.
pub const NULL_CODE: u32 = u32::MAX;

impl ColumnData {
    /// The values of a column of cells: numeric when they are numbers (and nulls), categorical when strings (and
    /// nulls); both is the error `ColumnInfo::from_values` gives, and other kinds are not taken by the device.
    pub fn from_values(column_name: &str, values: &[crate::bucket::CellValue]) -> Result<ColumnData, String> {
        use crate::bucket::CellValue;
        let numeric = values.iter().any(|v| matches!(v, CellValue::Integer(_) | CellValue::Float(_)));
        let strings = values.iter().any(|v| matches!(v, CellValue::String(_)));
        if numeric && strings {
            return Err(format!("This classifier can't train on column {column_name} which has both string and numeric values. Consider casting it to a string, or splitting it into a numeric-or-null and a string-or-null column."));
        }
        if values.iter().any(|v| !matches!(v, CellValue::Empty | CellValue::Integer(_) | CellValue::Float(_) | CellValue::String(_))) {
            return Err(format!("column {column_name}: only numbers, strings and nulls are built on the device"));
        }
        if !strings {
            return Ok(ColumnData::Numeric(values.iter().map(|v| if let CellValue::Empty = v { f64::NAN } else { v.to_double() }).collect()));
        }
        let mut names: Vec<String> = Vec::new();
        let mut index: HashMap<&str, u32> = HashMap::new();
        let mut codes = Vec::with_capacity(values.len());
        for v in values {
            codes.push(match v {
                CellValue::String(s) => *index.entry(s.as_str()).or_insert_with(|| {
                    names.push(s.clone());
                    names.len() as u32 - 1
                }),
                _ => NULL_CODE,
            });
        }
        Ok(ColumnData::Categorical { codes, names })
    }

    pub fn len(&self) -> usize {
        match self {
            ColumnData::Numeric(v) => v.len(),
            ColumnData::Categorical { codes, .. } => codes.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A number's key in the order of `<` (-0 as +0; never `NULL_ORDER_KEY`, i64::MIN): the bits, with a negative
/// number's magnitude bits flipped, so that keys compare as signed integers as the numbers do.
pub fn order_key(v: f64) -> i64 {
    let v = if v == 0.0 { 0.0 } else { v };
    let b = v.to_bits() as i64;
    if b < 0 { b ^ i64::MAX } else { b }
}

/// The number of an `order_key`.
pub fn from_order_key(k: i64) -> f64 {
    f64::from_bits((if k < 0 { k ^ i64::MAX } else { k }) as u64)
}

/// A null's key (`NULL_ORDER_KEY` in the kernels).
pub const NULL_ORDER_KEY: i64 = i64::MIN;

impl ColumnInfo {
    /// `from_values` on the device, from typed values: the same info, bucket descriptions and buckets.
    /// - Numeric: the rows' order keys sorted on the device (the engine's radix sort, `gpu::join::Keyed`), whose runs
    ///   are the distinct values; their split points as `BucketDescriptions` places them (`max_num_buckets`); then
    ///   each row's bucket by its key against the split points' keys (`numericBucketsKernel`).
    /// - Categorical: the codes present (`markCodesKernel`), their names' buckets, then each row's
    ///   (`categoricalBucketsKernel`).
    /// The buckets are packed on the device (`packBucketsKernel`). The host's work is a pass for the keys, the split
    /// points over the distinct values, and one copy of the packed words.
    pub fn from_column_device(column_name: &str, data: &ColumnData, max_num_buckets: i32) -> Result<ColumnInfo, String> {
        use crate::compute_kernel_grid::ComputeKernelArgument::{Buffer as B, Int};
        use crate::compute_kernel_metal::{alloc_uninitialized_array_sync, transfer_to_device_immutable, transfer_to_host_sync, MetalComputeQueue};
        use crate::randomforest_kernels_grid::kernel;
        let shared = crate::gpu::shared::get().ok_or("no Metal device")?;
        let device = &shared.device;
        let n = data.len();
        let run = |queue: &MetalComputeQueue, name: &str, args: &[(&str, crate::compute_kernel_grid::ComputeKernelArgument)], grid: u32| -> Result<(), String> {
            let k = kernel(name)?;
            let mut ctx = queue.new_bind_context(k.function);
            k.enqueue(args, &[grid], &mut ctx)
        };
        let mut descriptions = BucketDescriptions::default();
        let buckets = alloc_uninitialized_array_sync::<u32>(device, n);
        let mut queue = MetalComputeQueue::new(&shared.queue);
        match data {
            ColumnData::Numeric(values) => {
                let mut has_nulls = false;
                let keys: Vec<i64> = values.iter().map(|&v| if v.is_nan() { has_nulls = true; NULL_ORDER_KEY } else { order_key(v) }).collect();
                let mut distinct: Vec<f64> = Vec::new();
                if n > 0 {
                    let sorted = crate::distinct_keys(&keys)?;
                    distinct = sorted.into_iter().filter(|&k| k != NULL_ORDER_KEY).map(from_order_key).collect();
                }
                descriptions.initialize_from(has_nulls, distinct, Vec::new(), max_num_buckets)?;
                let split_keys: Vec<i64> = descriptions.numeric.splits.iter().map(|&s| order_key(s)).collect();
                let d_keys = transfer_to_device_immutable(device, &keys);
                let d_splits = transfer_to_device_immutable(device, &split_keys);
                run(&queue, "numericBuckets", &[
                    ("numRows", Int(n as i64)),
                    ("numSplits", Int(split_keys.len() as i64)),
                    ("numericOffset", Int(descriptions.numeric.offset as i64)),
                    ("keys", B(&d_keys)),
                    ("splitKeys", B(&d_splits)),
                    ("bucketsOut", B(&buckets)),
                ], n as u32)?;
            }
            ColumnData::Categorical { codes, names } => {
                let nc = names.len();
                let d_codes = transfer_to_device_immutable(device, codes);
                let present = transfer_to_device_immutable(device, &vec![0u32; nc + 1]);
                run(&queue, "markCodes", &[("numRows", Int(n as i64)), ("numCodes", Int(nc as i64)), ("codes", B(&d_codes)), ("presentOut", B(&present))], n as u32)?;
                queue.flush().await_()?;
                let present = transfer_to_host_sync::<u32>(&present, nc + 1);
                let used: Vec<String> = (0..nc).filter(|&c| present[c] != 0).map(|c| names[c].clone()).collect();
                descriptions.initialize_from(present[nc] != 0, Vec::new(), used, max_num_buckets)?;
                let mut code_buckets = vec![0u32; nc + 1];
                for c in (0..nc).filter(|&c| present[c] != 0) {
                    code_buckets[c] = descriptions.get_bucket(&crate::bucket::CellValue::String(names[c].clone()))?;
                }
                let d_code_buckets = transfer_to_device_immutable(device, &code_buckets);
                run(&queue, "categoricalBuckets", &[("numRows", Int(n as i64)), ("numCodes", Int(nc as i64)), ("codes", B(&d_codes)), ("codeBuckets", B(&d_code_buckets)), ("bucketsOut", B(&buckets))], n as u32)?;
            }
        }
        let num_buckets = descriptions.num_buckets() as u32;
        let entry_bits = crate::bucket::highest_bit(num_buckets, 0) as u32 + 1;
        let num_words = crate::bucket::WritableBucketList::words_required(n, num_buckets);
        let words = alloc_uninitialized_array_sync::<u32>(device, num_words);
        run(&queue, "packBuckets", &[
            ("numRows", Int(n as i64)),
            ("entryBits", Int(entry_bits as i64)),
            ("numWords", Int(num_words as i64)),
            ("buckets", B(&buckets)),
            ("wordsOut", B(&words)),
        ], num_words as u32)?;
        queue.flush().await_()?;
        let words = transfer_to_host_sync::<u32>(&words, num_words);
        let mut s = crate::memory_region::MemorySerializer;
        let buckets = BucketList::from_packed(n, num_buckets, &words, &mut s);
        let info = if descriptions.numeric.active {
            FeatureType::Real
        } else if !descriptions.strings.buckets.is_empty() {
            FeatureType::Categorical
        } else {
            FeatureType::Inutile
        };
        Ok(ColumnInfo { column_name: column_name.to_string(), info, index: -1, distinct_values: num_buckets as i32, buckets, bucket_descriptions: descriptions })
    }
}

impl DatasetFeatureSpace {
    /// `from_columns` on the device (`ColumnInfo::from_column_device`, 255 buckets), the columns in parallel.
    pub fn from_columns_device(columns: &[(String, ColumnData)], label_info: FeatureType) -> Result<Self, String> {
        use rayon::prelude::*;
        let infos: Vec<ColumnInfo> = columns.par_iter().map(|(name, data)| ColumnInfo::from_column_device(name, data, 255)).collect::<Result<_, _>>()?;
        let mut fs = DatasetFeatureSpace { label_info, ..Default::default() };
        for (i, mut info) in infos.into_iter().enumerate() {
            info.index = i as i32;
            if fs.column_info.contains_key(&column_hash(&info.column_name)) {
                return Err(format!("column {} given twice", info.column_name));
            }
            fs.add_column(info);
        }
        Ok(fs)
    }
}

#[cfg(test)]
mod device_tests {
    use super::*;
    use crate::bucket::CellValue;

    fn h(i: usize, k: u64) -> u64 {
        let mut x = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ k.wrapping_mul(0xD6E8_FEB8_6659_FD93);
        x ^= x >> 31;
        x = x.wrapping_mul(0x2C1B_3C6D_297A_2D39);
        x ^ (x >> 29)
    }

    /// The order keys order as the numbers do, and come back as them.
    #[test]
    fn order_keys_order_as_numbers() {
        let mut v = vec![f64::NEG_INFINITY, -1e300, -2.5, -1.0, -f64::MIN_POSITIVE, -0.0, 0.0, 5e-324, 1.0, 1.5, 1e300, f64::INFINITY];
        v.extend((0..1000).map(|i| (h(i, 1) as i64 as f64) * 1e-12));
        for a in &v {
            assert_eq!(from_order_key(order_key(*a)), if *a == 0.0 { 0.0 } else { *a });
            assert_ne!(order_key(*a), NULL_ORDER_KEY);
            for b in &v {
                assert_eq!(a < b, order_key(*a) < order_key(*b), "{a} {b}");
            }
        }
    }

    /// The device builds every column as the host does: the same info, bucket descriptions and packed buckets.
    #[test]
    fn device_columns_are_the_host_s() {
        if crate::gpu::shared::get().is_none() {
            eprintln!("no Metal device: skipped");
            return;
        }
        let cats = ["red", "green", "blue", "grey", "a", "zz"];
        let n = 5003;
        let columns: Vec<(&str, Vec<CellValue>)> = vec![
            ("many reals", (0..n).map(|i| CellValue::Float((h(i, 1) % 100_000) as f64 / 7.0 - 3000.0)).collect()),
            ("reals and nulls", (0..n).map(|i| if h(i, 2) % 10 == 0 { CellValue::Empty } else { CellValue::Float((h(i, 3) % 50) as f64 * 0.5 - 3.0) }).collect()),
            ("zeros", (0..n).map(|i| CellValue::Float(if i % 2 == 0 { -0.0 } else { 0.0 })).collect()),
            ("ints", (0..n).map(|i| CellValue::Integer((h(i, 6) % 6) as i64 - 2)).collect()),
            ("big ints", (0..n).map(|i| CellValue::Integer(h(i, 7) as i64)).collect()),
            ("categories and nulls", (0..n).map(|i| if h(i, 4) % 9 == 0 { CellValue::Empty } else { CellValue::String(cats[(h(i, 5) % 6) as usize].into()) }).collect()),
            ("many categories", (0..n).map(|i| CellValue::String(format!("form{}", h(i, 8) % 700))).collect()),
            ("only nulls", vec![CellValue::Empty; n]),
            ("constant", vec![CellValue::Integer(7); n]),
            ("one row", vec![CellValue::Float(2.0)]),
            ("no rows", vec![]),
        ];
        for (name, values) in &columns {
            let host = ColumnInfo::from_values(name, values, 255).unwrap();
            let data = ColumnData::from_values(name, values).unwrap();
            let device = ColumnInfo::from_column_device(name, &data, 255).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(host.info, device.info, "{name}");
            assert_eq!(host.distinct_values, device.distinct_values, "{name}");
            assert_eq!(format!("{:?}", host.bucket_descriptions), format!("{:?}", device.bucket_descriptions), "{name}");
            assert_eq!((host.buckets.entry_bits, host.buckets.num_buckets, host.buckets.num_entries), (device.buckets.entry_bits, device.buckets.num_buckets, device.buckets.num_entries), "{name}");
            let used = (host.buckets.entry_bits as usize * values.len()).div_ceil(32);
            assert_eq!(&host.buckets.storage.data()[..used], &device.buckets.storage.data()[..used], "{name}");
            for r in 0..values.len() as u32 {
                assert_eq!(host.buckets.get(r), device.buckets.get(r), "{name} row {r}");
            }
        }
        assert!(ColumnData::from_values("mixed", &[CellValue::Integer(1), CellValue::String("a".into())]).is_err());
    }
}
