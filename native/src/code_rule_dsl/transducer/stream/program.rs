//! The stream program: a straight-line list of instructions over masks (`10` §3.1).
//!
//! Instruction `i` defines value `%i`. Its operands are earlier values. A value has one of three
//! types: a mask (one `u64` per 64-byte block, bit `j` for byte `j`), bit-planes (a small number
//! per byte, one mask per bit), or a count (a running total per block, for the extract pass).
//!
//! There is no operation that walks bytes, events or tokens, and none that branches on data. The
//! only control construct is `rounds`, a fixed number of repetitions of a body with values fed
//! back from one round to the next (`08` §7), which may stop early once a round changes nothing
//! (`10` §4, Q2).

use super::verify::{self, Refusal};
use sha2::{Digest as _, Sha256};

/// A value: the index of the instruction that defines it.
pub(in crate::code_rule_dsl) type Value = u32;

/// The largest number of bit-planes a value has: a clamped count up to 255 needs 9.
pub(in crate::code_rule_dsl) const MAX_PLANES: usize = 9;

/// A value's type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl) enum Ty {
    /// One bit per byte.
    Mask,
    /// A number per byte, as this many planes (bit `p` of the number in plane `p`).
    Planes(u8),
    /// A running total of set bits, per block.
    Count,
}

/// A nibble table pair for `class` (`03` §3.1): byte `b` is in class bit `c` when bit `c` is set
/// in both `lo[b & 15]` and `hi[b >> 4]`. Up to eight classes per pair, each a product of a set of
/// high nibbles and a set of low nibbles. On NEON it is two `vqtbl1q_u8` per 16 bytes, as in
/// simdjson's `json_character_block::classify`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl) struct ClassTable {
    pub(in crate::code_rule_dsl) lo: [u8; 16],
    pub(in crate::code_rule_dsl) hi: [u8; 16],
}

impl ClassTable {
    /// Whether byte `b` is in any class of `bits`.
    pub(in crate::code_rule_dsl) fn member(&self, b: u8, bits: u8) -> bool {
        self.lo[usize::from(b & 15)] & self.hi[usize::from(b >> 4)] & bits != 0
    }
}

/// An instruction. Mask operands unless said otherwise.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl) enum Op {
    /// The first byte of every file in the stream.
    Starts,
    /// The same word in every block, e.g. simdjson's `ODD_BITS`.
    Const(u64),
    /// The bytes in any class of `bits` of table `table`.
    Class { table: u16, bits: u8 },
    Not(Value),
    And(Value, Value),
    Or(Value, Value),
    Xor(Value, Value),
    /// `a & !b`.
    AndNot(Value, Value),
    /// `(c & a) | (!c & b)`.
    Select(Value, Value, Value),
    /// The mask moved `k` bytes later: bit `j` is the operand's bit `j - k`. Carry: a halo, the
    /// previous block's word.
    Prev(Value, u8),
    /// The mask moved `k` bytes earlier: bit `j` is the operand's bit `j + k`, zero past the
    /// stream's end. Carry: a halo, the next block's word.
    Next(Value, u8),
    /// The stream as one integer, low byte first: `a + b`. Carry: 1 bit, forward.
    Add(Value, Value),
    /// The stream as one integer: `a - b`. Carry: a 1-bit borrow, forward.
    Sub(Value, Value),
    /// The stream as one integer, high byte first: `a + b` with carries running to earlier bytes,
    /// for `back_fill`. Carry: 1 bit, backward.
    AddBack(Value, Value),
    /// Inclusive prefix XOR (simdjson's `prefix_xor`). Carry: 1 bit of parity.
    Toggle(Value),
    /// A count mod `n` per byte: `s = ((reset ? 0 : s') + up - down) mod n`, where `s'` is the
    /// previous byte's. `ceil(log2 n)` planes. Carry: the count, a group element.
    ScanMod { n: u8, up: Value, down: Value, reset: Value },
    /// A count clamped to `0..=max + 1` per byte: `s = clamp((reset ? 0 : s') + up - down)`, where
    /// `max + 1` means "over". `ceil(log2(max + 2))` planes. Carry: the count; summaries are
    /// clamp maps.
    ScanClamp { max: u8, up: Value, down: Value, reset: Value },
    /// Plane `p` of a planes value, as a mask.
    Plane(Value, u8),
    /// The bytes whose planes value equals `c`.
    PlaneEq(Value, u8),
    /// The running total of the operand's set bits, through the end of each block: the sizes the
    /// extract pass reads (`10` §4.1). Carry: the total.
    Count(Value),
    /// The start of a body run `n` times; with `settle`, it stops after a round that changed no
    /// fed-back value. Defines no value.
    Rounds { n: u8, settle: bool },
    /// A value fed back across rounds: the operand in the first round, and in later rounds what
    /// the matching `End` feeds it. Only directly after `Rounds`.
    Feed(Value),
    /// The end of the innermost body: each `(feed, value)` gives a feed its next round's value.
    /// Values defined in the body are, after it, the last round's. Defines no value.
    End(Vec<(Value, Value)>),
}

