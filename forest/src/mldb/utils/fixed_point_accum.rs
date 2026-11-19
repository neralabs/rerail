//! fixed_point_accum.h (and .cc), in Rust.
//!
//! Jeremy Barnes, 1 April 2009. Copyright (c) 2009 Jeremy Barnes. This file is part of MLDB. Copyright 2015 mldb.ai
//! inc. Apache-2.0 (see `../LICENSE`).
//!
//! Fixed-point accurate accumulators.

use std::fmt;
use std::ops::{Add, AddAssign, Div, Sub, SubAssign};

/// `throwFixedPointOverflow` (fixed_point_accum.cc).
#[cold]
#[inline(never)]
pub fn throw_fixed_point_overflow() -> ! {
    panic!("fixed point overflow");
}

/// A structure to accumulate values between zero and one in a single 32 bit integer.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FixedPointAccum32Unsigned {
    pub rep: u32,
}

impl FixedPointAccum32Unsigned {
    pub const VAL_2_REP: f32 = (1u64 << 31) as f32;
    pub const REP_2_VAL: f32 = 1.0 / (1u64 << 31) as f32;
    pub const ADD_TO_ROUND: f32 = 1.0 / (1u64 << 32) as f32;
    pub const MAX_REP: u32 = u32::MAX;

    pub fn new(value: f32) -> Self {
        FixedPointAccum32Unsigned { rep: ((value + Self::ADD_TO_ROUND) * Self::VAL_2_REP) as u32 }
    }

    pub fn to_f32(self) -> f32 {
        self.rep as f32 * Self::REP_2_VAL
    }
}

impl From<f32> for FixedPointAccum32Unsigned {
    fn from(value: f32) -> Self {
        Self::new(value)
    }
}

impl AddAssign for FixedPointAccum32Unsigned {
    /// Saturates at `MAX_REP`.
    fn add_assign(&mut self, other: Self) {
        let new_rep = self.rep.wrapping_add(other.rep);
        self.rep = if new_rep < self.rep { Self::MAX_REP } else { new_rep };
    }
}

impl Add for FixedPointAccum32Unsigned {
    type Output = Self;
    fn add(mut self, other: Self) -> Self {
        self += other;
        self
    }
}

/// A signed fixed-point value with 30 fractional bits in 32 bits: the GPU kernels' `W32` weights.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FixedPointAccum32 {
    pub rep: i32,
}

impl FixedPointAccum32 {
    pub const VAL_2_REP: f32 = (1u64 << 30) as f32;
    pub const REP_2_VAL: f32 = 1.0 / (1u64 << 30) as f32;
    pub const ADD_TO_ROUND: f32 = 0.5 / (1u64 << 30) as f32;

    pub fn new(value: f32) -> Self {
        FixedPointAccum32 { rep: ((value + Self::ADD_TO_ROUND) * Self::VAL_2_REP) as i32 }
    }

    pub fn to_f32(self) -> f32 {
        self.rep as f32 * Self::REP_2_VAL
    }
}

impl From<f32> for FixedPointAccum32 {
    fn from(value: f32) -> Self {
        Self::new(value)
    }
}

impl From<FixedPointAccum32> for f32 {
    fn from(v: FixedPointAccum32) -> f32 {
        v.to_f32()
    }
}

impl AddAssign for FixedPointAccum32 {
    fn add_assign(&mut self, other: Self) {
        self.rep = self.rep.checked_add(other.rep).unwrap_or_else(|| throw_fixed_point_overflow());
    }
}

impl Add for FixedPointAccum32 {
    type Output = Self;
    fn add(mut self, other: Self) -> Self {
        self += other;
        self
    }
}

impl SubAssign for FixedPointAccum32 {
    fn sub_assign(&mut self, other: Self) {
        self.rep = self.rep.checked_sub(other.rep).unwrap_or_else(|| throw_fixed_point_overflow());
    }
}

impl Sub for FixedPointAccum32 {
    type Output = Self;
    fn sub(mut self, other: Self) -> Self {
        self -= other;
        self
    }
}

impl Div for FixedPointAccum32 {
    type Output = f32;
    fn div(self, other: Self) -> f32 {
        self.to_f32() / other.to_f32()
    }
}

impl Div<f32> for FixedPointAccum32 {
    type Output = f32;
    fn div(self, other: f32) -> f32 {
        self.to_f32() / other
    }
}

