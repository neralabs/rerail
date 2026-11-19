//! Tier 2's monoids (docs/parser-generators/15-analysis-and-plans.md §2): what a scan carries,
//! and how two carries combine. `a.then(b)` is `a`'s bytes followed by `b`'s. Every scan of a
//! plan is one of these, and the generators emit the same combines as CLIF and MSL. The tests'
//! walks are their sequential references (`block_plan_tests`).
//!
//! `add` (counts) and `or` (errors) are plain integer operations and have no type here.

/// A function from a state of `inputs` bits to one of `outputs` bits: the next state in the low
/// `inputs` bits, then halos (the last byte's values that the next block reads). S1 is one, from
/// `(escaped, inside)` to those and the halos of `content` and `word` (doc 16 §4.2).
///
/// The identity has no bytes, and so no halos of its own: applied to a full state, it keeps the
/// halos coming in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct StateFn {
    pub(super) inputs: u8,
    pub(super) outputs: u8,
    /// Entry `x`, `outputs` bits at bit `x * outputs`.
    pub(super) table: u64,
    pub(super) id: bool,
}

impl StateFn {
    pub(super) fn identity(inputs: u8, outputs: u8) -> Self {
        assert!(inputs <= outputs && (1u32 << inputs) * u32::from(outputs) <= 64, "a state function's table is one word");
        StateFn { inputs, outputs, table: 0, id: true }
    }

    pub(super) fn from(inputs: u8, outputs: u8, f: impl Fn(u64) -> u64) -> Self {
        let mut s = Self::identity(inputs, outputs);
        let mask = (1u64 << outputs) - 1;
        s.table = (0..1u64 << inputs).map(|x| (f(x) & mask) << (x * u64::from(outputs))).sum();
        s.id = false;
        s
    }

    /// The table's bits, as stored in an entry.
    pub(super) fn bits(self) -> u32 {
        (1u32 << self.inputs) * u32::from(self.outputs)
    }

    /// A full state (state bits, then halos) through the function.
    pub(super) fn apply(self, state: u64) -> u64 {
        if self.id {
            return state;
        }
        let x = state & ((1u64 << self.inputs) - 1);
        (self.table >> (x * u64::from(self.outputs))) & ((1u64 << self.outputs) - 1)
    }

    pub(super) fn then(self, b: StateFn) -> StateFn {
        match (self.id, b.id) {
            (true, _) => b,
            (_, true) => self,
            _ => StateFn::from(self.inputs, self.outputs, |x| b.apply(self.apply(x))),
        }
    }
}

/// The latest value present, scanning forward: a `last` scan's carry (the previous token's kind,
/// the last token start).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct Last(pub(super) Option<u64>);

impl Last {
    pub(super) fn then(self, b: Last) -> Last {
        Last(b.0.or(self.0))
    }
}

/// The earliest value present: a backward `last` scan's carry (the next token's kind). `a.then(b)`
/// is still `a`'s bytes before `b`'s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct First(pub(super) Option<u64>);

impl First {
    pub(super) fn then(self, b: First) -> First {
        First(self.0.or(b.0))
    }
}

/// A piece of the file's effect on the stack of open brackets (doc 14 §3), in 41 bits:
///
/// | bits | field |
/// |---|---|
/// | 0..4 | closes: how many, of brackets opened before the piece |
/// | 4..17 | the kinds those closes expect, innermost first |
/// | 17..21 | opens: how many it leaves open |
/// | 21..34 | their kinds, innermost first |
/// | 34 | the innermost object's pair is open at the piece's end |
/// | 35 | the piece set the pair bit (else it passes the one before through) |
/// | 36 | a close met an open of the other kind |
/// | 37..41 | the peak: the most levels above the piece's start, saturating at 15 |
///
/// A close reveals an object whose pair is open: the container it closed was that pair's value.
/// A file fails (`fails`) on a mismatch, on a close with nothing open, or past `max` levels.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct Dyck(pub(super) u64);

const KINDS: u64 = 0x1fff;
const PAIR: u64 = 1 << 34;
const SET: u64 = 1 << 35;
const MISMATCH: u64 = 1 << 36;

impl std::fmt::Debug for Dyck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Dyck {{ closes {} {:013b}, opens {} {:013b}, pair {} set {}, mismatch {}, peak {} }}",
            self.closes(),
            (self.0 >> 4) & KINDS,
            self.opens(),
            (self.0 >> 21) & KINDS,
            self.pair(),
            self.0 & SET != 0,
            self.0 & MISMATCH != 0,
            self.peak()
        )
    }
}

impl Dyck {
    pub(super) const ID: Dyck = Dyck(0);

    pub(super) fn open(kind: u8) -> Dyck {
        Dyck(1 << 17 | u64::from(kind & 1) << 21 | SET | 1 << 37)
    }

    pub(super) fn close(kind: u8) -> Dyck {
        Dyck(1 | u64::from(kind & 1) << 4 | PAIR | SET)
    }

    /// A key opens the innermost object's pair; a comma closes it.
    pub(super) fn key() -> Dyck {
        Dyck(PAIR | SET)
    }

    pub(super) fn comma() -> Dyck {
        Dyck(SET)
    }

    pub(super) fn closes(self) -> u8 {
        (self.0 & 15) as u8
    }

    pub(super) fn opens(self) -> u8 {
        ((self.0 >> 17) & 15) as u8
    }

    /// The kind of open `i`, innermost first.
    pub(super) fn open_kind(self, i: u8) -> u8 {
        ((self.0 >> (21 + u32::from(i))) & 1) as u8
    }

    pub(super) fn pair(self) -> bool {
        self.0 & PAIR != 0
    }

    pub(super) fn peak(self) -> u8 {
        ((self.0 >> 37) & 15) as u8
    }

    pub(super) fn error(self) -> bool {
        self.0 & MISMATCH != 0
    }

    /// From the file's start: whether the file fails.
    pub(super) fn fails(self, max: u8) -> bool {
        self.error() || self.closes() > 0 || self.peak() > max
    }

    pub(super) fn then(self, b: Dyck) -> Dyck {
        let (a, bb) = (self.0, b.0);
        let (ac, ao) = (a & 15, (a >> 17) & 15);
        let (bc, bo) = (bb & 15, (bb >> 17) & 15);
        let m = ao.min(bc);
        let (ak, bk) = ((a >> 21) & KINDS, (bb >> 4) & KINDS);
        let mm = (1u64 << m) - 1;
        let mismatch = (a | bb) & MISMATCH != 0 || (ak ^ bk) & mm != 0;
        let closes = (ac + bc - m).min(15);
        let opens = (ao - m + bo).min(15);
        let ck = (((a >> 4) & KINDS) | ((bk >> m) << ac)) & KINDS;
        let ok = (((bb >> 21) & KINDS & ((1u64 << bo) - 1)) | ((ak >> m) << bo)) & KINDS;
        // The pair: `b`'s if it set it, else `a`'s, passed through.
        let pair = if bb & SET != 0 { bb & (PAIR | SET) } else { a & (PAIR | SET) };
        // Heights from `a`'s start: `a` ends at `ao - ac`, and `b` climbs `b.peak` from there.
        let height = ao as i64 - ac as i64;
        let peak = (self.peak() as i64).max(height + b.peak() as i64).clamp(0, 15) as u64;
        Dyck(closes | ck << 4 | opens << 17 | ok << 21 | pair | u64::from(mismatch) << 36 | peak << 37)
    }
}
