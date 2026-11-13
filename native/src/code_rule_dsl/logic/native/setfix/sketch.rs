//! A column's statistics for the planner (decision 0112), in a fixed size whatever the rows: its distinct values
//! (a HyperLogLog of 256 registers, about 6.5% off), the sum of its values' row counts squared (a count sketch of 256
//! counters, about 9% off), and, while it holds at most `SMALL` values or a bounded integer page, each value's exact count (then the distinct
//! values and the sum are exact too). Compact labels use a 16 KiB page; sparse label sets retain at most 1,024 counters.
//! Identity columns keep the fixed-size fallback. Exact counts are updated as rows arrive, never by a planning-time scan.

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

#[derive(Clone)]
pub(super) struct Sketch {
    registers: Box<[u8; REGISTERS]>,
    counters: Box<[i32; REGISTERS]>,
    rows: u64,
    /// Each value's rows while there are at most `SMALL` values.
    small: Option<Vec<(i64, u32)>>,
    dense: Option<Dense>,
    sparse: Option<Sparse>,
}

impl Default for Sketch {
    fn default() -> Sketch {
        Sketch { registers: Box::new([0; REGISTERS]), counters: Box::new([0; REGISTERS]), rows: 0, small: Some(Vec::new()), dense:None, sparse:None }
    }
}

#[inline]
fn mix(v: i64) -> u64 {
    let mut x = v as u64 ^ 0x9e37_79b9_7f4a_7c15;
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
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
        let m = REGISTERS as f64;
        let sum: f64 = self.registers.iter().map(|&r| 2f64.powi(-(r as i32))).sum();
        let raw = 0.7213 / (1.0 + 1.079 / m) * m * m / sum;
        let zeros = self.registers.iter().filter(|&&r| r == 0).count();
        let est = if raw <= 2.5 * m && zeros > 0 { m * (m / zeros as f64).ln() } else { raw };
        (est.round() as usize).clamp(1, self.rows.max(1) as usize)
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

    /// Each value's rows, while the column holds at most `SMALL` values.
    pub(super) fn small(&self) -> Option<&[(i64, u32)]> {
        self.small.as_deref()
    }

    pub(super) fn exact_counts(&self) -> Option<std::borrow::Cow<'_,[(i64,u32)]>> {
        if let Some(s)=&self.small {return Some(std::borrow::Cow::Borrowed(s));}
        if let Some(s)=&self.sparse {return Some(std::borrow::Cow::Owned(s.counts.iter().map(|(&v,&n)|(v,n)).collect()));}
        self.dense.as_ref().map(|d|std::borrow::Cow::Owned(d.counts.iter().enumerate().filter(|(_,n)| **n>0).map(|(i,&n)|(d.base+i as i64,n)).collect()))
    }

    pub(super) fn bytes(&self) -> usize {
        self.sparse.as_ref().map_or(0,|s|s.counts.capacity()*24) + self.dense.as_ref().map_or(0,|_|DENSE_WIDTH*4) + REGISTERS * 5 + self.small.as_ref().map_or(0, |s| s.capacity() * 16)
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
}
