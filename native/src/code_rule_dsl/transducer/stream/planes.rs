//! Scans over bit-planes (`10` §2, `count`; `08` §2.2's mod-3 planes). A small number per byte is
//! held as planes: plane `p` is a mask of bit `p` of every byte's number. Arithmetic is ripple
//! logic across planes, a fixed number of mask operations per plane. A prefix over the block's 64
//! bytes is a Hillis–Steele scan: six rounds, shifting by 1, 2, 4, 8, 16, 32.
//!
//! Every loop here runs over the planes (`0..n`, fixed by the operation's parameters) or over
//! `SHIFTS`; none runs over bytes or depends on the data. A JIT unrolls them.

use super::program::MAX_PLANES;

/// The six Hillis–Steele distances for a 64-byte block.
pub(in crate::code_rule_dsl) const SHIFTS: [u32; 6] = [1, 2, 4, 8, 16, 32];

/// Planes for the signed working numbers of a clamped count: `max + 1 < 2^w` needs `w + 2`
/// planes for sums in `-2(max + 1)..=2(max + 1)`.
const WORK: usize = MAX_PLANES + 2;

/// A number per byte as planes, two's complement when signed. Only `planes[..n]` are used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl) struct Planes {
    pub(in crate::code_rule_dsl) planes: [u64; WORK],
    pub(in crate::code_rule_dsl) n: u8,
}

impl Planes {
    fn zero(n: usize) -> Self {
        Self { planes: [0; WORK], n: n as u8 }
    }

    /// The constant `c` at every byte, in `n` planes of two's complement.
    fn splat(c: i32, n: usize) -> Self {
        let mut r = Self::zero(n);
        for i in 0..n {
            r.planes[i] = 0u64.wrapping_sub(((c >> i) & 1) as u64);
        }
        r
    }

    fn len(&self) -> usize {
        usize::from(self.n)
    }

    /// The low `n` planes.
    pub(in crate::code_rule_dsl) fn truncate(mut self, n: usize) -> Self {
        for i in n..WORK {
            self.planes[i] = 0;
        }
        self.n = n as u8;
        self
    }

    /// The bytes whose number is `c` (unsigned, in the used planes).
    pub(in crate::code_rule_dsl) fn eq(&self, c: u16) -> u64 {
        let mut m = !0u64;
        for i in 0..self.len() {
            let bit = 0u64.wrapping_sub(u64::from((c >> i) & 1));
            m &= !(self.planes[i] ^ bit);
        }
        m
    }

    /// Byte `j`'s number, unsigned.
    pub(in crate::code_rule_dsl) fn at(&self, j: u32) -> u16 {
        let mut v = 0u16;
        for i in 0..self.len() {
            v |= (((self.planes[i] >> j) & 1) as u16) << i;
        }
        v
    }

    /// Byte `j`'s number, signed.
    fn at_signed(&self, j: u32) -> i32 {
        let n = self.len() as u32;
        let v = i32::from(self.at(j));
        (v << (32 - n)) >> (32 - n)
    }

    /// Every plane moved `s` bytes later, with `fill`'s planes in the first `s` bytes.
    fn shifted(&self, s: u32, fill: &Planes) -> Self {
        let low = (1u64 << s) - 1;
        let mut r = *self;
        for i in 0..self.len() {
            r.planes[i] = (self.planes[i] << s) | (fill.planes[i] & low);
        }
        r
    }
}

/// `(m & a) | (!m & b)`, plane by plane.
fn select(m: u64, a: &Planes, b: &Planes) -> Planes {
    let mut r = *a;
    for i in 0..a.len() {
        r.planes[i] = (m & a.planes[i]) | (!m & b.planes[i]);
    }
    r
}

/// `a + b + carry_in` mod `2^n`: a ripple-carry adder over the planes.
fn add(a: &Planes, b: &Planes, carry_in: u64) -> Planes {
    let mut r = *a;
    let mut c = carry_in;
    for i in 0..a.len() {
        let x = a.planes[i] ^ b.planes[i];
        r.planes[i] = x ^ c;
        c = (a.planes[i] & b.planes[i]) | (c & x);
    }
    r
}

/// `a - b` mod `2^n`.
fn sub(a: &Planes, b: &Planes) -> Planes {
    let mut nb = *b;
    for i in 0..b.len() {
        nb.planes[i] = !b.planes[i];
    }
    add(a, &nb, !0)
}

/// The bytes where `a < b`, both signed: the sign of `a - b` computed one plane wider.
fn lt(a: &Planes, b: &Planes) -> u64 {
    let n = a.len();
    let mut c = !0u64;
    for i in 0..n {
        let nb = !b.planes[i];
        let x = a.planes[i] ^ nb;
        c = (a.planes[i] & nb) | (c & x);
    }
    // The sign-extended top plane: a's sign plus not b's sign plus the carry.
    a.planes[n - 1] ^ !b.planes[n - 1] ^ c
}

