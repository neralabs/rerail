//! One bit per byte of the input, 64 bytes to a word. Bit `i` is byte `i`; bit `i % 64` of word
//! `i / 64`. Bits at or past `n` are always clear.

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Mask {
    pub n: usize,
    pub w: Vec<u64>,
}

const ODD: u64 = 0xAAAA_AAAA_AAAA_AAAA;

impl Mask {
    pub fn zeros(n: usize) -> Mask {
        Mask { n, w: vec![0; n.div_ceil(64)] }
    }
    pub fn ones(n: usize) -> Mask {
        let mut m = Mask { n, w: vec![!0; n.div_ceil(64)] };
        m.trim();
        m
    }
    pub fn from_fn(n: usize, mut f: impl FnMut(usize) -> bool) -> Mask {
        let mut m = Mask::zeros(n);
        for i in 0..n {
            if f(i) {
                m.set(i)
            }
        }
        m
    }
    fn trim(&mut self) {
        let r = self.n % 64;
        if r != 0 {
            if let Some(l) = self.w.last_mut() {
                *l &= (1u64 << r) - 1
            }
        }
    }
    #[inline]
    pub fn get(&self, i: usize) -> bool {
        i < self.n && self.w[i / 64] >> (i % 64) & 1 == 1
    }
    #[inline]
    pub fn set(&mut self, i: usize) {
        self.w[i / 64] |= 1 << (i % 64)
    }
    #[inline]
    pub fn clear(&mut self, i: usize) {
        self.w[i / 64] &= !(1 << (i % 64))
    }
    pub fn any(&self) -> bool {
        self.w.iter().any(|&x| x != 0)
    }
    pub fn count(&self) -> usize {
        self.w.iter().map(|x| x.count_ones() as usize).sum()
    }
    pub fn first(&self) -> Option<usize> {
        self.w.iter().position(|&x| x != 0).map(|j| j * 64 + self.w[j].trailing_zeros() as usize)
    }
    pub fn ones_iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.w.iter().enumerate().flat_map(|(j, &x)| {
            let mut x = x;
            std::iter::from_fn(move || {
                if x == 0 {
                    return None;
                }
                let b = x.trailing_zeros() as usize;
                x &= x - 1;
                Some(j * 64 + b)
            })
        })
    }

    pub fn and(&self, o: &Mask) -> Mask {
        Mask { n: self.n, w: self.w.iter().zip(&o.w).map(|(a, b)| a & b).collect() }
    }
    pub fn or(&self, o: &Mask) -> Mask {
        Mask { n: self.n, w: self.w.iter().zip(&o.w).map(|(a, b)| a | b).collect() }
    }
    pub fn xor(&self, o: &Mask) -> Mask {
        Mask { n: self.n, w: self.w.iter().zip(&o.w).map(|(a, b)| a ^ b).collect() }
    }
    pub fn not(&self) -> Mask {
        let mut m = Mask { n: self.n, w: self.w.iter().map(|a| !a).collect() };
        m.trim();
        m
    }
    pub fn andnot(&self, o: &Mask) -> Mask {
        Mask { n: self.n, w: self.w.iter().zip(&o.w).map(|(a, b)| a & !b).collect() }
    }

    /// Bit `i` is bit `i - k` of `self`: the word shifts up, and the top `k` bits of the word
    /// before come in at the bottom.
    pub fn prev(&self, k: usize) -> Mask {
        if k == 0 {
            return self.clone();
        }
        let (q, r) = (k / 64, k % 64);
        let mut out = Mask::zeros(self.n);
        for j in q..self.w.len() {
            let lo = self.w[j - q];
            let below = if j > q { self.w[j - q - 1] } else { 0 };
            out.w[j] = if r == 0 { lo } else { (lo << r) | (below >> (64 - r)) };
        }
        out.trim();
        out
    }
    /// Bit `i` is bit `i + k` of `self`.
    pub fn next(&self, k: usize) -> Mask {
        if k == 0 {
            return self.clone();
        }
        let (q, r) = (k / 64, k % 64);
        let mut out = Mask::zeros(self.n);
        let len = self.w.len();
        for j in 0..len {
            if j + q >= len {
                break;
            }
            let hi = self.w[j + q];
            let above = if j + q + 1 < len { self.w[j + q + 1] } else { 0 };
            out.w[j] = if r == 0 { hi } else { (hi >> r) | (above << (64 - r)) };
        }
        out.trim();
        out
    }
    /// Prefix XOR, with one parity bit carried from word to word.
    pub fn toggle(&self) -> Mask {
        let mut out = Mask::zeros(self.n);
        let mut carry = 0u64;
        for (j, &x) in self.w.iter().enumerate() {
            let mut p = x;
            for s in [1, 2, 4, 8, 16, 32] {
                p ^= p << s;
            }
            p ^= carry;
            out.w[j] = p;
            carry = if p >> 63 == 1 { !0 } else { 0 };
        }
        out.trim();
        out
    }
    /// `f[i] = s[i] | (f[i-1] & !e[i])`: from each start up to, not including, the next end.
    /// One add per word; the carry out of the add is the borrow passed to the next word.
    pub fn fill(s: &Mask, e: &Mask) -> Mask {
        let mut out = Mask::zeros(s.n);
        let mut cin = 0u64;
        for j in 0..s.w.len() {
            let (sw, ew) = (s.w[j], e.w[j]);
            let (a, b) = (sw, !ew | sw);
            let (t, o1) = a.overflowing_add(b);
            let (sum, o2) = t.overflowing_add(cin);
            let c = sum ^ a ^ b; // carries into each bit
            let f = sw | (!ew & c);
            out.w[j] = f;
            let _ = (o1, o2);
            cin = f >> 63;
        }
        out.trim();
        out
    }
    /// The mirror of `fill`: `b[i] = s[i] | (b[i+1] & !e[i])`. Words taken last to first, each
    /// bit-reversed so the same add runs on it.
    pub fn back_fill(s: &Mask, e: &Mask) -> Mask {
        let mut out = Mask::zeros(s.n);
        let mut cin = 0u64;
        for j in (0..s.w.len()).rev() {
            let (sw, ew) = (s.w[j].reverse_bits(), e.w[j].reverse_bits());
            let (a, b) = (sw, !ew | sw);
            let sum = a.wrapping_add(b).wrapping_add(cin);
            let c = sum ^ a ^ b;
            let f = sw | (!ew & c);
            out.w[j] = f.reverse_bits();
            cin = f >> 63;
        }
        out.trim();
        out
    }
    /// The byte after an odd run of set bits: the escaped bytes outside the run.
    pub fn after_odd_run(&self) -> Mask {
        self.escaped().andnot(self)
    }
    /// simdjson's escaped bytes (its escape scan, with one carry bit): each byte after an
    /// even-offset bit of a run, the run's own odd-offset bits included.
    pub fn escaped(&self) -> Mask {
        let mut out = Mask::zeros(self.n);
        let mut carry = 0u64;
        for (j, &bs) in self.w.iter().enumerate() {
            if bs == 0 {
                out.w[j] = carry;
                carry = 0;
                continue;
            }
            let potential = bs & !carry;
            let maybe_escaped = potential << 1;
            let series = (maybe_escaped | ODD).wrapping_sub(potential);
            let escape_and_terminal = series ^ ODD;
            let escaped = escape_and_terminal ^ (bs | carry);
            let escape = escape_and_terminal & bs;
            carry = escape >> 63;
            out.w[j] = escaped;
        }
        out.trim();
        out
    }
    /// The `k` bytes from each set bit: `self | prev(self) | … | prev(self, k-1)`.
    pub fn span(&self, k: usize) -> Mask {
        let mut out = self.clone();
        for j in 1..k {
            out = out.or(&self.prev(j));
        }
        out
    }
    /// Calls `f(start, end)` for each run of set bits, `end` exclusive.
    pub fn runs(&self, mut f: impl FnMut(usize, usize)) {
        let mut start: Option<usize> = None;
        let mut last = 0;
        for i in self.ones_iter() {
            match start {
                Some(_) if i == last + 1 => {}
                Some(s) => {
                    f(s, last + 1);
                    start = Some(i)
                }
                None => start = Some(i),
            }
            last = i;
        }
        if let Some(s) = start {
            f(s, last + 1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rnd(n: usize, seed: &mut u64, density: u64) -> Mask {
        Mask::from_fn(n, |_| {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 7;
            *seed ^= *seed << 17;
            *seed % 100 < density
        })
    }

    #[test]
    fn word_ops_match_bytewise() {
        let mut seed = 0x9e3779b97f4a7c15u64;
        for &n in &[1usize, 63, 64, 65, 200, 1000] {
            for d in [5u64, 30, 70] {
                let s = rnd(n, &mut seed, d);
                let e = rnd(n, &mut seed, d);
                // fill
                let mut f = vec![false; n];
                for i in 0..n {
                    f[i] = s.get(i) || (i > 0 && f[i - 1] && !e.get(i));
                }
                assert_eq!(Mask::fill(&s, &e), Mask::from_fn(n, |i| f[i]), "fill n={n}");
                // back_fill
                let mut b = vec![false; n];
                for i in (0..n).rev() {
                    b[i] = s.get(i) || (i + 1 < n && b[i + 1] && !e.get(i));
                }
                assert_eq!(Mask::back_fill(&s, &e), Mask::from_fn(n, |i| b[i]), "back_fill n={n}");
                // toggle
                let mut t = vec![false; n];
                for i in 0..n {
                    t[i] = s.get(i) ^ (i > 0 && t[i - 1]);
                }
                assert_eq!(s.toggle(), Mask::from_fn(n, |i| t[i]));
                // after_odd_run
                let mut run = 0;
                let mut a = vec![false; n];
                for i in 0..n {
                    if s.get(i) {
                        run += 1
                    } else {
                        a[i] = run % 2 == 1;
                        run = 0
                    }
                }
                assert_eq!(s.after_odd_run(), Mask::from_fn(n, |i| a[i]), "after_odd_run n={n} d={d}");
                // escaped
                let mut run = 0;
                let mut e = vec![false; n];
                for i in 0..n {
                    e[i] = run % 2 == 1;
                    run = if s.get(i) { run + 1 } else { 0 };
                }
                assert_eq!(s.escaped(), Mask::from_fn(n, |i| e[i]), "escaped n={n} d={d}");
                for k in [1usize, 2, 3, 9, 64, 70] {
                    assert_eq!(s.prev(k), Mask::from_fn(n, |i| i >= k && s.get(i - k)));
                    assert_eq!(s.next(k), Mask::from_fn(n, |i| s.get(i + k)));
                }
            }
        }
    }
}
