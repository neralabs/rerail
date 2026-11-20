//! A column's statistics for the planner (decision 0112), in a fixed size whatever the rows: its distinct values
//! (a HyperLogLog of 256 registers, about 6.5% off), the sum of its values' row counts squared (a count sketch of 256
//! counters, about 9% off), and, while it holds at most `SMALL` values or a bounded integer page, each value's exact count (then the distinct
//! values and the sum are exact too). Compact labels use a 16 KiB page; sparse label sets retain at most 1,024 counters.
//! Identity columns keep the fixed-size fallback. Exact counts are updated as rows arrive, never by a planning-time scan.
//! Past `SMALL` values it keeps its column's keys in a blocked Bloom filter too (`bloom::Kept`, decision 0477), set
//! from the same hash as each row is counted, and made again larger from the column where its keys outgrow it.
//! The screens test probes against it, and the planner counts the keys two columns share by it.

use crate::code_rule_dsl::logic::bloom::{self, Keys, Kept, Reading, Sample, Words};
use std::sync::Arc;

/// The values a column holds at most for its exact counts.
const SMALL: usize = 64;
const REGISTERS: usize = 256;
const DENSE_WIDTH: usize = 4096;
#[derive(Clone)]
struct Dense { base:i64, counts:Box<[u32;DENSE_WIDTH]>, ndv:usize, sq:u64 }
impl Dense {
    fn add(&mut self, v:i64)->bool {
        let Some(i)=v.checked_sub(self.base).and_then(|n|usize::try_from(n).ok()).filter(|&i|i<DENSE_WIDTH) else {return false};
        let n=&mut self.counts[i];
        if *n==0 {self.ndv+=1;}
        self.sq+=2*u64::from(*n)+1;
        *n+=1;
        true
    }
}

const LABEL_LIMIT:i64=1_i64<<32;
const SPARSE_LIMIT:usize=1024;
#[derive(Clone)]
struct Sparse { counts:std::collections::HashMap<i64,u32>, sq:u64 }
impl Sparse {
    fn from_counts(values:impl IntoIterator<Item=(i64,u32)>)->Self {
        let counts:std::collections::HashMap<_,_>=values.into_iter().collect();
        let sq=counts.values().map(|&n|u64::from(n).pow(2)).sum();
        Self {counts,sq}
    }
    fn add(&mut self,v:i64)->bool {
        if !(0..LABEL_LIMIT).contains(&v) || (self.counts.len()>=SPARSE_LIMIT && !self.counts.contains_key(&v)) {return false;}
        let n=self.counts.entry(v).or_default();self.sq+=2*u64::from(*n)+1;*n+=1;true
    }
}

pub(super) struct Sketch {
    registers: Box<[u8; REGISTERS]>,
    counters: Box<[i32; REGISTERS]>,
    rows: u64,
    /// Each value's rows while there are at most `SMALL` values.
    small: Option<Vec<(i64, u32)>>,
    dense: Option<Dense>,
    sparse: Option<Sparse>,
    /// Its keys' filter, once it holds more than `SMALL` values (while `small` holds them, they are its keys). This
    /// sketch is its one writer; the planner's statistics and the screens read it where it is.
    keys: Option<Arc<Kept>>,
    /// Whether it writes `keys`: a clone reads the filter it was cloned with until it first adds a key, then writes a
    /// copy of its own (a filter has one writer; the one it shares keeps all of the clone's keys until then).
    own_keys: bool,
    /// A sample of its distinct keys (`bloom::Sample`), offered each key its filter takes as new.
    sample: Sample,
}

impl Default for Sketch {
    fn default() -> Sketch {
        Sketch { registers: Box::new([0; REGISTERS]), counters: Box::new([0; REGISTERS]), rows: 0, small: Some(Vec::new()), dense:None, sparse:None, keys: None, own_keys: true, sample: Sample::default() }
    }
}

/// A clone shares the filter until it adds to it (`own_keys`): a kept relation reopened to be maintained copies its
/// columns' filters only where its rows change them.
impl Clone for Sketch {
    fn clone(&self) -> Sketch {
        Sketch { registers: self.registers.clone(), counters: self.counters.clone(), rows: self.rows, small: self.small.clone(), dense: self.dense.clone(),
                 sparse: self.sparse.clone(), keys: self.keys.clone(), own_keys: false, sample: self.sample.clone() }
    }
}

