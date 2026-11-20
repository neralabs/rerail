//! A column's statistics for the planner (decision 0112), in a fixed size whatever the rows: its distinct values
//! (a HyperLogLog of 256 registers, about 6.5% off), the sum of its values' row counts squared (a count sketch of 256
//! counters, about 9% off), and, while it holds at most `SMALL` values or a bounded integer page, each value's exact count (then the distinct
//! values and the sum are exact too). Compact labels use a 16 KiB page; sparse label sets retain at most 1,024 counters.
//! Identity columns keep the fixed-size fallback. Exact counts are updated as rows arrive, never by a planning-time scan.

/// The values a column holds at most for its exact counts.
const SMALL: usize = 64;
const REGISTERS: usize = 256;
const DENSE_WIDTH: usize = 4096;
use super::rows::BlockVec;

/// `n` zeros in a block (decision 0478 step 6c: a sketch's arrays are block arrays, not the heap's).
fn zeros<T: rerail_blocks::Pod + Copy + Default>(n: usize) -> BlockVec<T> {
    let mut v = BlockVec::new();
    v.resize(n, T::default());
    v
}

#[derive(Clone)]
struct Dense { base:i64, counts:BlockVec<u32>, ndv:usize, sq:u64 }
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
/// Slots of a sparse sketch's counts at most: twice its values, a power of two. It starts at `SPARSE_FIRST` and
/// doubles while its values pass half its slots.
const SPARSE_SLOTS:usize=2*SPARSE_LIMIT;
const SPARSE_FIRST:usize=128;
/// An empty slot's value (a sparse sketch's values are in `0..LABEL_LIMIT`).
const NO_VALUE:i64=-1;
/// A label set's exact counts, open addressing in a block: each slot `[value, rows]`, `[NO_VALUE, 0]` for none.
#[derive(Clone)]
struct Sparse { slots:BlockVec<[i64;2]>, len:usize, sq:u64 }
impl Sparse {
    fn slot(&self, v:i64)->usize {
        let m=self.slots.len()-1;
        let mut at=(mix(v) as usize)&m;
        while self.slots[at][0]!=NO_VALUE && self.slots[at][0]!=v {at=(at+1)&m;}
        at
    }
    /// Room for one more value: its slots doubled, each entry placed again, while its values pass half of them.
    fn room(&mut self) {
        if 2*(self.len+1)<=self.slots.len() || self.slots.len()>=SPARSE_SLOTS {return;}
        let mut grown=Self {slots:BlockVec::new(),len:0,sq:self.sq};
        grown.slots.resize(self.slots.len()*2,[NO_VALUE,0]);
        for e in self.slots.iter().filter(|e| e[0]!=NO_VALUE) {
            let at=grown.slot(e[0]);
            grown.slots[at]=*e;
            grown.len+=1;
        }
        *self=grown;
    }
    fn from_counts(values:impl IntoIterator<Item=(i64,u32)>)->Self {
        let mut s=Self {slots:BlockVec::new(),len:0,sq:0};
        s.slots.resize(SPARSE_FIRST,[NO_VALUE,0]);
        for (v,n) in values {
            s.room();
            let at=s.slot(v);
            if s.slots[at][0]==NO_VALUE {s.len+=1;}
            s.slots[at]=[v,i64::from(n)];
            s.sq+=u64::from(n).pow(2);
        }
        s
    }
    fn add(&mut self,v:i64)->bool {
        if !(0..LABEL_LIMIT).contains(&v) {return false;}
        if self.len>=SPARSE_LIMIT && self.slots[self.slot(v)][0]==NO_VALUE {return false;}
        self.room();
        let at=self.slot(v);
        if self.slots[at][0]==NO_VALUE {
            self.slots[at]=[v,0];
            self.len+=1;
        }
        let n=self.slots[at][1] as u64;
        self.sq+=2*n+1;
        self.slots[at][1]+=1;
        true
    }
    fn counts(&self)->impl Iterator<Item=(i64,u32)>+'_ {
        self.slots.iter().filter(|e| e[0]!=NO_VALUE).map(|e|(e[0],e[1] as u32))
    }
}

#[derive(Clone)]
pub(super) struct Sketch {
    registers: BlockVec<u8>,
    counters: BlockVec<i32>,
    rows: u64,
    /// Each value's rows (`[value, rows]`) while there are at most `SMALL` values.
    small: Option<BlockVec<[i64; 2]>>,
    dense: Option<Dense>,
    sparse: Option<Sparse>,
}