fn min(a: &Planes, b: &Planes) -> Planes {
    select(lt(a, b), a, b)
}

fn max(a: &Planes, b: &Planes) -> Planes {
    select(lt(a, b), b, a)
}

fn clamp(x: &Planes, lo: &Planes, hi: &Planes) -> Planes {
    min(&max(x, lo), hi)
}

/// The bytes where an unsigned `a >= c`.
fn ge_const(a: &Planes, c: u32) -> u64 {
    let mut gt = 0u64;
    let mut eq = !0u64;
    for i in (0..a.len()).rev() {
        let cb = 0u64.wrapping_sub(u64::from((c >> i) & 1));
        gt |= eq & a.planes[i] & !cb;
        eq &= !(a.planes[i] ^ cb);
    }
    gt | eq
}

/// `a + b` mod `m`, both in `0..m`: an adder one plane wider, then `m` taken off where the sum
/// reached it.
fn add_mod(a: &Planes, b: &Planes, m: u32) -> Planes {
    let n = a.len();
    let wide = add(&a.truncate(n + 1), &b.truncate(n + 1), 0);
    let over = ge_const(&wide, m);
    let less = sub(&wide, &Planes::splat(m as i32, n + 1));
    select(over, &less, &wide).truncate(n)
}

// --- A count mod n ------------------------------------------------------------------------

/// A count mod `n`'s summary of a block: its net change after its last reset, and whether it has
/// one. It is a map `x -> reset ? v : x + v`; maps compose associatively.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl) struct ModSummary {
    pub(in crate::code_rule_dsl) v: u8,
    pub(in crate::code_rule_dsl) reset: bool,
}

/// The per-byte steps of a count mod `n`: `+1` at `up`, `-1` at `down`, as numbers mod `n`.
fn mod_steps(n: u32, w: usize, up: u64, down: u64) -> Planes {
    let u = up & !down;
    let d = down & !up;
    let mut r = Planes::zero(w);
    for i in 0..w {
        r.planes[i] = (if i == 0 { u } else { 0 }) | (0u64.wrapping_sub(u64::from(((n - 1) >> i) & 1)) & d);
    }
    r
}

/// The block's count mod `n` at every byte, given the count before it. Returns the planes and
/// the count after the block.
pub(in crate::code_rule_dsl) fn scan_mod(
    n: u8,
    up: u64,
    down: u64,
    reset: u64,
    carry: u8,
) -> (Planes, u8) {
    let w = usize::from(super::program::mod_planes(n));
    let m = u32::from(n);
    let mut v = mod_steps(m, w, up, down);
    let mut f = reset;
    let none = Planes::zero(w);
    for s in SHIFTS {
        let earlier = v.shifted(s, &none);
        // Add the earlier partial sum only where no reset has been seen since it.
        let take = select(!f, &earlier, &none);
        v = add_mod(&v, &take, m);
        f |= f << s;
    }
    let before = select(!f, &Planes::splat(i32::from(carry), w), &none);
    let out = add_mod(&v, &before, m);
    let after = out.at(63) as u8;
    (out, after)
}

/// The block's summary for a count mod `n`: from popcounts, no scan.
pub(in crate::code_rule_dsl) fn summary_mod(n: u8, up: u64, down: u64, reset: u64) -> ModSummary {
    // The bytes after the last reset (all of them when there is none).
    let last = 63 - reset.leading_zeros().min(63);
    let tail = if reset == 0 { !0 } else { !0u64 << last };
    let u = (up & !down & tail).count_ones();
    let d = (down & !up & tail).count_ones();
    let m = u32::from(n);
    let v = ((u + 64 * m - d) % m) as u8;
    ModSummary { v, reset: reset != 0 }
}

/// `first` then `then`.
pub(in crate::code_rule_dsl) fn combine_mod(n: u8, first: ModSummary, then: ModSummary) -> ModSummary {
    if then.reset {
        then
    } else {
        ModSummary { v: ((u32::from(first.v) + u32::from(then.v)) % u32::from(n)) as u8, reset: first.reset }
    }
}

pub(in crate::code_rule_dsl) fn apply_mod(n: u8, s: ModSummary, carry: u8) -> u8 {
    if s.reset { s.v } else { ((u32::from(carry) + u32::from(s.v)) % u32::from(n)) as u8 }
}

// --- A clamped count ----------------------------------------------------------------------