/// The keys' hash: the Bloom filters' too (`logic::bloom`), so a filter and a sketch see the same keys alike.
#[inline]
pub(super) fn mix(v: i64) -> u64 {
    crate::code_rule_dsl::logic::bloom::hash(v)
}

impl Sketch {
    pub(super) fn add(&mut self, v: i64) {
        let h = mix(v);
        let r = (h >> 56) as usize;
        let rho = ((h << 8) | 0x80).leading_zeros() as u8 + 1;
        if rho > self.registers[r] {
            self.registers[r] = rho;
        }
        let (c, sign) = crate::code_rule_dsl::logic::join_size::bucket(h);
        self.counters[c] += sign;
        self.rows += 1;
        if let Some(keys) = &mut self.keys {
            if !self.own_keys {
                *keys = Arc::new(Kept::clone(keys));
                self.own_keys = true;
            }
            if keys.add_hashed(h) {
                self.sample.offer(h);
            }
        }
        if let Some(sparse)=&mut self.sparse {
            if !sparse.add(v) {self.sparse=None;}
        } else if let Some(dense) = &mut self.dense {
            if !dense.add(v) {
                if (0..LABEL_LIMIT).contains(&v) && dense.ndv<SPARSE_LIMIT {
                    let mut sparse=Sparse::from_counts(dense.counts.iter().enumerate().filter(|(_,n)| **n>0).map(|(i,&n)|(dense.base+i as i64,n)));
                    sparse.add(v);self.sparse=Some(sparse);
                }
                self.dense=None;
            }
        } else if let Some(small) = &mut self.small {
            if let Some(e) = small.iter_mut().find(|(x, _)| *x == v) {
                e.1 += 1;
            } else if small.len() < SMALL {
                small.push((v, 1));
            } else {
                // Retain bounded label domains beyond SMALL without linear searches.
                // Large identity columns retain the fixed-size sketch fallback.
                let base=v & !((DENSE_WIDTH as i64)-1);
                if (0..LABEL_LIMIT).contains(&v) && small.iter().all(|(x,_)| *x>=base && *x-base<DENSE_WIDTH as i64) {
                    let mut dense=Dense {base,counts:Box::new([0;DENSE_WIDTH]),ndv:small.len(),sq:0};
                    for &(x,n) in small.iter() {dense.counts[(x-base) as usize]=n;dense.sq+=u64::from(n).pow(2);}
                    dense.add(v);
                    self.dense=Some(dense);
                } else if (0..LABEL_LIMIT).contains(&v) && small.iter().all(|(x,_)|(0..LABEL_LIMIT).contains(x)) {
                    let mut sparse=Sparse::from_counts(small.iter().copied());sparse.add(v);self.sparse=Some(sparse);
                }
                // Its keys from here on in a filter, from the values it held and this one.
                let keys = Kept::new(bloom::kept_words_for(4 * SMALL));
                for h in small.iter().map(|&(x, _)| mix(x)).chain([h]) {
                    keys.add_hashed(h);
                    self.sample.offer(h);
                }
                self.keys = Some(Arc::new(keys));
                self.own_keys = true;
                self.small = None;
            }
        }
    }

    /// Distinct values.
    pub(super) fn ndv(&self) -> usize {
        if let Some(s)=&self.sparse {return s.counts.len();}
        if let Some(d)=&self.dense {return d.ndv;}
        if let Some(small) = &self.small {
            return small.len();
        }
        Self::estimate(self.registers.iter().copied(), self.rows)
    }

    /// Estimated fraction of the probe column's distinct keys present in the build column.
    /// Inclusion-exclusion can lose a small intersection in the sketch error. This only prices a plan.
    pub(super) fn overlap(&self, other: &Self) -> f64 {
        if self.rows == 0 || other.rows == 0 {
            return 0.0;
        }
        // The three estimates alike, none clamped to its rows: a side's estimate clamped where the union's is not
        // made a sketch overlap itself less than whole when its estimate ran over its rows.
        let left = Self::estimate(self.registers.iter().copied(), u64::MAX);
        let right = Self::estimate(other.registers.iter().copied(), u64::MAX);
        let union = Self::estimate(self.registers.iter().zip(other.registers.iter()).map(|(&a, &b)| a.max(b)), u64::MAX);
        (left.saturating_add(right).saturating_sub(union).min(left.min(right)) as f64) / left.max(1) as f64
    }