impl Default for Sketch {
    fn default() -> Sketch {
        Sketch { registers: zeros(REGISTERS), counters: zeros(REGISTERS), rows: 0, small: Some(BlockVec::new()), dense:None, sparse:None }
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
        let c = ((h >> 8) & 0xff) as usize;
        self.counters[c] += if h & 1 == 0 { 1 } else { -1 };
        self.rows += 1;
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
            if let Some(e) = small.iter_mut().find(|e| e[0] == v) {
                e[1] += 1;
            } else if small.len() < SMALL {
                small.push([v, 1]);
            } else {
                // Retain bounded label domains beyond SMALL without linear searches.
                // Large identity columns retain the fixed-size sketch fallback.
                let base=v & !((DENSE_WIDTH as i64)-1);
                if (0..LABEL_LIMIT).contains(&v) && small.iter().all(|e| e[0]>=base && e[0]-base<DENSE_WIDTH as i64) {
                    let mut dense=Dense {base,counts:zeros(DENSE_WIDTH),ndv:small.len(),sq:0};
                    for &[x,n] in small.iter() {dense.counts[(x-base) as usize]=n as u32;dense.sq+=(n as u64).pow(2);}
                    dense.add(v);
                    self.dense=Some(dense);
                } else if (0..LABEL_LIMIT).contains(&v) && small.iter().all(|e|(0..LABEL_LIMIT).contains(&e[0])) {
                    let mut sparse=Sparse::from_counts(small.iter().map(|e|(e[0],e[1] as u32)));sparse.add(v);self.sparse=Some(sparse);
                }
                self.small = None;
            }
        }
    }

    /// Distinct values.
    pub(super) fn ndv(&self) -> usize {
        if let Some(s)=&self.sparse {return s.len;}
        if let Some(d)=&self.dense {return d.ndv;}
        if let Some(small) = &self.small {
            return small.len();
        }
        Self::estimate(self.registers.iter().copied(), self.rows)
    }

    /// Estimated fraction of the probe column's distinct keys present in the build column.
    /// Inclusion-exclusion can lose a small intersection in the sketch error. This only prices a plan.
    pub(super) fn overlap(&self, other: &Self) -> f64 {
        let left = self.ndv();
        let right = other.ndv();
        let union = Self::estimate(self.registers.iter().zip(other.registers.iter()).map(|(&a, &b)| a.max(b)),
            self.rows.saturating_add(other.rows));
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
            return small.iter().map(|e| (e[1] as u64) * (e[1] as u64)).sum();
        }
        let est: f64 = self.counters.iter().map(|&c| (c as f64) * (c as f64)).sum();
        // At least a row a value, at most every row one value.
        (est.round() as u64).clamp(self.rows, self.rows * self.rows)
    }

    /// Each value's rows, while they are exact: read from the sketch's arrays.
    pub(super) fn exact_counts(&self) -> Option<Vec<(i64,u32)>> {
        if let Some(s)=&self.small {return Some(s.iter().map(|e|(e[0],e[1] as u32)).collect());}
        if let Some(s)=&self.sparse {return Some(s.counts().collect());}
        self.dense.as_ref().map(|d|d.counts.iter().enumerate().filter(|(_,n)| **n>0).map(|(i,&n)|(d.base+i as i64,n)).collect())
    }

    pub(super) fn bytes(&self) -> usize {
        self.sparse.as_ref().map_or(0,|s|s.slots.len()*16) + self.dense.as_ref().map_or(0,|_|DENSE_WIDTH*4) + REGISTERS * 5 + self.small.as_ref().map_or(0, |s| s.capacity() * 16)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Decision 0478 step 6c: a sketch's arrays (registers, counters, exact counts in each form) are block arrays.
    #[test]
    fn a_sketchs_arrays_are_in_blocks() {
        let mut s = Sketch::default();
        assert!(s.registers.facts().is_some() && s.counters.facts().is_some());
        for i in 0..100 { s.add(i); }
        assert!(s.dense.as_ref().is_some_and(|d| d.counts.facts().is_some()), "a page of counts");
        let mut t = Sketch::default();
        for i in 0..100 { t.add(i * 9_999); }
        assert!(t.sparse.as_ref().is_some_and(|x| x.slots.facts().is_some() && x.len == 100), "a label set's slots");
        assert_eq!(t.ndv(), 100);
    }

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
}
