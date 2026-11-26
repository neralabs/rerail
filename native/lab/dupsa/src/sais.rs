//! SA-IS (Nong, Zhang and Chan 2009), the algorithm of the sais-lite MLDB vendors (`ext/sais-lite-lcp`), over a
//! text read through `Text`: the symbols are never copied into an array first. The recursion's reduced string is the
//! algorithm's own working state (ranks of LMS substrings), not a copy of the text.
//!
//! The shape follows the AtCoder Library's `sa_is` (induced sorting with L/S bucket heads), generic over the text.

/// A text of symbols in `0..=upper`, read at any position.
pub trait Text {
    fn len(&self) -> usize;
    fn at(&self, i: usize) -> u32;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Text for [u32] {
    fn len(&self) -> usize {
        <[u32]>::len(self)
    }
    #[inline(always)]
    fn at(&self, i: usize) -> u32 {
        self[i]
    }
}

impl Text for Vec<u32> {
    fn len(&self) -> usize {
        <[u32]>::len(self)
    }
    #[inline(always)]
    fn at(&self, i: usize) -> u32 {
        self[i]
    }
}

const NONE: u32 = u32::MAX;

/// The suffix array of `s`, whose symbols lie in `0..=upper`.
pub fn suffix_array<T: Text + ?Sized>(s: &T, upper: u32) -> Vec<u32> {
    let n = s.len();
    assert!(n < NONE as usize, "a text of 2^32 symbols or more");
    match n {
        0 => return Vec::new(),
        1 => return vec![0],
        2 => return if s.at(0) < s.at(1) { vec![0, 1] } else { vec![1, 0] },
        _ if n < 10 => return naive(s),
        _ => {}
    }
    let upper = upper as usize;
    let mut sa = vec![NONE; n];
    // ls[i]: suffix i is S-type (smaller than suffix i + 1).
    let mut ls = vec![false; n];
    for i in (0..n - 1).rev() {
        let (a, b) = (s.at(i), s.at(i + 1));
        ls[i] = if a == b { ls[i + 1] } else { a < b };
    }
    // Bucket heads: sum_l[c] the first slot of c's L-bucket, sum_s[c] the first of its S-bucket.
    let mut sum_l = vec![0u32; upper + 2];
    let mut sum_s = vec![0u32; upper + 2];
    for (i, &l) in ls.iter().enumerate() {
        let c = s.at(i) as usize;
        if !l {
            sum_s[c] += 1;
        } else {
            sum_l[c + 1] += 1;
        }
    }
    for c in 0..=upper {
        sum_s[c] += sum_l[c];
        if c < upper {
            sum_l[c + 1] += sum_s[c];
        }
    }
    let mut buf = vec![0u32; upper + 2];
    let mut induce = |sa: &mut [u32], lms: &[u32]| {
        sa.fill(NONE);
        buf.copy_from_slice(&sum_s);
        for &d in lms {
            let d = d as usize;
            if d == n {
                continue;
            }
            let c = s.at(d) as usize;
            sa[buf[c] as usize] = d as u32;
            buf[c] += 1;
        }
        buf.copy_from_slice(&sum_l);
        let c = s.at(n - 1) as usize;
        sa[buf[c] as usize] = (n - 1) as u32;
        buf[c] += 1;
        for i in 0..n {
            let v = sa[i];
            if v != NONE && v >= 1 && !ls[v as usize - 1] {
                let c = s.at(v as usize - 1) as usize;
                sa[buf[c] as usize] = v - 1;
                buf[c] += 1;
            }
        }
        buf.copy_from_slice(&sum_l);
        for i in (0..n).rev() {
            let v = sa[i];
            if v != NONE && v >= 1 && ls[v as usize - 1] {
                let c = s.at(v as usize - 1) as usize + 1;
                buf[c] -= 1;
                sa[buf[c] as usize] = v - 1;
            }
        }
    };
    let is_lms = |i: usize| i >= 1 && i < n && !ls[i - 1] && ls[i];
    let mut lms_map = vec![NONE; n + 1];
    let mut lms = Vec::new();
    for i in 1..n {
        if is_lms(i) {
            lms_map[i] = lms.len() as u32;
            lms.push(i as u32);
        }
    }
    let m = lms.len();
    induce(&mut sa, &lms);
    if m > 0 {
        let sorted: Vec<u32> = sa.iter().copied().filter(|&v| v != NONE && lms_map[v as usize] != NONE).collect();
        let mut rec = vec![0u32; m];
        let mut rec_upper = 0u32;
        rec[lms_map[sorted[0] as usize] as usize] = 0;
        let end_of = |p: usize| {
            let k = lms_map[p] as usize + 1;
            if k < m { lms[k] as usize } else { n }
        };
        for w in sorted.windows(2) {
            let (mut l, mut r) = (w[0] as usize, w[1] as usize);
            let (end_l, end_r) = (end_of(l), end_of(r));
            let mut same = end_l - l == end_r - r;
            if same {
                while l < end_l && s.at(l) == s.at(r) {
                    l += 1;
                    r += 1;
                }
                if l == n || s.at(l) != s.at(r) {
                    same = false;
                }
            }
            if !same {
                rec_upper += 1;
            }
            rec[lms_map[w[1] as usize] as usize] = rec_upper;
        }
        let rec_sa = suffix_array(&rec, rec_upper);
        let sorted: Vec<u32> = rec_sa.iter().map(|&r| lms[r as usize]).collect();
        induce(&mut sa, &sorted);
    }
    sa
}

fn naive<T: Text + ?Sized>(s: &T) -> Vec<u32> {
    let n = s.len();
    let mut sa: Vec<u32> = (0..n as u32).collect();
    sa.sort_by(|&a, &b| compare(s, a as usize, b as usize));
    sa
}

/// Suffix `a` against suffix `b`.
pub fn compare<T: Text + ?Sized>(s: &T, mut a: usize, mut b: usize) -> std::cmp::Ordering {
    let n = s.len();
    while a < n && b < n {
        match s.at(a).cmp(&s.at(b)) {
            std::cmp::Ordering::Equal => {
                a += 1;
                b += 1;
            }
            o => return o,
        }
    }
    // The shorter suffix (the one that reached the end) is smaller.
    (n - a).cmp(&(n - b))
}

/// Kasai's LCP array: `lcp[r]` is the common prefix of suffixes `sa[r - 1]` and `sa[r]` (`lcp[0]` = 0).
pub fn lcp<T: Text + ?Sized>(s: &T, sa: &[u32]) -> Vec<u32> {
    let n = sa.len();
    let mut rank = vec![0u32; n];
    for (r, &p) in sa.iter().enumerate() {
        rank[p as usize] = r as u32;
    }
    let mut lcp = vec![0u32; n];
    let mut h = 0usize;
    for i in 0..n {
        let r = rank[i] as usize;
        if r == 0 {
            h = 0;
            continue;
        }
        let j = sa[r - 1] as usize;
        while i + h < n && j + h < n && s.at(i + h) == s.at(j + h) {
            h += 1;
        }
        lcp[r] = h as u32;
        h = h.saturating_sub(1);
    }
    lcp
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lcg(seed: &mut u64) -> u32 {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (*seed >> 33) as u32
    }

    #[test]
    fn matches_a_sort_of_every_suffix() {
        let mut seed = 7u64;
        for n in [0usize, 1, 2, 3, 9, 10, 11, 40, 100, 1000, 5000] {
            for upper in [0u32, 1, 3, 25, 1000] {
                let s: Vec<u32> = (0..n).map(|_| lcg(&mut seed) % (upper + 1)).collect();
                let sa = suffix_array(&s, upper);
                assert_eq!(sa, naive(s.as_slice()), "n {n} upper {upper}");
                let l = lcp(&s, &sa);
                for r in 1..n {
                    let (a, b) = (sa[r - 1] as usize, sa[r] as usize);
                    let mut h = 0;
                    while a + h < n && b + h < n && s[a + h] == s[b + h] {
                        h += 1;
                    }
                    assert_eq!(l[r] as usize, h);
                }
            }
        }
    }

    #[test]
    fn repetitive_texts() {
        for s in [vec![0u32; 500], (0..600).map(|i| (i % 3) as u32).collect(), (0..700).map(|i| u32::from(i % 7 == 0)).collect()] {
            let upper = *s.iter().max().unwrap();
            assert_eq!(suffix_array(&s, upper), naive(s.as_slice()));
        }
    }
}