impl fmt::Display for FixedPointAccum32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.6} ({:016x})", self.to_f32() as f64, self.rep as i64)
    }
}

/// A signed fixed-point value with 60 fractional bits in 64 bits.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FixedPointAccum64 {
    pub hl: i64,
}

impl FixedPointAccum64 {
    pub const VAL_2_HL: f32 = 1.0 * (1u64 << 60) as f32;
    pub const HL_2_VAL: f32 = 1.0 / (1u64 << 60) as f32;
    pub const VAL_2_H: f32 = (1u64 << 28) as f32;
    pub const H_2_VAL: f32 = 1.0 / (1u64 << 28) as f32;
    pub const ADD_TO_ROUND: f32 = 0.5 / (1u64 << 60) as f32;

    pub fn new(value: f32) -> Self {
        FixedPointAccum64 { hl: ((value + Self::ADD_TO_ROUND) * Self::VAL_2_HL) as i64 }
    }

    pub const fn construct_from_int(i: i64) -> Self {
        FixedPointAccum64 { hl: i }
    }

    pub const fn max() -> Self {
        Self::construct_from_int(i64::MAX)
    }

    pub const fn min() -> Self {
        Self::construct_from_int(i64::MIN)
    }

    pub fn to_f32(self) -> f32 {
        (self.hl >> 32) as f32 * Self::H_2_VAL
    }
}

impl From<f32> for FixedPointAccum64 {
    fn from(value: f32) -> Self {
        Self::new(value)
    }
}

impl From<FixedPointAccum64> for f32 {
    fn from(v: FixedPointAccum64) -> f32 {
        v.to_f32()
    }
}

impl AddAssign for FixedPointAccum64 {
    fn add_assign(&mut self, other: Self) {
        self.hl = self.hl.checked_add(other.hl).unwrap_or_else(|| throw_fixed_point_overflow());
    }
}

impl SubAssign for FixedPointAccum64 {
    fn sub_assign(&mut self, other: Self) {
        self.hl = self.hl.checked_sub(other.hl).unwrap_or_else(|| throw_fixed_point_overflow());
    }
}

impl Add for FixedPointAccum64 {
    type Output = Self;
    fn add(mut self, other: Self) -> Self {
        self += other;
        self
    }
}

impl Add<f32> for FixedPointAccum64 {
    type Output = Self;
    fn add(self, other: f32) -> Self {
        self + FixedPointAccum64::new(other)
    }
}

impl Add<f64> for FixedPointAccum64 {
    type Output = Self;
    fn add(self, other: f64) -> Self {
        self + FixedPointAccum64::new(other as f32)
    }
}

impl Sub for FixedPointAccum64 {
    type Output = Self;
    fn sub(mut self, other: Self) -> Self {
        self -= other;
        self
    }
}

impl Div for FixedPointAccum64 {
    type Output = f32;
    fn div(self, other: Self) -> f32 {
        self.to_f32() / other.to_f32()
    }
}

impl Div<f32> for FixedPointAccum64 {
    type Output = f32;
    fn div(self, other: f32) -> f32 {
        self.to_f32() / other
    }
}

impl fmt::Display for FixedPointAccum64 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.6} ({:016x})", self.to_f32() as f64, self.hl)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The layouts the GPU kernels read (`W32` is three 4-byte words).
    #[test]
    fn fixed_point_layouts() {
        assert_eq!(std::mem::size_of::<FixedPointAccum32>(), 4);
        assert_eq!(std::mem::size_of::<FixedPointAccum64>(), 8);
    }

    #[test]
    fn fixed_point_round_trip_and_overflow() {
        let a = FixedPointAccum32::new(0.25);
        assert_eq!(a.rep, 1 << 28);
        assert_eq!((a + a).to_f32(), 0.5);
        assert_eq!((a - a).rep, 0);
        assert_eq!(FixedPointAccum64::new(0.5).to_f32(), 0.5);
        let mut u = FixedPointAccum32Unsigned { rep: u32::MAX - 1 };
        u += FixedPointAccum32Unsigned { rep: 5 };
        assert_eq!(u.rep, u32::MAX, "unsigned saturates");
        let r = std::panic::catch_unwind(|| FixedPointAccum32 { rep: i32::MAX } + FixedPointAccum32 { rep: 1 });
        assert!(r.is_err(), "signed overflow is an error");
    }
}
