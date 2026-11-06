//! The stream program's operations: the contract every backend implements
//! (`docs/parser-generators/09-compile-and-lower.md` §3).
//!
//! Every operation is a kernel over one 64-byte block. A mask is one `u64` per block, bit `i` for
//! byte `i`; lanes are 64 bytes per block, one small integer per byte. An operation's cost per
//! block is fixed: no work depends on how many bits are set, where events are, or how long a
//! segment or run is. What crosses a block edge is the operation's carry, listed below; it is
//! reset at a file's first block (backward scans: its last), so files are independent.
//!
//! The first operations are the Metal set steps' (`metal_set_steps.rs` `SetStep`: clear, fill,
//! or, and, and-not) over the same layout: each file from a block boundary.
//!
//! | operation | carry |
//! |---|---|
//! | `Empty` `All` `FileFirst` `FileLast` `Or` `And` `AndNot` `Xor` `Not` `Pattern` | none |
//! | `Prev{k}` | the block before's word |
//! | `Next{k}` | the block after's word (read ahead, as simdjson's indexer runs a block behind) |
//! | `Toggle` | 1 parity bit |
//! | `Fill` | 1 borrow bit |
//! | `BackFill` | 1 borrow bit, backwards |
//! | `Classify` `LaneTest` `LaneConst` `LaneSelect` `LaneMap` `LaneEq` `LaneGe` `LaneEqLanes` | none |
//! | `LanePrev` | the block before's last lane |
//! | `MapScan` | the state after the block before: one lane |
//! | `Count` | the count after the block before: one lane |
//! | `Ucode` | the block before's last three bytes' cover and class bits; reads 3 bytes ahead |
//! | `Carry` `RoundsBegin` `RoundsEnd` | control: a loop over a range of operations |
//! | `Errors` | output: the error records of its masks |

pub(in crate::code_rule_dsl) type Id = usize;

/// The largest state count of a `MapScan`; state 15 is a sink that every map keeps.
pub(in crate::code_rule_dsl) const STATES: usize = 16;
pub(in crate::code_rule_dsl) const SINK: u8 = 15;

#[derive(Clone, Debug)]
pub(in crate::code_rule_dsl) enum Op {
    Empty,
    /// Every byte of every file.
    All,
    FileFirst,
    FileLast,
    Or(Id, Id),
    And(Id, Id),
    AndNot(Id, Id),
    Xor(Id, Id),
    Not(Id),
    /// Bytes whose position `p` in the batch has `p % k == c`. Only differences of positions are
    /// read, and files start at block boundaries, so a run's residues are the file's.
    Pattern { k: u32, c: u32 },
    /// Bit `i` is bit `i - k` of `a`, `1 <= k <= 64`.
    Prev { a: Id, k: u32 },
    /// Bit `i` is bit `i + k` of `a`, `1 <= k <= 64`.
    Next { a: Id, k: u32 },
    /// Prefix XOR.
    Toggle(Id),
    /// `f[i] = s[i] | (f[i-1] & !e[i])`: an add with carry per word.
    Fill { s: Id, e: Id },
    /// `b[i] = s[i] | (b[i+1] & !e[i])`: `Fill` on bit-reversed words, last block first.
    BackFill { s: Id, e: Id },
    /// Lanes: each byte's set of the group's (at most 8) classes, by a 256-entry table
    /// (`Program::groups`); NEON's `vqtbl4q_u8` four times.
    Classify(usize),
    /// A mask: the lanes holding `bit`.
    LaneTest { a: Id, bit: u8 },
    LaneConst(u8),
    /// Lanes of `a` where `m` is set, of `b` elsewhere.
    LaneSelect { m: Id, a: Id, b: Id },
    /// Each lane `x` becomes `table[x]`.
    LaneMap { a: Id, table: Box<[u8; 256]> },
    LaneEq { a: Id, k: u8 },
    LaneGe { a: Id, k: u8 },
    LaneEqLanes(Id, Id),
    /// Lane `i` is lane `i - 1` of `a`; `init` before a file's first byte.
    LanePrev { a: Id, init: u8 },
    /// A walk over at most 15 states, as a scan of maps: byte `i`'s map sends state `s` to lane
    /// `i` of `entries[s]`. The maps are composed in six steps of a Hillis–Steele scan within
    /// the block; the lanes are the states after each byte, from `init` at a file's start.
    MapScan { entries: Vec<Id>, init: u8 },
    /// `x ↦ clamp(x + a, lo, hi)` maps, composed by the same scan: +1 at `up` (at most
    /// `max + 1`), -1 at `down` (at least 0), 0 at `reset`; at `first`, the step from 0.
    Count { up: Id, down: Id, reset: Option<Id>, first: Option<Id>, max: u8 },
    /// UTF-8 characters whose first byte is in `over`, a mask per class of code points over each
    /// character's bytes. A block with no byte of 0x80 or above takes simdjson's ASCII path.
    Ucode { over: Id, classes: Vec<Membership> },
    /// Mask `k` of a multi-mask operation.
    Pick(Id, usize),
    /// A loop's carry: `init` on entry, then the loop's `next`.
    Carry(Id),
    RoundsBegin,
    /// Repeats the operations after `begin` while a carry changed and fewer than `n` rounds ran;
    /// without `settle`, all `n` rounds run. Its value: the bytes still changing after the last.
    RoundsEnd { begin: usize, n: usize, settle: bool, carries: Vec<(Id, Id)> },
    /// Error records: `all` = every set byte, ordered by position across the parts; otherwise
    /// each part's first byte in each file. Without `byte`, one record per file, no position.
    Errors { parts: Vec<(String, Id)>, all: bool, byte: bool },
}

