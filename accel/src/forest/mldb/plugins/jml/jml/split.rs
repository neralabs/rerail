//! split.h, split_fwd.h and split.cc, in Rust.
//!
//! Jeremy Barnes, 5 March 2009. Copyright (c) 2009 Jeremy Barnes. This file is part of MLDB. Copyright 2015 mldb.ai
//! inc. Apache-2.0 (see the LICENSE at `accel/src/forest/mldb/`).
//!
//! Split class to divide a feature set up based upon the value of one feature.
//!
//! JML's `Feature_Space`, `Feature_Set`, `Feature_Info`, `Optimization_Info` and `DB::Store` are not brought over:
//! the constructor from a feature space, `print`, `serialize`/`reconstitute`, `validate(fs)` and `apply` over a
//! `Feature_Set` need them. `optimize` takes the feature-to-index map that `Optimization_Info` held, and
//! `get_op_from_feature` takes whether the feature's type is categorical or string.

use super::feature::{Feature, MISSING_FEATURE};
use std::collections::BTreeMap;
use std::fmt;

/// The third outcome of `apply` (split_fwd.h).
pub const MISSING: i32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Op {
    /// Test is val(feature) < value.
    Less = 0,
    /// Test is val(feature) == value.
    Equal = 1,
    /// Test is always true if feature not missing.
    NotMissing = 2,
}

impl Op {
    pub fn print(self) -> &'static str {
        match self {
            Op::Less => "LESS",
            Op::Equal => "EQUAL",
            Op::NotMissing => "NOT_MISSING",
        }
    }
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.print())
    }
}

/// Weight on true, false and missing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Weights {
    val: [f32; 3],
}

impl std::ops::Index<i32> for Weights {
    type Output = f32;
    fn index(&self, i: i32) -> &f32 {
        &self.val[i as usize]
    }
}

impl std::ops::IndexMut<i32> for Weights {
    fn index_mut(&mut self, i: i32) -> &mut f32 {
        &mut self.val[i as usize]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Split {
    /// Feature to obtain.
    feature: Feature,
    /// Value to test against; **must not be NaN**.
    split_val: f32,
    /// What operation to apply?
    op: Op,
    /// Is this optimized for predict?
    opt: bool,
    /// Index for an optimized predict (23 bits upstream).
    idx: u32,
}

impl Default for Split {
    fn default() -> Self {
        let s = Split { feature: MISSING_FEATURE, split_val: 0.0, op: Op::Less, opt: false, idx: 0 };
        s.validate().expect("default split");
        s
    }
}

impl Split {
    pub fn new(feature: Feature, split_val: f32, op: Op) -> Result<Self, String> {
        let mut s = Split { feature, split_val, op, opt: false, idx: 0 };
        if split_val == f32::NEG_INFINITY && op == Op::Less {
            s.split_val = 0.0;
            s.op = Op::NotMissing;
        }
        s.validate()?;
        Ok(s)
    }

    pub fn swap(&mut self, other: &mut Split) {
        std::mem::swap(&mut self.feature, &mut other.feature);
        std::mem::swap(&mut self.split_val, &mut other.split_val);
        std::mem::swap(&mut self.op, &mut other.op);
        std::mem::swap(&mut self.opt, &mut other.opt);
        std::mem::swap(&mut self.idx, &mut other.idx);
    }

    /// Will return true (1), false (0) or `MISSING`.
    #[inline(always)]
    pub fn apply_value(&self, feature_val: f32) -> i32 {
        if feature_val.is_nan() {
            return MISSING;
        }
        let equal = feature_val == self.split_val;
        let less = feature_val < self.split_val;
        let all = (less as i32) | ((equal as i32) << 1) | 4;
        ((all & (1 << self.op as i32)) != 0) as i32
    }

    /// Weights of an optimized split over a dense feature vector.
    pub fn apply_dense(&self, fset: &[f32], weight: f32) -> Result<Weights, String> {
        let mut result = Weights::default();
        self.apply_dense_into(fset, &mut result, weight)?;
        Ok(result)
    }

    /// Update the given weights with this split, over a dense feature vector (an optimized split).
    pub fn apply_dense_into(&self, fset: &[f32], weights: &mut Weights, weight: f32) -> Result<(), String> {
        if !self.opt {
            return Err("Split::apply(): wrong method for unoptimized split".into());
        }
        weights[self.apply_value(fset[self.idx as usize])] += weight;
        Ok(())
    }

    /// Update the given weights with the feature's values (all of them values of this split's feature): none is
    /// missing, one goes to its side, several share the weight.
    pub fn apply_values(&self, values: &[f32], weights: &mut Weights, weight: f32) {
        match values {
            [] => weights[MISSING] += weight,
            [v] => weights[self.apply_value(*v)] += weight,
            vs => {
                let f = weight / vs.len() as f32;
                for v in vs {
                    weights[self.apply_value(*v)] += f;
                }
            }
        }
    }

    /// Optimize for the given feature to optimized index map (`Optimization_Info::feature_to_optimized_index`).
    pub fn optimize(&mut self, feature_to_optimized_index: &BTreeMap<Feature, i32>) -> Result<(), String> {
        let idx = *feature_to_optimized_index.get(&self.feature).ok_or("Split::optimize(): feature not found")?;
        self.idx = idx as u32;
        self.opt = true;
        Ok(())
    }

    /// Given the type of a feature, figure out which type of relationship it should have.
    pub fn get_op_from_feature(categorical_or_string: bool) -> Op {
        if categorical_or_string { Op::Equal } else { Op::Less }
    }

    pub fn feature(&self) -> &Feature {
        &self.feature
    }
    pub fn split_val(&self) -> f32 {
        self.split_val
    }
    pub fn op(&self) -> Op {
        self.op
    }

    /// Check that everything is OK.
    pub fn validate(&self) -> Result<(), String> {
        if self.split_val.is_nan() {
            return Err(format!("bad split val (feature {}, op {})", self.feature, self.op));
        }
        Ok(())
    }
}

impl PartialEq for Split {
    fn eq(&self, other: &Self) -> bool {
        self.split_val == other.split_val && self.feature == other.feature && self.op == other.op
    }
}

impl PartialOrd for Split {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.feature.cmp(&other.feature).then(self.split_val.total_cmp(&other.split_val)).then(self.op.cmp(&other.op)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_apply_ops() {
        let f = Feature::new(1, 2, 3);
        let less = Split::new(f, 0.5, Op::Less).unwrap();
        assert_eq!(less.apply_value(0.4), 1);
        assert_eq!(less.apply_value(0.5), 0);
        assert_eq!(less.apply_value(f32::NAN), MISSING);
        let eq = Split::new(f, 2.0, Op::Equal).unwrap();
        assert_eq!(eq.apply_value(2.0), 1);
        assert_eq!(eq.apply_value(3.0), 0);
        let nm = Split::new(f, f32::NEG_INFINITY, Op::Less).unwrap();
        assert_eq!(nm.op(), Op::NotMissing);
        assert_eq!(nm.apply_value(-1e9), 1);
        assert!(Split::new(f, f32::NAN, Op::Less).is_err());
        let mut w = Weights::default();
        less.apply_values(&[0.1, 0.9], &mut w, 1.0);
        assert_eq!((w[1], w[0], w[MISSING]), (0.5, 0.5, 0.0));
        let mut o = less;
        o.optimize(&BTreeMap::from([(f, 1)])).unwrap();
        assert_eq!(o.apply_dense(&[9.0, 0.1], 2.0).unwrap()[1], 2.0);
    }
}
