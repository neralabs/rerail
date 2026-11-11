//! bucket.h and bucket.cc, in Rust.
//!
//! Mathieu Marquis Bolduc, March 11th 2016. Copyright (c) 2016 mldb.ai inc. This file is part of MLDB. Apache-2.0
//! (see `../LICENSE`).
//!
//! Structures to bucketize sets of cell values: the bit-packed per-row bucket numbers the GPU kernels read
//! (`bucketData`, `bucketEntryBits`), and the descriptions that map a value to its bucket.
//!
//! MLDB's `CellValue` (sql/cell_value.h) is not brought over; [`CellValue`] here holds the cases bucket.cc switches
//! on. `BucketDescriptions::discretize` and `getValue` were declared in bucket.h with no definition upstream, so
//! they have none here either.

use crate::forest::memory_region::{FrozenMemoryRegionT, MappedSerializer, MutableMemoryRegionT};
use crate::forest::bit_range_ops::{BitExtractor, BitWriter};
use crate::forest::buckets::{BucketFreqs, bucket_dist_full, bucket_dist_reduced};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

/// `highest_bit(x, none)`: the index of the highest set bit, or `none` when `x` is 0.
pub fn highest_bit(x: u32, none: i32) -> i32 {
    if x == 0 { none } else { 31 - x.leading_zeros() as i32 }
}

/// A value to bucket (the cases of MLDB's `CellValue` that bucketing distinguishes).
#[derive(Clone, Debug)]
pub enum CellValue {
    Empty,
    Integer(i64),
    Float(f64),
    String(String),
    Blob(Vec<u8>),
    Path(String),
    /// Seconds since the epoch.
    Timestamp(f64),
    /// (months, days, seconds).
    TimeInterval(i32, i32, f64),
}

impl CellValue {
    /// The kind, in `CellValue::CellType` order.
    fn kind(&self) -> u8 {
        match self {
            CellValue::Empty => 0,
            CellValue::Integer(_) => 1,
            CellValue::Float(_) => 2,
            CellValue::String(_) => 3,
            CellValue::Blob(_) => 5,
            CellValue::Path(_) => 6,
            CellValue::Timestamp(_) => 7,
            CellValue::TimeInterval(..) => 8,
        }
    }

    pub fn is_numeric(&self) -> bool {
        matches!(self, CellValue::Integer(_) | CellValue::Float(_))
    }

    pub fn to_double(&self) -> f64 {
        match self {
            CellValue::Integer(i) => *i as f64,
            CellValue::Float(f) => *f,
            other => panic!("cell value {other:?} is not numeric"),
        }
    }

    fn to_string_value(&self) -> String {
        match self {
            CellValue::Empty => String::new(),
            CellValue::Integer(i) => i.to_string(),
            CellValue::Float(f) => f.to_string(),
            CellValue::String(s) | CellValue::Path(s) => s.clone(),
            CellValue::Blob(b) => format!("{b:?}"),
            CellValue::Timestamp(t) => t.to_string(),
            CellValue::TimeInterval(m, d, s) => format!("{m}M{d}D{s}S"),
        }
    }
}

impl PartialEq for CellValue {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for CellValue {}

impl PartialOrd for CellValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CellValue {
    /// Numbers compare by value; otherwise by kind, then within the kind.
    fn cmp(&self, other: &Self) -> Ordering {
        use CellValue::*;
        match (self, other) {
            (a, b) if a.is_numeric() && b.is_numeric() => a.to_double().total_cmp(&b.to_double()),
            (String(a), String(b)) | (Path(a), Path(b)) => a.cmp(b),
            (Blob(a), Blob(b)) => a.cmp(b),
            (Timestamp(a), Timestamp(b)) => a.total_cmp(b),
            (TimeInterval(a, b, c), TimeInterval(d, e, f)) => (a, b).cmp(&(d, e)).then(c.total_cmp(f)),
            (a, b) => a.kind().cmp(&b.kind()),
        }
    }
}

impl std::hash::Hash for CellValue {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        match self {
            v if v.is_numeric() => v.to_double().to_bits().hash(h),
            v => {
                v.kind().hash(h);
                v.to_string_value().hash(h);
            }
        }
    }
}