impl Op {
    /// The operands, in order.
    pub(in crate::code_rule_dsl) fn operands(&self) -> Vec<Value> {
        match self {
            Op::Starts | Op::Const(_) | Op::Class { .. } | Op::Rounds { .. } => vec![],
            Op::Not(a)
            | Op::Prev(a, _)
            | Op::Next(a, _)
            | Op::Toggle(a)
            | Op::Plane(a, _)
            | Op::PlaneEq(a, _)
            | Op::Count(a)
            | Op::Feed(a) => vec![*a],
            Op::And(a, b)
            | Op::Or(a, b)
            | Op::Xor(a, b)
            | Op::AndNot(a, b)
            | Op::Add(a, b)
            | Op::Sub(a, b)
            | Op::AddBack(a, b) => vec![*a, *b],
            Op::Select(c, a, b) => vec![*c, *a, *b],
            Op::ScanMod { up, down, reset, .. } | Op::ScanClamp { up, down, reset, .. } => {
                vec![*up, *down, *reset]
            }
            Op::End(feeds) => feeds.iter().flat_map(|&(f, v)| [f, v]).collect(),
        }
    }

    /// Whether the operation carries from the next block instead of the previous one.
    pub(in crate::code_rule_dsl) fn backward(&self) -> bool {
        matches!(self, Op::Next(..) | Op::AddBack(..))
    }
}

/// The number of planes a count mod `n` needs: `ceil(log2 n)`.
pub(in crate::code_rule_dsl) fn mod_planes(n: u8) -> u8 {
    (8 - (n - 1).leading_zeros()) as u8
}

/// The number of planes a clamped count up to `max` needs, to hold `0..=max + 1`:
/// `ceil(log2(max + 2))`.
pub(in crate::code_rule_dsl) fn clamp_planes(max: u8) -> u8 {
    (16 - (u16::from(max) + 1).leading_zeros()) as u8
}

/// A verified stream program. Every constructor runs the verifier, so no other kind exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct Program {
    tables: Vec<ClassTable>,
    insts: Vec<Op>,
    types: Vec<Option<Ty>>,
    outputs: Vec<(String, Value)>,
}

impl Program {
    /// Build a program through the verifier.
    pub(in crate::code_rule_dsl) fn new(
        tables: Vec<ClassTable>,
        insts: Vec<Op>,
        outputs: Vec<(String, Value)>,
    ) -> Result<Self, Refusal> {
        let types = verify::verify(&tables, &insts, &outputs)?;
        Ok(Self { tables, insts, types, outputs })
    }

    pub(in crate::code_rule_dsl) fn tables(&self) -> &[ClassTable] {
        &self.tables
    }

    pub(in crate::code_rule_dsl) fn insts(&self) -> &[Op] {
        &self.insts
    }

    /// The type of value `v`; `None` for `Rounds` and `End`, which define none.
    pub(in crate::code_rule_dsl) fn ty(&self, v: Value) -> Option<Ty> {
        self.types[v as usize]
    }

    pub(in crate::code_rule_dsl) fn outputs(&self) -> &[(String, Value)] {
        &self.outputs
    }

    /// The content digest: SHA-256 of the printed body, so equal programs have equal digests
    /// and the text form round-trips it.
    pub(in crate::code_rule_dsl) fn digest(&self) -> [u8; 32] {
        Sha256::digest(super::text::print_body(self).as_bytes()).into()
    }
}