    fn estimate(registers: impl Iterator<Item = u8>, rows: u64) -> usize {
        if rows == 0 { return 0; }
        let m = REGISTERS as f64;
        let mut sum = 0.0;
        let mut zeros = 0;
        for rank in registers {
            sum += 2f64.powi(-i32::from(rank));
            zeros += usize::from(rank == 0);
        }
        let raw = 0.7213 / (1.0 + 1.079 / m) * m * m / sum;
        let est = if raw <= 2.5 * m && zeros > 0 { m * (m / zeros as f64).ln() } else { raw };
        (est.round() as usize).clamp(1, usize::try_from(rows).unwrap_or(usize::MAX))
    }

    /// The sum over values of each value's rows squared.
    pub(super) fn sq(&self) -> u64 {
        if let Some(s)=&self.sparse {return s.sq;}
        if let Some(d)=&self.dense {return d.sq;}
        if let Some(small) = &self.small {
            return small.iter().map(|&(_, n)| u64::from(n) * u64::from(n)).sum();
        }
        let est: f64 = self.counters.iter().map(|&c| (c as f64) * (c as f64)).sum();
        // At least a row a value, at most every row one value.
        (est.round() as u64).clamp(self.rows, self.rows * self.rows)
    }

    pub(super) fn exact_counts(&self) -> Option<std::borrow::Cow<'_,[(i64,u32)]>> {
        if let Some(s)=&self.small {return Some(std::borrow::Cow::Borrowed(s));}
        if let Some(s)=&self.sparse {return Some(std::borrow::Cow::Owned(s.counts.iter().map(|(&v,&n)|(v,n)).collect()));}
        self.dense.as_ref().map(|d|std::borrow::Cow::Owned(d.counts.iter().enumerate().filter(|(_,n)| **n>0).map(|(i,&n)|(d.base+i as i64,n)).collect()))
    }

    /// Its filter made again at twice its keys where they outgrew it (`Kept::outgrown`), from `all`: every value it
    /// was given (a dead row's too: a filter may hold more keys than its column, never fewer). The filters double,
    /// so a column's values are read again a few times at most, and only by a column of many distinct values.
    pub(super) fn grow<I: Iterator<Item = i64>>(&mut self, all: impl FnOnce() -> I) {
        let Some(keys) = &self.keys else { return };
        if !keys.outgrown() {
            return;
        }
        let made = Kept::new(bloom::kept_words_for(2 * self.ndv().max(Words::keys(&**keys) as usize)));
        for v in all() {
            made.add_hashed(mix(v));
        }
        self.keys = Some(Arc::new(made));
        self.own_keys = true;
    }

    /// Its keys, as the statistics read them: its few values, else its filter (held where it is) as read now.
    pub(super) fn keys(&self) -> Keys {
        match (&self.small, &self.keys) {
            (Some(small), _) => Keys::Few(small.iter().map(|&(v, _)| v).collect()),
            (None, Some(k)) => Keys::Filter(Arc::new(Reading::new(Arc::clone(k), self.sample.hashes()))),
            (None, None) => unreachable!("a sketch past its few values keeps their filter"),
        }
    }

    /// Its counts for the planner's join sizes (`logic::join_size`): its counters, and its few values' counts.
    pub(super) fn counts(&self) -> crate::code_rule_dsl::logic::join_size::JoinCounts {
        crate::code_rule_dsl::logic::join_size::JoinCounts::new(&self.counters, self.small.as_deref(), self.rows, self.sq())
    }

    /// Its keys' filter, where it keeps one (past its few values); else its few values are its keys.
    pub(crate) fn filter(&self) -> Option<&Arc<Kept>> {
        self.keys.as_ref()
    }

    pub(super) fn bytes(&self) -> usize {
        self.keys.as_ref().map_or(0, |k| k.bytes()) + self.sample.hashes().len() * 8 + self.sparse.as_ref().map_or(0,|s|s.counts.capacity()*24) + self.dense.as_ref().map_or(0,|_|DENSE_WIDTH*4) + REGISTERS * 5 + self.small.as_ref().map_or(0, |s| s.capacity() * 16)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn sketches_estimate_distinct_values_and_squares() {
        let mut few = Sketch::default();
        for i in 0..1000 {
            few.add(i % 10);
        }
        assert_eq!((few.ndv(), few.sq()), (10, 10 * 100 * 100));
        let mut many = Sketch::default();
        for i in 0..200_000i64 {
            many.add(i % 50_000);
        }
        let ndv = many.ndv() as f64;
        assert!((ndv / 50_000.0 - 1.0).abs() < 0.2, "{ndv}");
        // 50,000 values of 4 rows: 800,000.
        let sq = many.sq() as f64;
        assert!((sq / 800_000.0 - 1.0).abs() < 0.35, "{sq}");
        let mut skewed = Sketch::default();
        for i in 0..100_000i64 {
            skewed.add(if i % 2 == 0 { 7 } else { i });
        }
        // One value of 50,000 rows dominates: about 2.5e9.
        let sq = skewed.sq() as f64;
        assert!((sq / 2.5e9 - 1.0).abs() < 0.2, "{sq}");
    }

    /// Past its few values a sketch keeps its keys' filter: every key added is held as it grows from 256 to
    /// 100,000 keys, and a clone writes a copy of its own once it adds.
    #[test]
    fn a_sketch_keeps_a_filter_of_its_keys_as_they_grow() {
        let mut sk = Sketch::default();
        assert!(matches!(sk.keys(), Keys::Few(_)));
        let values = |n: i64| (0..n).map(|i| i * 7_919);
        for v in values(256) {
            sk.add(v);
        }
        let first = Arc::clone(sk.filter().expect("past its few values"));
        for v in values(100_000).skip(256) {
            sk.add(v);
            if v % 1_024 == 0 {
                sk.grow(|| values(100_000).take_while(|&x| x <= v));
            }
        }
        sk.grow(|| values(100_000));
        let Keys::Filter(reading) = sk.keys() else { panic!("a filter") };
        let grown = Arc::clone(&reading.filter);
        assert!(grown.len() > first.len() && values(100_000).all(|v| grown.may_hold(v)), "{} words", grown.len());
        assert_eq!(reading.sample.len(), bloom::SAMPLE);
        let distinct = sk.keys().distinct();
        assert!((distinct / 100_000.0 - 1.0).abs() < 0.1, "{distinct}");
        let absent = (1..).map(|i| -i).find(|&v| !grown.may_hold(v)).unwrap();
        let mut copy = sk.clone();
        copy.add(absent);
        assert!(copy.keys().may_hold(absent) && !sk.keys().may_hold(absent), "a clone's key is in its own filter only");
    }

    // Bounded histograms (were the integration test `bounded_histograms`, which included this file alone: it reads
    // the engine's filters now).
    #[test]
    fn compact_integer_domains_keep_exact_counts_past_64_distinct_values() {
        let mut s=Sketch::default();
        let mut sq=0;
        for value in 0..200 { for _ in 0..value+1 { s.add(value); } sq+=(value as u64+1).pow(2); }
        assert_eq!(s.ndv(),200,"bounded labels must retain exact cardinality");
        assert_eq!(s.sq(),sq,"bounded labels must retain exact posting counts");
        assert_eq!(s.exact_counts().unwrap().iter().copied().collect::<std::collections::HashMap<_,_>>()[&137],138);
        assert!(s.bytes()<20_000,"histogram memory must be bounded");
        s.add(1_i64<<40);
        assert!(s.exact_counts().is_none(),"an escaped domain must fall back, not claim a complete histogram");
    }
    #[test]
    fn sparse_label_domains_also_keep_exact_counts_with_bounded_memory() {
        let mut s=Sketch::default();
        for i in 0..200 {for _ in 0..3 {s.add(i*293);}}
        assert_eq!(s.exact_counts().unwrap().len(),200);
        assert_eq!(s.sq(),1800);
        assert!(s.bytes()<65536);
        for i in 200..2000 {s.add((i*293)%60000);}
        assert!(s.exact_counts().is_none(),"large domains must return to bounded sketches");
    }
    #[test]
    fn label_ids_do_not_lose_histograms_when_the_atom_arena_grows() {
        let mut s=Sketch::default();
        for i in 0..200 {s.add(600_000+i*293);}
        assert_eq!(s.exact_counts().unwrap().len(),200);
    }
}
