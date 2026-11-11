//! feature.h and feature.cc, in Rust.
//!
//! Jeremy Barnes, 29 March 2009. Copyright (c) 2009 Jeremy Barnes. This file is part of MLDB. Copyright 2015 mldb.ai
//! inc. Apache-2.0 (see the LICENSE at `accel/src/forest/mldb/`).
//!
//! Definition of a feature.

use std::fmt;

pub type FeatureId = u32;

/// This structure identifies a single feature. This is a simple structure with a static type, into which the
/// definition for a "feature" is encoded by each feature space.
///
/// It maintains three fields: one for the type, and two for the arguments. This should be enough for bigrams, etc,
/// without making it too heavy. These fields *mean* nothing at all to the machine learning algorithms; the names
/// (type, arg1, arg2) are simply mnemonic. Different features with the same value will be interpreted differently
/// by different feature sets: its meaning depends upon its feature set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Feature {
    pub args: [FeatureId; 3],
}

impl Feature {
    pub const fn new(type_: FeatureId, arg1: FeatureId, arg2: FeatureId) -> Self {
        Feature { args: [type_, arg1, arg2] }
    }

    pub fn type_(&self) -> FeatureId {
        self.args[0]
    }
    pub fn set_type(&mut self, v: FeatureId) {
        self.args[0] = v;
    }
    pub fn arg1(&self) -> FeatureId {
        self.args[1]
    }
    pub fn set_arg1(&mut self, v: FeatureId) {
        self.args[1] = v;
    }
    pub fn arg2(&self) -> FeatureId {
        self.args[2]
    }
    pub fn set_arg2(&mut self, v: FeatureId) {
        self.args[2] = v;
    }

    pub fn hash(&self) -> usize {
        let (id1, id2, id3) = (self.type_() as usize, self.arg1() as usize, self.arg2() as usize);
        (3usize.wrapping_mul(id1).wrapping_add(31usize.wrapping_mul(id2)).wrapping_add(71usize.wrapping_mul(id3)))
            .wrapping_mul(103usize.wrapping_mul(id1).wrapping_add(1411usize.wrapping_mul(id2)).wrapping_add(3179usize.wrapping_mul(id3)))
    }

    pub fn print(&self) -> String {
        format!("({} {} {})", self.type_() as i32, self.arg1() as i32, self.arg2() as i32)
    }
}

impl std::hash::Hash for Feature {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        h.write_usize(Feature::hash(self));
    }
}

impl fmt::Display for Feature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.print())
    }
}

/// This is the feature used when we want one we are sure will never be used as a real feature code.
pub const MISSING_FEATURE: Feature = Feature::new(u32::MAX, u32::MAX, 0);