/// Holds an array of bucket indexes, efficiently.
#[derive(Clone, Debug, Default)]
pub struct BucketList {
    pub storage: FrozenMemoryRegionT<u32>,
    pub entry_bits: i32,
    pub num_buckets: i32,
    pub num_entries: usize,
}

impl BucketList {
    #[inline(always)]
    pub fn get(&self, i: u32) -> u32 {
        let mut extractor = BitExtractor::new(self.storage.data());
        extractor.advance(i as isize * self.entry_bits as isize);
        extractor.extract_fast(self.entry_bits as u32)
    }

    #[inline(always)]
    pub fn at(&self, i: u32) -> u32 {
        assert!((i as usize) < self.num_entries, "bucket list: {i} of {}", self.num_entries);
        self.get(i)
    }

    pub fn row_count(&self) -> usize {
        self.num_entries
    }

    /// The list as a view of its words (`storagePtr`).
    pub fn as_ref(&self) -> BucketListRef<'_> {
        BucketListRef { storage: self.storage.data(), entry_bits: self.entry_bits as u32, num_entries: self.num_entries }
    }
}

/// A bucket list over words it does not own: upstream's `BucketList` with only its `storagePtr` set, as the
/// kernels build one over a feature's slice of all the bucket data.
#[derive(Clone, Copy, Debug)]
pub struct BucketListRef<'a> {
    pub storage: &'a [u32],
    pub entry_bits: u32,
    pub num_entries: usize,
}

impl BucketListRef<'_> {
    #[inline(always)]
    pub fn get(&self, i: u32) -> u32 {
        let mut extractor = BitExtractor::new(self.storage);
        extractor.advance(i as isize * self.entry_bits as isize);
        extractor.extract_fast(self.entry_bits)
    }
}

/// Writable version of the above.
#[derive(Debug, Default)]
pub struct WritableBucketList {
    pub storage: MutableMemoryRegionT<u32>,
    /// Bits written so far (the writer's position).
    written: usize,
    pub entry_bits: i32,
    pub num_buckets: i32,
    pub num_entries: usize,
}

impl BucketList {
    /// A list of `num_entries` buckets below `num_buckets` already packed as `WritableBucketList` writes them (the
    /// feature space's columns built on the device), copied from `words`.
    pub fn from_packed(num_entries: usize, num_buckets: u32, words: &[u32], serializer: &mut impl MappedSerializer) -> BucketList {
        let mut w = WritableBucketList::new(num_entries, num_buckets, serializer);
        let n = words.len().min(w.storage.length());
        w.storage.data_mut()[..n].copy_from_slice(&words[..n]);
        w.written = num_entries * w.entry_bits as usize;
        w.freeze(serializer)
    }
}

impl WritableBucketList {
    /// How many words (`u32`) are required for the given number of buckets each containing a number up to
    /// `num_elements`?
    pub fn words_required(num_elements: usize, num_buckets: u32) -> usize {
        let entry_bits = highest_bit(num_buckets, 0) as usize + 1;
        // +1 for extractFast
        (entry_bits * num_elements).div_ceil(32) + 1
    }

    /// Initialize from already-allocated memory. There must be at least `words_required` words already allocated.
    pub fn with_memory(num_elements: usize, num_buckets: u32, mem: MutableMemoryRegionT<u32>) -> Self {
        let mut r = WritableBucketList::default();
        r.init(num_elements, num_buckets, mem);
        r
    }

    pub fn new(num_elements: usize, num_buckets: u32, serializer: &mut impl MappedSerializer) -> Self {
        let mem = serializer.allocate_writable_t::<u32>(Self::words_required(num_elements, num_buckets), 4);
        Self::with_memory(num_elements, num_buckets, mem)
    }

    pub fn init(&mut self, num_elements: usize, num_buckets: u32, mem: MutableMemoryRegionT<u32>) {
        assert!(mem.length() >= Self::words_required(num_elements, num_buckets));
        self.num_buckets = num_buckets as i32;
        // Minimum of 1 bit to avoid divide by zero on rowCount()
        self.entry_bits = highest_bit(num_buckets, 0) + 1;
        self.storage = mem;
        self.written = 0;
        self.num_entries = num_elements;
    }

    #[inline]
    pub fn write(&mut self, value: u32) {
        let mut w = BitWriter::new(self.storage.data_mut());
        w.skip(self.written as u32);
        w.write(value, self.entry_bits as u32);
        self.written += self.entry_bits as usize;
    }

