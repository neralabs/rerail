//! buckets.h and buckets.cc, in Rust.
//!
//! Jeremy Barnes, 12 September 2011. Copyright (c) 2011 Jeremy Barnes and mldb.ai inc. This file is part of MLDB.
//! Copyright 2015 mldb.ai inc. Apache-2.0 (see `../LICENSE`).
//!
//! Bucketing algorithms, extracted.

/// `bit_equal`: equal, or both NaN.
pub fn bit_equal(f1: f32, f2: f32) -> bool {
    (f1.is_nan() && f2.is_nan()) || f1 == f2
}

/// The type of the frequency distribution: (value, frequency), sorted by value (`sparse_distribution<float, float,
/// sorted_vector<float, float>>`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BucketFreqs {
    pub entries: Vec<(f32, f32)>,
}

impl BucketFreqs {
    /// From pairs; sorted by value.
    pub fn new(mut pairs: Vec<(f32, f32)>) -> Self {
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
        BucketFreqs { entries: pairs }
    }

    pub fn size(&self) -> usize {
        self.entries.len()
    }

    /// The sum of the frequencies.
    pub fn total(&self) -> f64 {
        self.entries.iter().map(|e| e.1 as f64).sum()
    }
}

/// Structure describing the set of buckets for one value.
#[derive(Clone, Debug, Default)]
pub struct BucketInfo {
    pub initialized: bool,
    /// Bucket numbers, sorted by example.
    pub buckets: Vec<u16>,
    /// Split points between the buckets.
    pub splits: Vec<f32>,
}

/// Create a distribution of bucket split points where we have few enough values that each element can go in its own
/// bucket. This usually occurs with categorical and boolean features.
pub fn bucket_dist_full(result: &mut Vec<f32>, freqs: &BucketFreqs) {
    result.clear();
    result.reserve(freqs.size().saturating_sub(1));
    // We want the split points, which are half way in between the current value and the next one.
    for w in freqs.entries.windows(2) {
        result.push((w[0].0 + w[1].0) * 0.5);
    }
}

/// Create a distribution of bucket split points where we don't have enough values, and thus we need to put
/// elements together in the buckets. This primarily occurs with real valued features.
///
/// NOTE (upstream): this algorithm tends to overcluster well-distributed data. It should probably be re-done to
/// produce a number of buckets closer to the required number.
pub fn bucket_dist_reduced(result: &mut Vec<f32>, freqs: &BucketFreqs, num_buckets: usize) -> Result<(), String> {
    // Find good split points.
    let total = freqs.total();
    let mut bucket_sizes: Vec<f64> = Vec::new();
    result.clear();
    result.reserve(num_buckets);
    let mut num: f64 = 0.0;
    let mut per_bucket: f32 = (total / num_buckets as f64) as f32;

    // Go through and look for the really big buckets. We take these out of the bucket size calculation as otherwise
    // we end up with too few buckets. This calculation is not exact (since the bucket size becomes smaller after),
    // but it should fix up the most problematic cases.
    let mut num_big_buckets = 0usize;
    let mut num_in_big_buckets = 0.0f64;
    for &(_, n) in &freqs.entries {
        if n >= per_bucket {
            num_in_big_buckets += n as f64;
            num_big_buckets += 1;
        }
    }
    if num_buckets != num_big_buckets {
        per_bucket = ((total - num_in_big_buckets) / (num_buckets as f64 - num_big_buckets as f64)) as f32;
    }

    let e = &freqs.entries;
    for i in 0..e.len() {
        let (v, n) = e[i];
        num += n as f64;
        // We split if we have enough in our bucket, or if the next one would have enough by itself.
        if n >= per_bucket && num > n as f64 {
            // Extra split, since this one had enough by itself.
            let val = ((v as f64 + e[i - 1].0 as f64) / 2.0) as f32;
            result.push(val);
            bucket_sizes.push(num - n as f64);
            num -= n as f64;
        }
        if i + 1 < e.len() && (num >= per_bucket as f64 || e[i + 1].1 >= per_bucket) {
            let mut val = (v + e[i + 1].0) * 0.5f32;
            // If the two values are one ulp apart, then we need to make sure that it gets rounded up otherwise we
            // will have two buckets with the same split value.
            if val == v {
                val = e[i + 1].0;
            }
            result.push(val);
            bucket_sizes.push(num);
            num = 0.0;
        }
    }
    for i in 1..result.len() {
        if result[i] == result[i - 1] {
            return Err("two buckets with the same split point".into());
        }
    }
    Ok(())
}

/// Create a distribution of buckets for the given set of values, targeting the given number of buckets.
pub fn bucket_dist(result: &mut Vec<f32>, freqs: &BucketFreqs, num_buckets: usize) -> Result<(), String> {
    if num_buckets >= freqs.size() {
        bucket_dist_full(result, freqs);
        Ok(())
    } else {
        bucket_dist_reduced(result, freqs, num_buckets)
    }
}

/// Purely functional version of the same.
pub fn bucket_dist_of(freqs: &BucketFreqs, num_buckets: usize) -> Result<Vec<f32>, String> {
    let mut result = Vec::new();
    bucket_dist(&mut result, freqs, num_buckets)?;
    Ok(result)
}

pub fn get_freqs(mut values: Vec<f32>) -> Result<BucketFreqs, String> {
    values.sort_by(|a, b| a.total_cmp(b));
    let mut freqs2: Vec<(f32, f32)> = Vec::with_capacity(values.len());
    let mut last = f32::NEG_INFINITY;
    let mut count = 0;
    for (i, &v) in values.iter().enumerate() {
        if v.is_nan() {
            return Err("NaN in values".into());
        }
        if i == 0 {
            last = v;
            count += 1;
        } else if bit_equal(last, v) {
            count += 1;
        } else {
            freqs2.push((last, count as f32));
            count = 1;
            last = v;
        }
    }
    freqs2.push((last, count as f32));
    Ok(BucketFreqs::new(freqs2))
}

/// Create a set of buckets for the given set of values.
pub fn create_buckets(values: &[f32], num_buckets: usize) -> Result<BucketInfo, String> {
    let freqs = get_freqs(values.to_vec())?;
    let mut result = BucketInfo::default();
    bucket_dist(&mut result.splits, &freqs, num_buckets)?;
    let mut bucket_count = vec![0i32; result.splits.len() + 1];
    for &value in values {
        let bucket = result.splits.partition_point(|&s| s <= value);
        bucket_count[bucket] += 1;
        result.buckets.push(bucket as u16);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_full_and_reduced() {
        let freqs = get_freqs(vec![3.0, 1.0, 2.0, 2.0]).unwrap();
        assert_eq!(freqs.entries, vec![(1.0, 1.0), (2.0, 2.0), (3.0, 1.0)]);
        assert_eq!(bucket_dist_of(&freqs, 10).unwrap(), vec![1.5, 2.5]);
        let values: Vec<f32> = (0..1000).map(|i| (i % 100) as f32).collect();
        let b = create_buckets(&values, 10).unwrap();
        assert!(b.splits.len() <= 12 && b.splits.len() >= 8, "{} splits", b.splits.len());
        assert!(b.splits.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(b.buckets.len(), 1000);
        assert!(get_freqs(vec![f32::NAN]).is_err());
    }
}
