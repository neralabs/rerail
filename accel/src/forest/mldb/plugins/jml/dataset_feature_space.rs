//! dataset_feature_space.h and the parts of dataset_feature_space.cc training and prediction need, in Rust.
//!
//! Jeremy Barnes, 13 March 2015. Copyright (c) 2015 mldb.ai inc. This file is part of MLDB. Apache-2.0 (see the
//! LICENSE at `accel/src/forest/mldb/`).
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

use crate::forest::bucket::{BucketDescriptions, BucketList};
use crate::forest::jml::feature::{Feature, MISSING_FEATURE};
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
    pub fn from_values(column_name: &str, values: &[crate::forest::bucket::CellValue], max_num_buckets: i32) -> Result<ColumnInfo, String> {
        use crate::forest::bucket::{CellValue, WritableBucketList};
        use crate::forest::memory_region::MemorySerializer;
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
    pub fn bucket_of(&self, value: &crate::forest::bucket::CellValue) -> u32 {
        self.bucket_descriptions.get_bucket(value).unwrap_or_else(|_| unseen_bucket(&self.bucket_descriptions))
    }

    /// A column of values in this column's buckets (`bucket_of`), bit-packed, with room for the unseen bucket.
    pub fn bucketize(&self, values: &[crate::forest::bucket::CellValue]) -> BucketList {
        use crate::forest::bucket::WritableBucketList;
        use crate::forest::memory_region::MemorySerializer;
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
    pub fn from_columns(columns: &[(String, Vec<crate::forest::bucket::CellValue>)], label_info: FeatureType) -> Result<Self, String> {
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