    pub fn freeze(&mut self, serializer: &mut impl MappedSerializer) -> BucketList {
        BucketList {
            entry_bits: self.entry_bits,
            num_buckets: self.num_buckets,
            num_entries: self.num_entries,
            storage: serializer.freeze_t(&mut self.storage),
        }
    }

    pub fn row_count(&self) -> usize {
        self.num_entries
    }

    #[inline(always)]
    pub fn get(&self, i: u32) -> u32 {
        let mut extractor = BitExtractor::new(self.storage.data());
        extractor.advance(i as isize * self.entry_bits as isize);
        extractor.extract_fast(self.entry_bits as u32)
    }
}

/// A bucket list written from several threads: each writes its own range from `at_offset`.
#[derive(Debug, Default)]
pub struct ParallelWritableBucketList {
    pub list: WritableBucketList,
}

impl ParallelWritableBucketList {
    pub fn new(num_elements: usize, num_buckets: u32, serializer: &mut impl MappedSerializer) -> Self {
        ParallelWritableBucketList { list: WritableBucketList::new(num_elements, num_buckets, serializer) }
    }

    pub fn with_memory(num_elements: usize, num_buckets: u32, mem: MutableMemoryRegionT<u32>) -> Self {
        ParallelWritableBucketList { list: WritableBucketList::with_memory(num_elements, num_buckets, mem) }
    }

    /// A writer at the given offset, which must start on a word boundary. This allows the bucket list to be
    /// written from multiple threads. Must only be called without any writing having taken place.
    pub fn at_offset(&self, offset: usize) -> WritableBucketList {
        let bits_to_skip = offset * self.list.entry_bits as usize;
        assert_eq!(bits_to_skip % 32, 0, "bucket list: offset {offset} is not on a word boundary");
        let start = bits_to_skip / 32 * 4;
        let raw = self.list.storage.raw().range(start, self.list.storage.raw().length());
        WritableBucketList { storage: MutableMemoryRegionT::new(raw), written: 0, entry_bits: self.list.entry_bits, num_buckets: -1, num_entries: 0 }
    }