/// Code points as a two-level table: `index[cp >> 8]` names a 256-bit page of `pages`.
#[derive(Clone, Debug)]
pub(in crate::code_rule_dsl) struct Membership {
    pub index: Vec<u16>,
    pub pages: Vec<[u64; 4]>,
}

impl Membership {
    pub(in crate::code_rule_dsl) fn new(ranges: &[(u32, u32)]) -> Self {
        let mut pages: Vec<[u64; 4]> = vec![[0; 4]];
        let mut index = vec![0u16; 0x1100 + 1];
        for &(lo, hi) in ranges {
            for cp in lo..=hi.min(0x10ffff) {
                let p = (cp >> 8) as usize;
                if index[p] == 0 {
                    pages.push([0; 4]);
                    index[p] = (pages.len() - 1) as u16;
                }
                let b = (cp & 0xff) as usize;
                pages[index[p] as usize][b / 64] |= 1 << (b % 64);
            }
        }
        Membership { index, pages }
    }
    #[inline]
    pub(in crate::code_rule_dsl) fn has(&self, cp: u32) -> bool {
        let page = &self.pages[self.index[(cp >> 8).min(0x1100) as usize] as usize];
        let b = (cp & 0xff) as usize;
        page[b / 64] >> (b % 64) & 1 == 1
    }
}

impl Op {
    /// The values the operation reads.
    pub(in crate::code_rule_dsl) fn inputs(&self) -> Vec<Id> {
        match self {
            Op::Empty | Op::All | Op::FileFirst | Op::FileLast | Op::Pattern { .. } | Op::Classify(_) | Op::LaneConst(_) | Op::RoundsBegin => vec![],
            Op::Or(a, b) | Op::And(a, b) | Op::AndNot(a, b) | Op::Xor(a, b) | Op::LaneEqLanes(a, b) => vec![*a, *b],
            Op::Fill { s, e } | Op::BackFill { s, e } => vec![*s, *e],
            Op::Not(a) | Op::Toggle(a) | Op::Pick(a, _) | Op::Carry(a) => vec![*a],
            Op::Prev { a, .. } | Op::Next { a, .. } | Op::LaneTest { a, .. } | Op::LaneMap { a, .. } | Op::LaneEq { a, .. } | Op::LaneGe { a, .. } | Op::LanePrev { a, .. } => vec![*a],
            Op::LaneSelect { m, a, b } => vec![*m, *a, *b],
            Op::MapScan { entries, .. } => entries.clone(),
            Op::Count { up, down, reset, first, .. } => [vec![*up, *down], reset.iter().copied().collect(), first.iter().copied().collect()].concat(),
            Op::Ucode { over, .. } => vec![*over],
            Op::RoundsEnd { carries, .. } => carries.iter().flat_map(|&(c, n)| [c, n]).collect(),
            Op::Errors { parts, .. } => parts.iter().map(|p| p.1).collect(),
        }
    }
}

/// A token kind's start mask and the mask it continues through.
#[derive(Clone, Debug)]
pub(in crate::code_rule_dsl) struct TokenDef {
    pub kind: String,
    pub start: Id,
    pub through: Option<Id>,
}

#[derive(Clone, Debug)]
pub(in crate::code_rule_dsl) struct Program {
    pub ops: Vec<Op>,
    /// Operations nothing reads (and that write no output) are not run.
    pub live: Vec<bool>,
    /// The class tables of the `Classify` operations: per byte value, its classes as bits.
    pub groups: Vec<Box<[u8; 256]>>,
    pub tokens: Vec<TokenDef>,
    pub munch: Vec<(String, Id, Vec<(Vec<u8>, Option<String>)>)>,
    pub keywords: Vec<(String, Vec<(Vec<u8>, Option<String>)>, Vec<Vec<u8>>, bool)>,
    pub skip: Id,
}

impl Program {
    pub(in crate::code_rule_dsl) fn mark_live(&mut self) {
        let mut live = vec![false; self.ops.len()];
        let mut stack: Vec<Id> = vec![self.skip];
        stack.extend(self.tokens.iter().flat_map(|t| std::iter::once(t.start).chain(t.through)));
        stack.extend(self.munch.iter().map(|m| m.1));
        for (id, op) in self.ops.iter().enumerate() {
            if matches!(op, Op::Errors { .. } | Op::RoundsBegin | Op::RoundsEnd { .. }) {
                stack.push(id);
            }
        }
        while let Some(id) = stack.pop() {
            if !live[id] {
                live[id] = true;
                stack.extend(self.ops[id].inputs());
            }
        }
        self.live = live;
    }

    pub(in crate::code_rule_dsl) fn live_ops(&self) -> usize {
        self.live.iter().filter(|&&l| l).count()
    }

    /// For each value, the operation after which nothing reads it: its last reader, or the end
    /// of the outermost loop that reads it without defining it.
    pub(in crate::code_rule_dsl) fn last_uses(&self) -> Vec<usize> {
        let loops: Vec<(usize, usize)> = self
            .ops
            .iter()
            .enumerate()
            .filter_map(|(e, op)| if let Op::RoundsEnd { begin, .. } = op { Some((*begin, e)) } else { None })
            .collect();
        let mut last: Vec<usize> = (0..self.ops.len()).collect();
        for (u, op) in self.ops.iter().enumerate() {
            if !self.live[u] {
                continue;
            }
            for d in op.inputs() {
                let mut at = u;
                for &(b, e) in &loops {
                    if b <= u && u <= e && !(b <= d && d <= e) {
                        at = at.max(e);
                    }
                }
                last[d] = last[d].max(at);
            }
        }
        let end = self.ops.len();
        for t in &self.tokens {
            last[t.start] = end;
            if let Some(x) = t.through {
                last[x] = end;
            }
        }
        for m in &self.munch {
            last[m.1] = end;
        }
        last[self.skip] = end;
        last
    }
}
