//! dataset_feature_space.h, in Rust.
//!
//! Jeremy Barnes, 13 March 2015. Copyright (c) 2015 mldb.ai inc. This file is part of MLDB. Apache-2.0 (see the
//! LICENSE at `accel/src/forest/mldb/`).
//!
//! Feature space for datasets to allow training of classifiers: one feature per column, each column with its info,
//! and (when bucketized) its bit-packed bucket numbers and bucket descriptions, which the trainer reads.
//!
//! Upstream builds the space from an MLDB `Dataset` (dataset_feature_space.cc, not copied); here the caller adds
//! its columns (`add_column`). The parts that serve JML's `Feature_Space` interface over MLDB's dataset
//! (`encodeFeature`, `encodeLabel`, serialization, the version two hash mapping) are not brought over. The
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