    /// Append all of the bits from `buckets`.
    pub fn append(&mut self, buckets: &WritableBucketList) {
        assert_eq!(self.list.entry_bits, buckets.entry_bits);
        for i in 0..buckets.num_entries {
            self.list.write(buckets.get(i as u32));
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct NumericValues {
    pub active: bool,
    pub offset: u32,
    pub splits: Vec<f64>,
}

impl NumericValues {
    pub fn num_buckets(&self) -> usize {
        self.splits.len() + self.active as usize
    }

    pub fn get_bucket(&self, val: f64) -> Result<u32, String> {
        if !self.active {
            return Err("Attempt to get bucket from non-numeric value".into());
        }
        Ok(self.splits.partition_point(|&s| s < val) as u32 + self.offset)
    }
}

#[derive(Clone, Debug, Default)]
pub struct OrdinalValues {
    pub active: bool,
    pub offset: u32,
    pub splits: Vec<CellValue>,
}

impl OrdinalValues {
    pub fn num_buckets(&self) -> usize {
        self.splits.len() + self.active as usize
    }

    pub fn get_bucket(&self, val: &CellValue) -> Result<u32, String> {
        if !self.active {
            return Err("Attempt to get bucket from non-ordinal value".into());
        }
        Ok(self.splits.partition_point(|s| s < val) as u32 + self.offset)
    }
}

#[derive(Clone, Debug, Default)]
pub struct CategoricalValues {
    pub offset: u32,
    pub buckets: Vec<CellValue>,
}

impl CategoricalValues {
    pub fn num_buckets(&self) -> usize {
        self.buckets.len()
    }

    pub fn get_bucket(&self, val: &CellValue) -> Result<u32, String> {
        match self.buckets.iter().position(|b| b == val) {
            Some(i) => Ok(i as u32 + self.offset),
            None => Err(format!("categorical value '{}' not found in col", val.to_string_value())),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct StringValues {
    pub offset: u32,
    pub buckets: Vec<String>,
}

impl StringValues {
    pub fn num_buckets(&self) -> usize {
        self.buckets.len()
    }

    pub fn get_bucket(&self, val: &CellValue) -> Result<u32, String> {
        let s = match val {
            CellValue::String(s) => s.as_str(),
            other => return Err(format!("categorical value '{}' not found in col", other.to_string_value())),
        };
        let i = self.buckets.partition_point(|b| b.as_str() < s);
        if i == self.buckets.len() || self.buckets[i] != s {
            return Err(format!("categorical value '{s}' not found in col"));
        }
        Ok(i as u32 + self.offset)
    }
}

#[derive(Clone, Debug, Default)]
pub struct BucketDescriptions {
    pub has_nulls: bool,
    pub numeric: NumericValues,
    pub strings: StringValues,
    pub blobs: CategoricalValues,
    pub paths: CategoricalValues,
    pub timestamps: OrdinalValues,
    pub intervals: OrdinalValues,
}

impl BucketDescriptions {
    pub fn initialize(&mut self, values: Vec<CellValue>, num_buckets: i32) -> Result<(), String> {
        // Segment by type of value
        let mut type_values: Vec<HashMap<CellValue, usize>> = vec![HashMap::new(); 9];
        for v in values {
            *type_values[v.kind() as usize].entry(v).or_default() += 1;
        }
        // Bucketize each type. Strings can't be bucketized.
        let has_nulls = !type_values[0].is_empty();
        // Bucketize numeric values
        let mut numerics = Vec::new();
        for k in [1, 2] {
            for v in type_values[k].keys() {
                numerics.push(v.to_double());
            }
        }
        // Get string values
        let string_values: Vec<String> = type_values[3].keys().map(|v| v.to_string_value()).collect();
        self.initialize_from(has_nulls, numerics, string_values, num_buckets)
    }

    pub fn initialize_from(&mut self, has_nulls: bool, mut numeric_values: Vec<f64>, mut string_values: Vec<String>, num_buckets: i32) -> Result<(), String> {
        self.has_nulls = has_nulls;
        let mut n = has_nulls as usize;

        self.numeric.offset = n as u32;
        self.numeric.active = false;
        self.numeric.splits.clear();

        if !numeric_values.is_empty() {
            numeric_values.sort_by(|a, b| a.total_cmp(b));
            numeric_values.dedup();
            let freqs = BucketFreqs::new(numeric_values.iter().map(|&v| (v as f32, 1.0)).collect());
            let mut split_points: Vec<f32> = Vec::new();
            if num_buckets != -1 && numeric_values.len() > num_buckets as usize {
                bucket_dist_reduced(&mut split_points, &freqs, num_buckets as usize)?;
            } else {
                bucket_dist_full(&mut split_points, &freqs);
            }
            self.numeric.active = true;
            self.numeric.splits = split_points.into_iter().map(|v| v as f64).collect();
            n += self.numeric.splits.len() + 1;
        }

        // Get string values
        string_values.sort();
        string_values.dedup();
        self.strings.offset = n as u32;
        n += string_values.len();
        self.strings.buckets = string_values;

        // TODO (upstream): complete this (MLDB-1457)
        self.blobs.offset = n as u32;
        self.timestamps.offset = n as u32;
        self.intervals.offset = n as u32;
        self.blobs.buckets.clear();
        self.timestamps.splits.clear();
        self.intervals.splits.clear();
        self.timestamps.active = false;
        self.intervals.active = false;

        assert_eq!(n, self.num_buckets());
        Ok(())
    }

    /// Look up the bucket number for the given value.
    pub fn get_bucket(&self, val: &CellValue) -> Result<u32, String> {
        match val {
            CellValue::Empty => {
                if !self.has_nulls {
                    return Err("Unknown type for bucket".into());
                }
                Ok(0)
            }
            CellValue::Integer(_) | CellValue::Float(_) => self.numeric.get_bucket(val.to_double()),
            CellValue::String(_) => self.strings.get_bucket(val),
            CellValue::Blob(_) => self.blobs.get_bucket(val),
            CellValue::Path(_) => self.paths.get_bucket(val),
            CellValue::Timestamp(_) => self.timestamps.get_bucket(val),
            CellValue::TimeInterval(..) => self.intervals.get_bucket(val),
        }
    }

    pub fn get_split(&self, bucket: u32) -> Result<CellValue, String> {
        assert!((bucket as usize) < self.num_buckets(), "bucket {bucket} of {}", self.num_buckets());
        if bucket < self.numeric.offset {
            Ok(CellValue::Empty)
        } else if bucket < self.strings.offset {
            // As upstream: the first numeric bucket has no split below it (`at(-1)` throws).
            let i = (bucket - self.numeric.offset) as usize;
            i.checked_sub(1).and_then(|i| self.numeric.splits.get(i)).map(|&v| CellValue::Float(v)).ok_or_else(|| "vector::at: out of range".to_string())
        } else if bucket < self.blobs.offset {
            Ok(CellValue::String(self.strings.buckets[(bucket - self.strings.offset) as usize].clone()))
        } else {
            Err("Invalid bucket".into())
        }
    }

    pub fn is_only_numeric(&self) -> bool {
        // todo (upstream): complete this
        self.strings.buckets.is_empty()
    }

    pub fn num_buckets(&self) -> usize {
        self.intervals.offset as usize + self.intervals.num_buckets()
    }

    pub fn merge(inputs: &[(BucketList, BucketDescriptions)], serializer: &mut impl MappedSerializer, num_buckets: i32) -> Result<(BucketList, BucketDescriptions), String> {
        let mut desc = BucketDescriptions::default();
        let mut all_numeric = Vec::new();
        let mut all_strings = HashSet::new();
        let mut total_rows = 0;
        let mut has_nulls = false;
        for (l, d) in inputs {
            has_nulls = has_nulls || d.has_nulls;
            all_numeric.extend_from_slice(&d.numeric.splits);
            all_strings.extend(d.strings.buckets.iter().cloned());
            total_rows += l.row_count();
        }
        desc.initialize_from(has_nulls, all_numeric, all_strings.into_iter().collect(), num_buckets)?;

        // Finally, perform the bucketed lookup
        let mut buckets = WritableBucketList::new(total_rows, desc.num_buckets() as u32, serializer);
        for (l, d) in inputs {
            let mut old_to_new = vec![0u32; d.num_buckets()];
            for (j, o) in old_to_new.iter_mut().enumerate() {
                *o = desc.get_bucket(&d.get_split(j as u32)?)?;
            }
            for j in 0..l.num_entries {
                buckets.write(old_to_new[l.get(j as u32) as usize]);
            }
        }
        Ok((buckets.freeze(serializer), desc))
    }
}

#[cfg(test)]
mod tests {
    use crate::forest::memory_region::MemorySerializer;
    use super::*;

    #[test]
    fn bucket_list_write_freeze_read() {
        let mut s = MemorySerializer;
        let n = 1000;
        let mut w = ParallelWritableBucketList::new(n, 300, &mut s);
        assert_eq!(w.list.entry_bits, 9);
        // Two writers: the first half, then the second from a word boundary (9 * 32 rows).
        let split = 9 * 32 / 9 * 32 / 32;
        let mut second = w.at_offset(split);
        for i in 0..split {
            w.list.write((i * 7 % 300) as u32);
        }
        for i in split..n {
            second.write((i * 7 % 300) as u32);
        }
        let l = w.list.freeze(&mut s);
        for i in 0..n {
            assert_eq!(l.at(i as u32), (i * 7 % 300) as u32, "row {i}");
        }
    }

    #[test]
    fn bucket_descriptions_numeric_and_strings() {
        let mut d = BucketDescriptions::default();
        let values = vec![CellValue::Empty, CellValue::Integer(1), CellValue::Float(2.0), CellValue::Integer(3), CellValue::String("b".into()), CellValue::String("a".into())];
        d.initialize(values, -1).unwrap();
        assert_eq!(d.num_buckets(), 1 + 3 + 2);
        assert_eq!(d.get_bucket(&CellValue::Empty).unwrap(), 0);
        assert_eq!(d.get_bucket(&CellValue::Integer(1)).unwrap(), 1);
        // lower_bound: a value on a split goes below it.
        assert_eq!(d.get_bucket(&CellValue::Float(2.5)).unwrap(), 2);
        assert_eq!(d.get_bucket(&CellValue::Integer(3)).unwrap(), 3);
        assert_eq!(d.get_bucket(&CellValue::String("b".into())).unwrap(), 5);
        assert!(d.get_bucket(&CellValue::String("c".into())).is_err());
        assert!(matches!(d.get_split(2).unwrap(), CellValue::Float(v) if v == 1.5));
    }
}