/// A clamp map `x -> clamp(x + d, lo, hi)` on `0..=top`. Clamp maps are closed under
/// composition, so a block's effect on a clamped count is one map, and its summary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl) struct ClampMap {
    pub(in crate::code_rule_dsl) d: i16,
    pub(in crate::code_rule_dsl) lo: u16,
    pub(in crate::code_rule_dsl) hi: u16,
}

/// The same maps per byte, as planes.
#[derive(Clone, Copy)]
struct Maps {
    d: Planes,
    lo: Planes,
    hi: Planes,
}

/// `first` then `then`, per byte: `d = sat(d1 + d2)`, `lo = clamp(lo1 + d2, lo2, hi2)`,
/// `hi = clamp(hi1 + d2, lo2, hi2)`. `d` saturates at `±top`, past which it cannot matter.
fn compose(first: &Maps, then: &Maps, top: &Planes, neg_top: &Planes) -> Maps {
    let d = clamp(&add(&first.d, &then.d, 0), neg_top, top);
    let lo = clamp(&add(&first.lo, &then.d, 0), &then.lo, &then.hi);
    let hi = clamp(&add(&first.hi, &then.d, 0), &then.lo, &then.hi);
    Maps { d, lo, hi }
}

/// The block's maps through every byte, each the composition of the byte's own with every
/// earlier byte's since the block began.
fn clamp_prefix(max: u8, up: u64, down: u64, reset: u64) -> (Maps, Planes, Planes) {
    let top = i32::from(max) + 1;
    let n = usize::from(super::program::clamp_planes(max)) + 2;
    let u = up & !down;
    let dn = down & !up;
    // d: +1 at u, -1 (all planes set) at dn.
    let mut d = Planes::zero(n);
    d.planes[0] = u | dn;
    for i in 1..n {
        d.planes[i] = dn;
    }
    // At a reset the map is the constant clamp(d, 0, top): 1 after an up, else 0.
    let top_p = Planes::splat(top, n);
    let zero = Planes::zero(n);
    let one_at = |m: u64| {
        let mut p = zero;
        p.planes[0] = m;
        p
    };
    let lo = one_at(reset & u);
    let hi = select(reset, &one_at(u), &top_p);
    let identity = Maps { d: zero, lo: zero, hi: top_p };
    let neg_top = Planes::splat(-top, n);
    let mut maps = Maps { d, lo, hi };
    for s in SHIFTS {
        let earlier = Maps {
            d: maps.d.shifted(s, &identity.d),
            lo: maps.lo.shifted(s, &identity.lo),
            hi: maps.hi.shifted(s, &identity.hi),
        };
        maps = compose(&earlier, &maps, &top_p, &neg_top);
    }
    (maps, top_p, neg_top)
}

/// The block's clamped count at every byte, given the count before it. Returns the planes
/// (`clamp_planes(max)` of them) and the count after the block.
pub(in crate::code_rule_dsl) fn scan_clamp(
    max: u8,
    up: u64,
    down: u64,
    reset: u64,
    carry: u16,
) -> (Planes, u16) {
    let (maps, _, _) = clamp_prefix(max, up, down, reset);
    let n = maps.d.len();
    let x = add(&Planes::splat(i32::from(carry), n), &maps.d, 0);
    let out = clamp(&x, &maps.lo, &maps.hi).truncate(usize::from(super::program::clamp_planes(max)));
    let after = out.at(63);
    (out, after)
}

/// The block's clamp map: the last byte's prefix map.
pub(in crate::code_rule_dsl) fn summary_clamp(max: u8, up: u64, down: u64, reset: u64) -> ClampMap {
    let (maps, _, _) = clamp_prefix(max, up, down, reset);
    ClampMap {
        d: maps.d.at_signed(63) as i16,
        lo: maps.lo.at_signed(63) as u16,
        hi: maps.hi.at_signed(63) as u16,
    }
}

/// `first` then `then`.
pub(in crate::code_rule_dsl) fn combine_clamp(max: u8, first: ClampMap, then: ClampMap) -> ClampMap {
    let top = i32::from(max) + 1;
    let c = |x: i32| x.clamp(i32::from(then.lo), i32::from(then.hi)) as u16;
    ClampMap {
        d: (i32::from(first.d) + i32::from(then.d)).clamp(-top, top) as i16,
        lo: c(i32::from(first.lo) + i32::from(then.d)),
        hi: c(i32::from(first.hi) + i32::from(then.d)),
    }
}

pub(in crate::code_rule_dsl) fn apply_clamp(s: ClampMap, carry: u16) -> u16 {
    (i32::from(carry) + i32::from(s.d)).clamp(i32::from(s.lo), i32::from(s.hi)) as u16
}
