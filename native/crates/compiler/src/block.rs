//! Block programs: the transducer's side program over source bytes
//! (`docs/parser-generators/12-what-is-wanted.md` §3), held in `Program::block_programs` as value
//! programs are held in `Program::value_programs`.
//!
//! A block program is a straight-line list of operations over 64-bit masks: bit `j` of a block's
//! word is byte `j` of the block's 64 source bytes. Operation `i` defines value `%i`. Every
//! executor lowers each operation to straight bit operations:
//!
//! - the CPU interpreter here: one dispatch per operation over the blocks, each block a NEON
//!   compare and a bitmask (a SWAR form off aarch64);
//! - the native IR (`native_ir::BlockNative`), which Cranelift (`native_cpu_block`) emits as one
//!   loop over blocks around a straight-line body, and Metal (`metal_block_program`) emits as a
//!   straight-line kernel;
//! - the Metal interpreter (`metal_block_program`), the set program's shape: every operation in
//!   order, each lane building its own mask word of 16, 32 or 64 bytes by SWAR on `uint4`s.
//!
//! Operations are added one at a time, each through every executor before the next.
//!
//! The block size is the executor's, not the program's: a program means one bit per source
//! byte. The CPU works in 64-byte words; Metal gives each lane 16, 32 or 64 bytes
//! (`metal_block_program::LaneWidth`).

use crate::Error;

/// An operation. Operands are earlier values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlockOp {
    /// The bytes equal to the byte: simdjson's `in.eq(c)`.
    Eq(u8),
    /// The bytes in any class of `bits` of table `table` (`ClassTable`).
    Class { table: u16, bits: u8 },
    /// The bytes whose byte before is in table `before`'s classes and which are themselves in
    /// table `table`'s, for a shared class of `bits`: simdjson's `check_special_cases`
    /// (`byte_1_high & byte_1_low` at the byte before, `byte_2_high` at the byte), one vector AND
    /// before the bitmask. The byte before the stream is 0x00. No carry: every executor reads
    /// the byte before from the source.
    Pair { before: u16, table: u16, bits: u8 },
    /// The same 64-bit pattern over every 64 bytes of the stream, e.g. simdjson's `ODD_BITS`.
    Const(u64),
    Not(u16),
    And(u16, u16),
    Or(u16, u16),
    Xor(u16, u16),
    /// `a & !b`.
    AndNot(u16, u16),
    /// Each byte's bit from `k` bytes before (`1..=63`), across blocks: simdjson's `follows`
    /// (`a << k` with the previous word's top `k` bits). Carry: a halo, the 64 bits before.
    Prev(u16, u8),
    /// Each byte's bit is the XOR of `a`'s bits up to and including it, across the stream:
    /// simdjson's `prefix_xor` (the inside-string mask). Carry: parity.
    Toggle(u16),
    /// The bytes just after a run of `a`'s bits of odd length: simdjson's escaped characters
    /// (`a` the backslashes), by its add-with-carry. Carry: borrow (generate and propagate).
    Escaped(u16),
    /// Bitlex's `fill(s, e)`: each byte from a byte of `s` up to, not including, the next byte of
    /// `e`; a byte in both starts a new fill. One add per word (`carry::fill`). Carry: whether a
    /// fill is open into the next block, a borrow like `escaped`'s.
    Fill(u16, u16),
    /// Bitlex's `back_fill(s, e)`, `fill`'s mirror: each byte from a byte of `s` back to, not
    /// including, the byte of `e` before it: `b[i] = s[i] | (b[i + 1] & !e[i])`. A fact carried
    /// back to the start of what it belongs to, a look ahead past any number of bytes. One add per
    /// word on the bits reversed (`carry::back_fill`). Carry: whether a fill comes back into the
    /// block from the next, so it runs over the blocks last to first: an executor runs it in a
    /// pass of its own after the values it reads (`stages`).
    BackFill(u16, u16),
    /// The bytes inside the stream at a delay `d`: bit `i` is set when `d <= i < len + d`. A
    /// program with a delay (`BlockProgram::delay`) runs past the end; the compiler cleans `~`
    /// with this, so no value has bits outside the stream. No carry: each executor knows the
    /// length and the position, so `within` at a delay is never a `prev` of `within`.
    Within(u8),
    /// The bytes at stream positions `i` with `i % k == r` (`k` in 1..=63, `r < k`): a residue
    /// of the position, for a period that does not divide 64 (`Const` holds the others). No
    /// carry: each executor knows the position, as for `within`.
    Phase(u8, u8),
    /// The bytes whose stream position has bit `j` set (`j` in 0..=63): one plane of the
    /// position as a number, for numbers kept as planes (`bitlex_compile`'s `number`). No carry:
    /// each executor knows the position, as for `phase`. Below bit 6 the plane is a pattern of
    /// the word; from bit 6 on, all of the word or none.
    Position(u8),
    /// A carry of a fixed point (`BlockProgram::rounds`): in a block's first round, `init`'s
    /// word; in each round after, its next's word in the round before.
    Carry(u16),
    /// A row program's (`BlockProgram::rows`): the rows whose column `col` compares by `op` with
    /// `rhs`, each value read as a signed `width`-bit integer (16, 32 or 64: a column held
    /// narrow compares its lanes as they are). Bit `j` of a word is row `j` of its 64.
    Cmp { col: u16, op: CmpOp, rhs: Rhs, width: u8 },
    /// A row program's: the rows whose column `col` holds a value `v` whose bit is set in set
    /// `set` (bit `v % 64` of word `v / 64`; a value outside the set's words has none). A unary
    /// relation's members, or a text test's verdicts by symbol, read as a mask.
    Bit { set: u16, col: u16 },
}

/// How a row comparison compares (`BlockOp::Cmp`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
}

impl CmpOp {
    const ALL: [(CmpOp, &'static str); 4] = [(CmpOp::Eq, "eq"), (CmpOp::Ne, "ne"), (CmpOp::Lt, "lt"), (CmpOp::Le, "le")];

    fn name(self) -> &'static str {
        CmpOp::ALL.iter().find(|p| p.0 == self).expect("every comparison is named").1
    }

    /// Whether `a op b`.
    pub fn holds(self, a: i64, b: i64) -> bool {
        match self {
            CmpOp::Eq => a == b,
            CmpOp::Ne => a != b,
            CmpOp::Lt => a < b,
            CmpOp::Le => a <= b,
        }
    }
}

/// A row comparison's right side: another column of the row, or a parameter, one value for
/// every row of a run (an outer binding, a constant).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Rhs {
    Col(u16),
    Param(u16),
}

/// What a row program reads (`BlockProgram::rows`): its columns, its parameters and its sets,
/// by count. Each run gives them (`RowBatch`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RowShape {
    pub columns: u16,
    pub params: u16,
    pub sets: u16,
}

/// A row program's input for one run: its columns (each `len` values), its parameters and its
/// sets (`RowShape`).
#[derive(Clone, Copy)]
pub struct RowBatch<'a> {
    pub columns: &'a [&'a [i64]],
    pub params: &'a [i64],
    pub sets: &'a [&'a [u64]],
    pub len: usize,
}

/// `v` read as a signed `width`-bit integer.
pub fn lane(v: i64, width: u8) -> i64 {
    match width {
        16 => i64::from(v as i16),
        32 => i64::from(v as i32),
        _ => v,
    }
}

/// The bound on a block's rounds: a strictly causal group is right on one more bit each round,
/// so 64 rounds make a block's 64 bits right and one more confirms it.
pub const MAX_ROUNDS: usize = 65;

/// The carried operations' word functions, shared by the CPU executors' references and the
/// interpreter: each takes the block's word and the carry in, and gives the result and the carry
/// out.
pub mod carry {
    /// simdjson's even bits.
    pub const EVEN: u64 = 0x5555_5555_5555_5555;

    /// `prev`: the word shifted up `k` bits, the previous word's top `k` bits below.
    pub fn prev(word: u64, previous: u64, k: u8) -> u64 {
        (word << k) | (previous >> (64 - u32::from(k)))
    }

    /// `toggle`: the word's prefix XOR, inverted when `carry` (all ones or zero) is set; the
    /// carry out is the last bit, spread.
    pub fn toggle(word: u64, carry: u64) -> (u64, u64) {
        let y = prefix_xor(word) ^ carry;
        (y, ((y as i64) >> 63) as u64)
    }

    /// simdjson's `prefix_xor` on aarch64: a carry-less multiply by all ones (`vmull_p64`).
    #[cfg(all(target_arch = "aarch64", target_feature = "aes"))]
    pub fn prefix_xor(word: u64) -> u64 {
        use core::arch::aarch64::vmull_p64;
        // SAFETY: `aes` (PMULL) is enabled for this target.
        unsafe { vmull_p64(word, u64::MAX) as u64 }
    }

    /// Elsewhere, six shift-XOR rounds.
    #[cfg(not(all(target_arch = "aarch64", target_feature = "aes")))]
    pub fn prefix_xor(mut word: u64) -> u64 {
        for k in [1, 2, 4, 8, 16, 32] {
            word ^= word << k;
        }
        word
    }

    /// `fill`: `f[i] = s[i] | (f[i - 1] & !e[i])`, with `f[-1] = cin` (0 or 1). With `a = s` and
    /// `b = !e | s`, the add `a + b + cin` generates a carry at each start and propagates it
    /// over each byte that is neither a start nor an end, so the carry into bit `i` is
    /// `f[i - 1]`. The carry out is the add's overflow, `f[63]`.
    pub fn fill(s: u64, e: u64, cin: u64) -> (u64, u64) {
        let (a, b) = (s, !e | s);
        let into = a.wrapping_add(b).wrapping_add(cin) ^ a ^ b;
        let f = s | (!e & into);
        (f, f >> 63)
    }

    /// `back_fill`: `fill` on the bits reversed, `b[i] = s[i] | (b[i + 1] & !e[i])` with
    /// `b[64] = cin` (0 or 1, the next block's bit 0); the carry out is `b[0]`.
    pub fn back_fill(s: u64, e: u64, cin: u64) -> (u64, u64) {
        let (f, _) = fill(s.reverse_bits(), e.reverse_bits(), cin);
        let b = f.reverse_bits();
        (b, b & 1)
    }

    /// `escaped`: simdjson's branch-free escape finder (`find_escaped_branchless`) with the
    /// carry in (`cin`, 0 or 1: the block's first byte is escaped). Every byte an odd-offset
    /// bit of a run escapes, escaped bits of the run included; the carry out is the add's
    /// overflow.
    pub fn escaped(run: u64, cin: u64) -> (u64, u64) {
        let b = run & !cin;
        let follows = (b << 1) | cin;
        let odd_starts = b & !EVEN & !follows;
        let (sequences, overflow) = odd_starts.overflowing_add(b);
        ((EVEN ^ (sequences << 1)) & follows, u64::from(overflow))
    }
}

impl BlockOp {
    /// The operation's text mnemonic and operands (`rir`).
    pub fn text(&self) -> String {
        let two = |name: &str, a: &u16, b: &u16| format!("{name} %{a}, %{b}");
        match self {
            BlockOp::Eq(byte) => format!("eq {byte:#04x}"),
            BlockOp::Class { table, bits } => format!("class t{table} {bits:#04x}"),
            BlockOp::Pair { before, table, bits } => format!("pair t{before} t{table} {bits:#04x}"),
            BlockOp::Const(c) => format!("const {c:#018x}"),
            BlockOp::Not(a) => format!("not %{a}"),
            BlockOp::And(a, b) => two("and", a, b),
            BlockOp::Or(a, b) => two("or", a, b),
            BlockOp::Xor(a, b) => two("xor", a, b),
            BlockOp::AndNot(a, b) => two("andnot", a, b),
            BlockOp::Prev(a, k) => format!("prev %{a}, {k}"),
            BlockOp::Toggle(a) => format!("toggle %{a}"),
            BlockOp::Escaped(a) => format!("escaped %{a}"),
            BlockOp::Fill(a, b) => two("fill", a, b),
            BlockOp::BackFill(a, b) => two("back_fill", a, b),
            BlockOp::Within(0) => "within".to_owned(),
            BlockOp::Within(d) => format!("within {d}"),
            BlockOp::Phase(k, r) => format!("phase {k} {r}"),
            BlockOp::Position(j) => format!("position {j}"),
            BlockOp::Carry(a) => format!("carry %{a}"),
            BlockOp::Cmp { col, op, rhs, width } => {
                let rhs = match rhs {
                    Rhs::Col(c) => format!("c{c}"),
                    Rhs::Param(p) => format!("p{p}"),
                };
                format!("cmp {} c{col} {rhs} {width}", op.name())
            }
            BlockOp::Bit { set, col } => format!("bit s{set} c{col}"),
        }
    }

    /// The values the operation reads.
    pub fn operands(&self) -> Vec<u16> {
        match *self {
            BlockOp::Eq(_) | BlockOp::Class { .. } | BlockOp::Pair { .. } | BlockOp::Const(_) | BlockOp::Within(_) | BlockOp::Phase(..) | BlockOp::Position(_) | BlockOp::Cmp { .. } | BlockOp::Bit { .. } => vec![],
            BlockOp::Not(a) | BlockOp::Prev(a, _) | BlockOp::Toggle(a) | BlockOp::Escaped(a) | BlockOp::Carry(a) => vec![a],
            BlockOp::And(a, b) | BlockOp::Or(a, b) | BlockOp::Xor(a, b) | BlockOp::AndNot(a, b) | BlockOp::Fill(a, b) | BlockOp::BackFill(a, b) => {
                vec![a, b]
            }
        }
    }

    /// The operation with each operand `v` replaced by `f(v)`.
    pub fn renumbered(&self, f: impl Fn(u16) -> u16) -> Self {
        match *self {
            BlockOp::Eq(_) | BlockOp::Class { .. } | BlockOp::Pair { .. } | BlockOp::Const(_) | BlockOp::Within(_) | BlockOp::Phase(..) | BlockOp::Position(_) | BlockOp::Cmp { .. } | BlockOp::Bit { .. } => *self,
            BlockOp::Not(a) => BlockOp::Not(f(a)),
            BlockOp::Prev(a, k) => BlockOp::Prev(f(a), k),
            BlockOp::Toggle(a) => BlockOp::Toggle(f(a)),
            BlockOp::Escaped(a) => BlockOp::Escaped(f(a)),
            BlockOp::Carry(a) => BlockOp::Carry(f(a)),
            BlockOp::Fill(a, b) => BlockOp::Fill(f(a), f(b)),
            BlockOp::BackFill(a, b) => BlockOp::BackFill(f(a), f(b)),
            BlockOp::And(a, b) => BlockOp::And(f(a), f(b)),
            BlockOp::Or(a, b) => BlockOp::Or(f(a), f(b)),
            BlockOp::Xor(a, b) => BlockOp::Xor(f(a), f(b)),
            BlockOp::AndNot(a, b) => BlockOp::AndNot(f(a), f(b)),
        }
    }

    /// Parse `text`'s output: a mnemonic and its operands, as words.
    pub fn parse(words: &[String]) -> Result<Self, String> {
        let byte = |word: &str| -> Result<u8, String> {
            word.strip_prefix("0x")
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                .ok_or_else(|| format!("a bad byte `{word}`"))
        };
        let value = |word: &str| -> Result<u16, String> {
            word.trim_end_matches(',')
                .strip_prefix('%')
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| format!("a bad value `{word}`"))
        };
        let table = |t: &str| -> Result<u16, String> {
            t.strip_prefix('t').and_then(|t| t.parse().ok()).ok_or_else(|| format!("a bad table `{t}`"))
        };
        let words: Vec<&str> = words.iter().map(String::as_str).collect();
        let index = |word: &str, prefix: char| -> Result<u16, String> {
            word.strip_prefix(prefix).and_then(|v| v.parse().ok()).ok_or_else(|| format!("a bad operand `{word}`"))
        };
        match words.as_slice() {
            ["cmp", op, c, r, w] => {
                let op = CmpOp::ALL.iter().find(|p| p.1 == *op).ok_or_else(|| format!("a bad comparison `{op}`"))?.0;
                let rhs = if r.starts_with('p') { Rhs::Param(index(r, 'p')?) } else { Rhs::Col(index(r, 'c')?) };
                let width = w.parse().map_err(|_| format!("a bad width `{w}`"))?;
                Ok(BlockOp::Cmp { col: index(c, 'c')?, op, rhs, width })
            }
            ["bit", set, c] => Ok(BlockOp::Bit { set: index(set, 's')?, col: index(c, 'c')? }),
            ["pair", a, t, b] => Ok(BlockOp::Pair { before: table(a)?, table: table(t)?, bits: byte(b)? }),
            ["eq", b] => Ok(BlockOp::Eq(byte(b)?)),
            ["class", t, b] => Ok(BlockOp::Class {
                table: t.strip_prefix('t').and_then(|t| t.parse().ok()).ok_or_else(|| format!("a bad table `{t}`"))?,
                bits: byte(b)?,
            }),
            ["const", c] => c
                .strip_prefix("0x")
                .and_then(|hex| u64::from_str_radix(hex, 16).ok())
                .map(BlockOp::Const)
                .ok_or_else(|| format!("a bad constant `{c}`")),
            ["not", a] => Ok(BlockOp::Not(value(a)?)),
            ["and", a, b] => Ok(BlockOp::And(value(a)?, value(b)?)),
            ["or", a, b] => Ok(BlockOp::Or(value(a)?, value(b)?)),
            ["xor", a, b] => Ok(BlockOp::Xor(value(a)?, value(b)?)),
            ["andnot", a, b] => Ok(BlockOp::AndNot(value(a)?, value(b)?)),
            ["prev", a, k] => Ok(BlockOp::Prev(value(a)?, k.parse().map_err(|_| format!("a bad distance `{k}`"))?)),
            ["toggle", a] => Ok(BlockOp::Toggle(value(a)?)),
            ["escaped", a] => Ok(BlockOp::Escaped(value(a)?)),
            ["fill", a, b] => Ok(BlockOp::Fill(value(a)?, value(b)?)),
            ["back_fill", a, b] => Ok(BlockOp::BackFill(value(a)?, value(b)?)),
            ["carry", a] => Ok(BlockOp::Carry(value(a)?)),
            ["within"] => Ok(BlockOp::Within(0)),
            ["within", d] => match d.parse::<u8>() {
                Ok(d) if (1..=63).contains(&d) => Ok(BlockOp::Within(d)),
                _ => Err(format!("a bad within delay `{d}`")),
            },
            ["phase", k, r] => match (k.parse::<u8>(), r.parse::<u8>()) {
                (Ok(k), Ok(r)) if (1..=63).contains(&k) && r < k => Ok(BlockOp::Phase(k, r)),
                _ => Err(format!("a bad phase `{k} {r}`")),
            },
            ["position", j] => match j.parse::<u8>() {
                Ok(j) if j < 64 => Ok(BlockOp::Position(j)),
                _ => Err(format!("a bad position bit `{j}`")),
            },
            _ => Err(format!("an unknown block operation `{}`", words.join(" "))),
        }
    }

    fn canonical(&self, out: &mut Vec<u8>) {
        let two = |out: &mut Vec<u8>, tag: u8, a: u16, b: u16| {
            out.push(tag);
            out.extend_from_slice(&a.to_le_bytes());
            out.extend_from_slice(&b.to_le_bytes());
        };
        match *self {
            BlockOp::Eq(byte) => out.extend_from_slice(&[0, byte]),
            BlockOp::Class { table, bits } => {
                out.push(7);
                out.extend_from_slice(&table.to_le_bytes());
                out.push(bits);
            }
            BlockOp::Pair { before, table, bits } => {
                out.push(11);
                out.extend_from_slice(&before.to_le_bytes());
                out.extend_from_slice(&table.to_le_bytes());
                out.push(bits);
            }
            BlockOp::Const(c) => {
                out.push(1);
                out.extend_from_slice(&c.to_le_bytes());
            }
            BlockOp::Not(a) => two(out, 2, a, a),
            BlockOp::And(a, b) => two(out, 3, a, b),
            BlockOp::Or(a, b) => two(out, 4, a, b),
            BlockOp::Xor(a, b) => two(out, 5, a, b),
            BlockOp::AndNot(a, b) => two(out, 6, a, b),
            BlockOp::Prev(a, k) => two(out, 8, a, u16::from(k)),
            BlockOp::Toggle(a) => two(out, 9, a, a),
            BlockOp::Escaped(a) => two(out, 10, a, a),
            BlockOp::Fill(a, b) => two(out, 12, a, b),
            // `within` alone keeps its byte; at a delay, a new one and the delay.
            BlockOp::Within(0) => out.push(13),
            BlockOp::Within(d) => out.extend([14, d]),
            BlockOp::Phase(k, r) => out.extend([15, k, r]),
            BlockOp::Carry(a) => two(out, 16, a, a),
            BlockOp::BackFill(a, b) => two(out, 17, a, b),
            BlockOp::Position(j) => out.extend([18, j]),
            BlockOp::Cmp { col, op, rhs, width } => {
                out.push(19);
                out.extend_from_slice(&col.to_le_bytes());
                out.push(CmpOp::ALL.iter().position(|p| p.0 == op).expect("named") as u8);
                let (tag, at) = match rhs {
                    Rhs::Col(c) => (0, c),
                    Rhs::Param(p) => (1, p),
                };
                out.push(tag);
                out.extend_from_slice(&at.to_le_bytes());
                out.push(width);
            }
            BlockOp::Bit { set, col } => {
                out.push(20);
                out.extend_from_slice(&set.to_le_bytes());
                out.extend_from_slice(&col.to_le_bytes());
            }
        }
    }
}

/// A nibble table pair for `class` (`03-bitlex.md` §3.1, simdjson's `classify`): byte `b` is in
/// class bit `c` when bit `c` is set in both `lo[b & 15]` and `hi[b >> 4]`. Up to eight classes
/// per pair, each a product of a set of high nibbles and a set of low nibbles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ClassTable {
    pub lo: [u8; 16],
    pub hi: [u8; 16],
}

impl ClassTable {
    /// Whether byte `b` is in any class of `bits`.
    pub fn member(&self, b: u8, bits: u8) -> bool {
        self.lo[usize::from(b & 15)] & self.hi[usize::from(b >> 4)] & bits != 0
    }
}

/// A block program: its class tables, its operations, the values it outputs, and the outputs
/// whose positions the extract pass writes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct BlockProgram {
    pub tables: Vec<ClassTable>,
    pub ops: Vec<BlockOp>,
    pub outputs: Vec<u16>,
    /// Values, each also an output, whose set bits the extract pass writes as byte positions
    /// (`docs/parser-generators/10-bit-programs.md` §4.1): simdjson's structural
    /// indexes from its structural mask.
    pub extracts: Vec<u16>,
    /// Values, each also an output, that each executor tests over the whole stream: a total per
    /// file (`BlockRun::totals`), 1 when the output has any bit and 0 when it has none. Tier 2
    /// reads them (`EMPTY`): the fallback's masks are totals, and a file with any bit goes to
    /// tree-sitter. Only whether there is a bit is wanted, so no executor counts them.
    pub totals: Vec<u16>,
    /// Values, each also an output, that tier 2 reads as masks, a word of 64 bytes at a time:
    /// the last byte of each token whose end its row does not know (`SPAN m=1`), found as the
    /// next set bit. Nothing flattens them.
    pub masks: Vec<u16>,
    /// How many bytes late the outputs are (0..=63): each executor runs over the stream and
    /// `delay` bytes past it, and an output's bit `i + delay` is byte `i`'s. The compiler delays
    /// values so that `next` reads earlier bits (`bitlex_compile`).
    pub delay: u8,
    /// Each output's own delay (`0..=delay`), in `outputs`' order; an output without one is
    /// `delay` late. An output is never brought to the program's delay by a `prev`: the host
    /// shifts each by its own (decision 0069).
    pub output_delays: Vec<u8>,
    /// The fixed points (Bitlex's `rounds until settled`): each group's carries, each with the
    /// value that is its next. A block's rounds run its group's members (the values on a way
    /// from a carry to a next) until no carry changes; every such way passes a `prev`, so they
    /// settle within `MAX_ROUNDS` (docs/parser-generators/03-bitlex.md §2.7).
    pub rounds: Vec<Vec<(u16, u16)>>,
    /// A row program's inputs: its stream is rows, not bytes, bit `j` of word `i` row `64 i + j`
    /// of a run's columns (decision 0095 §2). Its values come from `Cmp` and `Bit` and the mask
    /// operations; it reads no bytes, has no delay, carries nothing from word to word and
    /// writes no tier-2 masks. Its extract pass writes row numbers: a filter's survivors.
    pub rows: Option<RowShape>,
}

/// A rounds group's layout: its members in order (the values on a way from one of its carries
/// to one of its nexts, carries included), and the last of them, where the group runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub members: Vec<usize>,
    pub carries: Vec<(usize, usize)>,
}

/// Each value's group, of `ops` under `rounds`: the values on a way from a group's carry to
/// one of its nexts. Refused: a carry in no group or two, a next before its carry, groups that
/// share a value, a way from a carry to a next that passes no `prev`, or a group reading a value
/// that waits on a later group.
pub fn groups(ops: &[BlockOp], rounds: &[Vec<(u16, u16)>]) -> Result<Vec<Group>, String> {
    let n = ops.len();
    let users = {
        let mut u: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (at, op) in ops.iter().enumerate() {
            for o in op.operands() {
                u[usize::from(o)].push(at);
            }
        }
        u
    };
    let mut owner: Vec<Option<usize>> = vec![None; n];
    let mut out = Vec::new();
    let carries = ops.iter().filter(|op| matches!(op, BlockOp::Carry(_))).count();
    if carries != rounds.iter().map(Vec::len).sum::<usize>() {
        return Err("a carry in no rounds group, or in two".into());
    }
    for (g, pairs) in rounds.iter().enumerate() {
        let pairs: Vec<(usize, usize)> = pairs.iter().map(|&(c, x)| (usize::from(c), usize::from(x))).collect();
        if pairs.is_empty() || pairs.iter().any(|&(c, x)| c >= n || x >= n || x <= c || !matches!(ops[c], BlockOp::Carry(_))) {
            return Err(format!("rounds group {g}: a pair that is not a carry and a later value"));
        }
        // Forward from the carries, backward from the nexts.
        let mut ahead = vec![false; n];
        let mut todo: Vec<usize> = pairs.iter().map(|p| p.0).collect();
        while let Some(x) = todo.pop() {
            if !std::mem::replace(&mut ahead[x], true) {
                todo.extend(users[x].iter().copied());
            }
        }
        let mut behind = vec![false; n];
        let mut todo: Vec<usize> = pairs.iter().map(|p| p.1).collect();
        while let Some(x) = todo.pop() {
            if !std::mem::replace(&mut behind[x], true) && !matches!(ops[x], BlockOp::Carry(_)) {
                todo.extend(ops[x].operands().into_iter().map(usize::from));
            }
        }
        let members: Vec<usize> = (0..n).filter(|&x| ahead[x] && behind[x]).collect();
        // A fixed point settles block by block, first to last: a `back_fill` carries the other way.
        if let Some(&m) = members.iter().find(|&&m| matches!(ops[m], BlockOp::BackFill(..))) {
            return Err(format!("rounds group {g}: `back_fill` %{m} in a fixed point"));
        }
        for &m in &members {
            if owner[m].replace(g).is_some() {
                return Err(format!("rounds group {g} shares value %{m} with another"));
            }
        }
        // Strict: from the carries, without passing a `prev`, no next.
        let mut loose = vec![false; n];
        let mut todo: Vec<usize> = pairs.iter().map(|p| p.0).collect();
        while let Some(x) = todo.pop() {
            if !std::mem::replace(&mut loose[x], true) {
                todo.extend(users[x].iter().copied().filter(|&u| !matches!(ops[u], BlockOp::Prev(..))));
            }
        }
        if let Some(&(c, x)) = pairs.iter().find(|&&(_, x)| loose[x]) {
            return Err(format!("rounds group {g}: carry %{c} reaches its next %{x} with no `prev` on the way"));
        }
        out.push(Group { members, carries: pairs });
    }
    // A group's members read, outside it, only values of earlier groups or none.
    let mut waits: Vec<Vec<bool>> = vec![vec![false; rounds.len()]; n];
    for x in 0..n {
        let mut w = vec![false; rounds.len()];
        if let Some(g) = owner[x] {
            w[g] = true;
        }
        for o in ops[x].operands() {
            for (g, &b) in waits[usize::from(o)].iter().enumerate() {
                w[g] |= b;
            }
        }
        waits[x] = w;
    }
    for (g, group) in out.iter().enumerate() {
        let last = *group.members.last().expect("a carry at least");
        for &m in &group.members {
            for o in ops[m].operands().into_iter().map(usize::from).filter(|&o| owner[o] != Some(g)) {
                if let Some(h) = (0..rounds.len()).find(|&h| waits[o][h] && *out[h].members.last().expect("a member") > last) {
                    return Err(format!("rounds group {g} reads %{o}, which waits on the later group {h}"));
                }
            }
        }
    }
    Ok(out)
}

/// A file of a batch: its bytes, and its program's index in the batch's programs. Every executor
/// takes a batch in this shape (decision 0066).
#[derive(Clone, Copy)]
pub struct BatchFile<'a> {
    pub bytes: &'a [u8],
    pub program: usize,
}

/// A run's results: each output's words, each extract's positions (`u32` byte offsets, in
/// order), and each total: 1 when its output has a bit, else 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockRun {
    pub outputs: Vec<Vec<u64>>,
    pub positions: Vec<Vec<u32>>,
    pub totals: Vec<u64>,
}

impl BlockProgram {
    /// The verifier's rules for a block program: at least one operation and one output, every
    /// operand an earlier value, and outputs that name values.
    pub fn verify(&self) -> Result<(), Error> {
        let fits = self.ops.len() <= usize::from(u16::MAX);
        let outputs = !self.outputs.is_empty() && self.outputs.iter().all(|&v| usize::from(v) < self.ops.len());
        let earlier = self
            .ops
            .iter()
            .enumerate()
            .all(|(at, op)| op.operands().iter().all(|&v| usize::from(v) < at));
        // A row program reads columns and no bytes; a byte program reads no columns.
        let streams = match self.rows {
            Some(shape) => {
                let col = |c: u16| c < shape.columns;
                let reads = self.ops.iter().all(|op| match *op {
                    BlockOp::Cmp { col: c, rhs, width, .. } => {
                        col(c) && [16, 32, 64].contains(&width) && match rhs {
                            Rhs::Col(d) => col(d),
                            Rhs::Param(p) => p < shape.params,
                        }
                    }
                    BlockOp::Bit { set, col: c } => set < shape.sets && col(c),
                    BlockOp::Const(_) | BlockOp::Not(_) | BlockOp::And(..) | BlockOp::Or(..) | BlockOp::Xor(..) | BlockOp::AndNot(..) => true,
                    _ => false,
                });
                reads && self.delay == 0 && self.output_delays.is_empty() && self.rounds.is_empty() && self.masks.is_empty()
            }
            None => !self.ops.iter().any(|op| matches!(op, BlockOp::Cmp { .. } | BlockOp::Bit { .. })),
        };
        let tables = streams && self.ops.iter().all(|op| match *op {
            BlockOp::Class { table, bits } => usize::from(table) < self.tables.len() && bits != 0,
            BlockOp::Pair { before, table, bits } => {
                usize::from(before) < self.tables.len() && usize::from(table) < self.tables.len() && bits != 0
            }
            // Up to 63 bytes: the halo is one 64-bit word on every executor.
            BlockOp::Prev(_, k) => (1..=63).contains(&k),
            BlockOp::Phase(k, r) => (1..=63).contains(&k) && r < k,
            BlockOp::Position(j) => j < 64,
            _ => true,
        });
        // The extract pass writes positions from the undelayed outputs.
        let extracts = self.extracts.iter().chain(&self.totals).chain(&self.masks).all(|v| self.outputs.contains(v));
        let delay = self.delay <= 63
            && self.output_delays.len() <= self.outputs.len()
            && self.output_delays.iter().all(|&d| d == u8::MAX || d <= self.delay);
        if self.ops.is_empty() || !fits || !outputs || !earlier || !tables || !extracts || !delay {
            return Err(Error::InvalidTarget(0));
        }
        groups(&self.ops, &self.rounds).map_err(|_| Error::InvalidTarget(0))?;
        Ok(())
    }

    /// The bytes the program digest covers.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(self.tables.len() as u32).to_le_bytes());
        for table in &self.tables {
            out.extend_from_slice(&table.lo);
            out.extend_from_slice(&table.hi);
        }
        out.extend_from_slice(&(self.ops.len() as u32).to_le_bytes());
        for op in &self.ops {
            op.canonical(&mut out);
        }
        out.extend_from_slice(&(self.outputs.len() as u32).to_le_bytes());
        for v in &self.outputs {
            out.extend_from_slice(&v.to_le_bytes());
        }
        // Appended only when present, so programs without extracts keep their digests.
        if !self.extracts.is_empty() {
            out.extend_from_slice(&(self.extracts.len() as u32).to_le_bytes());
            for v in &self.extracts {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        // Likewise the totals, tagged.
        if !self.totals.is_empty() {
            out.extend_from_slice(b"totals");
            out.extend_from_slice(&(self.totals.len() as u32).to_le_bytes());
            for v in &self.totals {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        // Likewise the masks, tagged.
        if !self.masks.is_empty() {
            out.extend_from_slice(b"masks");
            out.extend_from_slice(&(self.masks.len() as u32).to_le_bytes());
            for v in &self.masks {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        // Likewise the delay, tagged.
        if self.delay != 0 {
            out.extend_from_slice(b"delay");
            out.push(self.delay);
        }
        // The rounds groups, tagged, when there are any.
        if !self.rounds.is_empty() {
            out.extend_from_slice(b"rounds");
            out.extend_from_slice(&(self.rounds.len() as u32).to_le_bytes());
            for g in &self.rounds {
                out.extend_from_slice(&(g.len() as u32).to_le_bytes());
                for &(c, x) in g {
                    out.extend_from_slice(&c.to_le_bytes());
                    out.extend_from_slice(&x.to_le_bytes());
                }
            }
        }
        // A row program's shape, tagged.
        if let Some(shape) = self.rows {
            out.extend_from_slice(b"rows");
            for n in [shape.columns, shape.params, shape.sets] {
                out.extend_from_slice(&n.to_le_bytes());
            }
        }
        // And the outputs' own delays, when any is not the program's.
        if (0..self.outputs.len()).any(|k| self.output_delay(k) != self.delay) {
            out.extend_from_slice(b"odelay");
            out.extend((0..self.outputs.len()).map(|k| self.output_delay(k)));
        }
        out
    }

    /// Output `k`'s delay: its own, or the program's.
    pub fn output_delay(&self, k: usize) -> u8 {
        // `u8::MAX`: none of its own (an output assembled before the program's delay was read).
        match self.output_delays.get(k) {
            Some(&d) if d != u8::MAX => d,
            _ => self.delay,
        }
    }

    /// Each extract's output index (its place in `outputs`).
    pub fn extract_outputs(&self) -> Vec<usize> {
        self.extracts
            .iter()
            .map(|v| self.outputs.iter().position(|o| o == v).expect("verified: an extract is an output"))
            .collect()
    }

    /// Each mask's output index.
    pub fn mask_outputs(&self) -> Vec<usize> {
        self.masks
            .iter()
            .map(|v| self.outputs.iter().position(|o| o == v).expect("verified: a mask is an output"))
            .collect()
    }

    /// Each total's output index.
    pub fn total_outputs(&self) -> Vec<usize> {
        self.totals
            .iter()
            .map(|v| self.outputs.iter().position(|o| o == v).expect("verified: a total is an output"))
            .collect()
    }

    /// The CPU interpreter with the extract pass: the outputs, then each extract's positions,
    /// sized by its count before it is written (`extract`).
    pub fn run(&self, bytes: &[u8]) -> BlockRun {
        self.run_files(&[bytes]).pop().expect("one file")
    }

    /// `run` over several files at once (`interpret_files`).
    pub fn run_files(&self, files: &[&[u8]]) -> Vec<BlockRun> {
        let (extracts, totals) = (self.extract_outputs(), self.total_outputs());
        self.interpret_files(files)
            .into_iter()
            .map(|outputs| {
                let totals = totals.iter().map(|&k| u64::from(outputs[k].iter().any(|&w| w != 0))).collect();
                let positions = extracts
                    .iter()
                    .map(|&k| {
                        let words = &outputs[k];
                        let count = words.iter().map(|w| w.count_ones() as usize).sum();
                        extract::positions(words, count)
                    })
                    .collect();
                BlockRun { outputs, positions, totals }
            })
            .collect()
    }

    /// The CPU interpreter: every output's words over `bytes`. Bits past the end of `bytes` are
    /// zero.
    pub fn interpret(&self, bytes: &[u8]) -> Vec<Vec<u64>> {
        self.interpret_files(&[bytes]).pop().expect("one file")
    }

    /// The CPU interpreter over a batch of files under this program: one dispatch per operation
    /// over every file's blocks (each file's stream and `delay` bytes past it, end to end). A
    /// carried operation starts each file with a zero carry, as a stream of its own; `within`
    /// and the byte before a block are the block's file's.
    pub fn interpret_files(&self, files: &[&[u8]]) -> Vec<Vec<Vec<u64>>> {
        let (values, spans) = self.all_values(files);
        spans
            .into_iter()
            .zip(files)
            .map(|(span, bytes)| {
                self.outputs
                    .iter()
                    .enumerate()
                    .map(|(k, &v)| undelay(&values[usize::from(v)][span.clone()], self.output_delay(k), bytes.len()))
                    .collect()
            })
            .collect()
    }

    /// Every operation's words over a batch of files end to end (each file's blocks, `delay`
    /// bytes past it included), as they are made: not cut at the file's end, not undelayed.
    /// With each file's span of blocks. For checks of what is claimed about a value
    /// (`token_local`).
    pub fn all_values(&self, files: &[&[u8]]) -> (Vec<Vec<u64>>, Vec<std::ops::Range<usize>>) {
        let extra = usize::from(self.delay);
        // Each block's file (its length, and its block number within it), and whether it is
        // the file's first.
        let (mut blocks, mut before) = (Vec::new(), Vec::new());
        let mut place: Vec<(usize, usize)> = Vec::new();
        let mut spans = Vec::with_capacity(files.len());
        for bytes in files {
            let own = padded_by(bytes, extra);
            spans.push(blocks.len()..blocks.len() + own.len());
            place.extend((0..own.len()).map(|at| (at, bytes.len())));
            before.extend(before_blocks(&own));
            blocks.extend(own);
        }
        let first: Vec<bool> = place.iter().map(|&(at, _)| at == 0).collect();
        // A carried pass: the carry is `zero` at each file's first block.
        fn carried<C: Copy>(xs: impl Iterator<Item = (usize, u64, u64)>, first: &[bool], zero: C, f: impl Fn(u64, u64, C) -> (u64, C)) -> Vec<u64> {
            let mut c = zero;
            xs.map(|(i, x, y)| {
                if first[i] {
                    c = zero;
                }
                let (out, next) = f(x, y, c);
                c = next;
                out
            })
            .collect()
        }
        let n = self.ops.len();
        let groups = groups(&self.ops, &self.rounds).expect("a verified program");
        let mut owner: Vec<Option<usize>> = vec![None; n];
        for (g, group) in groups.iter().enumerate() {
            for &m in &group.members {
                owner[m] = Some(g);
            }
        }
        // The groups each value waits on: its own, or one a value it reads waits on.
        let mut waits: Vec<Vec<usize>> = vec![Vec::new(); n];
        for x in 0..n {
            let mut w: Vec<usize> = owner[x].into_iter().collect();
            for o in self.ops[x].operands() {
                w.extend(waits[usize::from(o)].iter().copied());
            }
            w.sort_unstable();
            w.dedup();
            waits[x] = w;
        }
        let last: Vec<usize> = groups.iter().map(|g| *g.members.last().expect("a member")).collect();
        let mut values: Vec<Vec<u64>> = vec![Vec::new(); n];
        let mut ran = vec![false; groups.len()];
        let mut deferred: Vec<Vec<usize>> = vec![Vec::new(); groups.len()];
        // Each value in order, op by op over every block; a group's members at its last, block
        // by block (`rounds`); a value that waits on a group not run yet, after it.
        let mut order: Vec<usize> = Vec::with_capacity(n);
        for x in 0..n {
            if let Some(g) = owner[x] {
                if x == last[g] {
                    order.push(usize::MAX - g);
                    order.extend(std::mem::take(&mut deferred[g]));
                    ran[g] = true;
                }
                continue;
            }
            match waits[x].iter().copied().filter(|&g| !ran[g]).max_by_key(|&g| last[g]) {
                Some(g) => deferred[g].push(x),
                None => order.push(x),
            }
        }
        for x in order {
            if x > n {
                let g = usize::MAX - x;
                self.rounds_group(&groups[g], &first, &mut values);
                continue;
            }
            let op = &self.ops[x];
            let v = |at: u16| &values[usize::from(at)];
            let two = |a: u16, b: u16, f: fn(u64, u64) -> u64| -> Vec<u64> {
                v(a).iter().zip(v(b)).map(|(&x, &y)| f(x, y)).collect()
            };
            let one = |a: u16| v(a).iter().enumerate().map(|(i, &x)| (i, x, 0));
            let words: Vec<u64> = match *op {
                BlockOp::Within(d) => place.iter().map(|&(at, len)| within_at(at, len, d)).collect(),
                BlockOp::Phase(k, r) => place.iter().map(|&(at, _)| phase(64 * at, k, r)).collect(),
                BlockOp::Position(j) => place.iter().map(|&(at, _)| position(at, j)).collect(),
                BlockOp::Carry(_) => unreachable!("a carry is its group's, run with it"),
                BlockOp::Cmp { .. } | BlockOp::Bit { .. } => unreachable!("verified: a row program's, run by `run_rows`"),
                BlockOp::Eq(byte) => blocks.iter().map(|block| eq(block, byte)).collect(),
                BlockOp::Class { table, bits } => {
                    let table = &self.tables[usize::from(table)];
                    blocks.iter().map(|block| class(block, table, bits)).collect()
                }
                BlockOp::Pair { before: t, table, bits } => {
                    let (a, b) = (&self.tables[usize::from(t)], &self.tables[usize::from(table)]);
                    blocks.iter().zip(&before).map(|(block, prev)| pair(prev, block, a, b, bits)).collect()
                }
                BlockOp::Const(c) => vec![c; blocks.len()],
                BlockOp::Not(a) => v(a).iter().map(|&x| !x).collect(),
                BlockOp::And(a, b) => two(a, b, |x, y| x & y),
                BlockOp::Or(a, b) => two(a, b, |x, y| x | y),
                BlockOp::Xor(a, b) => two(a, b, |x, y| x ^ y),
                BlockOp::AndNot(a, b) => two(a, b, |x, y| x & !y),
                // The carried operations: one pass over the blocks, the carry in a register.
                BlockOp::Prev(a, k) => carried(one(a), &first, 0u64, |x, _, previous| (carry::prev(x, previous, k), x)),
                BlockOp::Toggle(a) => carried(one(a), &first, 0u64, |x, _, c| carry::toggle(x, c)),
                BlockOp::Escaped(a) => carried(one(a), &first, 0u64, |x, _, c| carry::escaped(x, c)),
                BlockOp::Fill(a, b) => {
                    let pairs = v(a).iter().zip(v(b)).enumerate().map(|(i, (&x, &y))| (i, x, y));
                    carried(pairs, &first, 0u64, |x, y, c| carry::fill(x, y, c))
                }
                // Last block to first, the carry zero at each file's last block.
                BlockOp::BackFill(a, b) => {
                    let (s, e) = (v(a), v(b));
                    let mut out = vec![0u64; s.len()];
                    let mut c = 0u64;
                    for i in (0..s.len()).rev() {
                        if i + 1 == s.len() || first[i + 1] {
                            c = 0;
                        }
                        (out[i], c) = carry::back_fill(s[i], e[i], c);
                    }
                    out
                }
            };
            values[x] = words;
        }
        (values, spans)
    }
}

impl BlockProgram {
    /// A rounds group over every block, block by block: from the carries' initial words, its
    /// members again until no carry changes, each carried member from the settled block before
    /// (zero at a file's first block). The values the group does not hold are made already.
    fn rounds_group(&self, group: &Group, first: &[bool], values: &mut [Vec<u64>]) {
        let blocks = first.len();
        let mut local = vec![0u64; self.ops.len()];
        let mut state = vec![0u64; group.members.len()];
        let mut outs = vec![0u64; group.members.len()];
        for &m in &group.members {
            values[m] = vec![0; blocks];
        }
        let carry_of = |c: usize| group.carries.iter().position(|p| p.0 == c).expect("a carry of the group");
        for i in 0..blocks {
            if first[i] {
                state.iter_mut().for_each(|x| *x = 0);
            }
            let mut cur: Vec<u64> = group
                .carries
                .iter()
                .map(|&(c, _)| match self.ops[c] {
                    BlockOp::Carry(init) => values[usize::from(init)][i],
                    _ => unreachable!("verified: a carry"),
                })
                .collect();
            let mut settled = false;
            for _ in 0..MAX_ROUNDS {
                for (k, &m) in group.members.iter().enumerate() {
                    let get = |o: u16| if group.members.binary_search(&usize::from(o)).is_ok() { local[usize::from(o)] } else { values[usize::from(o)][i] };
                    let (w, c) = match self.ops[m] {
                        BlockOp::Carry(_) => (cur[carry_of(m)], 0),
                        BlockOp::Not(a) => (!get(a), 0),
                        BlockOp::And(a, b) => (get(a) & get(b), 0),
                        BlockOp::Or(a, b) => (get(a) | get(b), 0),
                        BlockOp::Xor(a, b) => (get(a) ^ get(b), 0),
                        BlockOp::AndNot(a, b) => (get(a) & !get(b), 0),
                        BlockOp::Prev(a, d) => (carry::prev(get(a), state[k], d), get(a)),
                        BlockOp::Toggle(a) => carry::toggle(get(a), state[k]),
                        BlockOp::Escaped(a) => carry::escaped(get(a), state[k]),
                        BlockOp::Fill(a, b) => carry::fill(get(a), get(b), state[k]),
                        op => unreachable!("a value that reads nothing is no member: {op:?}"),
                    };
                    local[m] = w;
                    outs[k] = c;
                }
                let next: Vec<u64> = group.carries.iter().map(|&(_, x)| local[x]).collect();
                if next == cur {
                    settled = true;
                    break;
                }
                cur = next;
            }
            assert!(settled, "a strictly causal group settles within {MAX_ROUNDS} rounds");
            for &m in &group.members {
                values[m][i] = local[m];
            }
            state.copy_from_slice(&outs);
        }
    }
}

/// The CPU interpreter over a batch (decision 0066): each file's run under its program, in the
/// files' order. One `interpret_files` per program, over all of that program's files.
pub fn run_batch(programs: &[&BlockProgram], files: &[BatchFile<'_>]) -> Vec<BlockRun> {
    let mut out: Vec<Option<BlockRun>> = vec![None; files.len()];
    for (p, program) in programs.iter().enumerate() {
        let mine: Vec<usize> = (0..files.len()).filter(|&i| files[i].program == p).collect();
        let bytes: Vec<&[u8]> = mine.iter().map(|&i| files[i].bytes).collect();
        for (i, run) in mine.into_iter().zip(program.run_files(&bytes)) {
            out[i] = Some(run);
        }
    }
    out.into_iter().map(|run| run.expect("every file names a program of the batch")).collect()
}

impl BlockProgram {
    /// The CPU interpreter of a row program (`rows`): every value's words over the batch's rows,
    /// `values` holding them value by value (`words` each, reused from run to run), then each
    /// output's words, cut at the batch's length. Straight-line word operations over the
    /// columns; `Cmp` and `Bit` build a word from 64 rows each.
    pub fn interpret_rows<'v>(&'v self, batch: &RowBatch<'_>, values: &'v mut Vec<u64>) -> impl Iterator<Item = &'v [u64]> + 'v {
        let words = batch.len.div_ceil(64);
        values.clear();
        values.resize(self.ops.len() * words, 0);
        for (x, op) in self.ops.iter().enumerate() {
            let (before, rest) = values.split_at_mut(x * words);
            let out = &mut rest[..words];
            let v = |a: u16| &before[usize::from(a) * words..][..words];
            match *op {
                BlockOp::Cmp { col, op, rhs, width } => {
                    let a = batch.columns[usize::from(col)];
                    for (w, word) in out.iter_mut().enumerate() {
                        let rows = &a[64 * w..batch.len.min(64 * w + 64)];
                        let mut m = 0u64;
                        match rhs {
                            Rhs::Param(p) => {
                                let b = lane(batch.params[usize::from(p)], width);
                                for (j, &x) in rows.iter().enumerate() {
                                    m |= u64::from(op.holds(lane(x, width), b)) << j;
                                }
                            }
                            Rhs::Col(c) => {
                                let b = &batch.columns[usize::from(c)][64 * w..];
                                for (j, (&x, &y)) in rows.iter().zip(b).enumerate() {
                                    m |= u64::from(op.holds(lane(x, width), lane(y, width))) << j;
                                }
                            }
                        }
                        *word = m;
                    }
                }
                BlockOp::Bit { set, col } => {
                    let (s, a) = (batch.sets[usize::from(set)], batch.columns[usize::from(col)]);
                    for (w, word) in out.iter_mut().enumerate() {
                        let mut m = 0u64;
                        for (j, &x) in a[64 * w..batch.len.min(64 * w + 64)].iter().enumerate() {
                            let hit = usize::try_from(x).ok().and_then(|x| s.get(x / 64).map(|word| word >> (x % 64) & 1)).unwrap_or(0);
                            m |= hit << j;
                        }
                        *word = m;
                    }
                }
                BlockOp::Const(c) => out.fill(c),
                BlockOp::Not(a) => out.iter_mut().zip(v(a)).for_each(|(o, &x)| *o = !x),
                BlockOp::And(a, b) => out.iter_mut().zip(v(a).iter().zip(v(b))).for_each(|(o, (&x, &y))| *o = x & y),
                BlockOp::Or(a, b) => out.iter_mut().zip(v(a).iter().zip(v(b))).for_each(|(o, (&x, &y))| *o = x | y),
                BlockOp::Xor(a, b) => out.iter_mut().zip(v(a).iter().zip(v(b))).for_each(|(o, (&x, &y))| *o = x ^ y),
                BlockOp::AndNot(a, b) => out.iter_mut().zip(v(a).iter().zip(v(b))).for_each(|(o, (&x, &y))| *o = x & !y),
                _ => unreachable!("verified: a row program reads no bytes"),
            }
        }
        // Bits past the batch are cut from the outputs (a `Not` sets them).
        for &o in &self.outputs {
            let at = usize::from(o) * words;
            mask_tail(&mut values[at..at + words], batch.len);
        }
        let values: &'v [u64] = values;
        self.outputs.iter().map(move |&o| &values[usize::from(o) * words..][..words])
    }

    /// A row program of `ops` over `shape`'s inputs, with its outputs and the outputs whose rows
    /// the extract pass writes; verified.
    pub fn row_program(shape: RowShape, ops: Vec<BlockOp>, outputs: Vec<u16>, extracts: Vec<u16>) -> Result<Self, Error> {
        let program = BlockProgram { ops, outputs, extracts, rows: Some(shape), ..BlockProgram::default() };
        program.verify()?;
        Ok(program)
    }

    /// A row program's operations, in order (an interpreter of them elsewhere: the device's `metal_join`).
    pub fn ops(&self) -> &[BlockOp] {
        &self.ops
    }

    /// A program's outputs: the operations whose values it gives.
    pub fn outputs(&self) -> &[u16] {
        &self.outputs
    }

    /// A row program on the CPU JIT (`native_cpu_rows`).
    pub fn compile_rows(&self) -> Result<crate::native_cpu_rows::CompiledRows, Error> {
        self.verify()?;
        let shape = self.rows.ok_or(Error::InvalidTarget(0))?;
        crate::native_cpu_rows::compile(&crate::block_native::BlockNative::lower(self), usize::from(shape.columns))
    }

    /// A row program's extract pass: the rows (numbers within the batch, in order) whose bit
    /// output `k` sets, appended to `out`.
    pub fn extract_rows(&self, batch: &RowBatch<'_>, k: usize, values: &mut Vec<u64>, out: &mut Vec<u32>) {
        let words = self.interpret_rows(batch, values).nth(k).expect("an output");
        for (w, &word) in words.iter().enumerate() {
            let mut m = word;
            while m != 0 {
                out.push(64 * w as u32 + m.trailing_zeros());
                m &= m - 1;
            }
        }
    }
}

/// The extract pass on the CPU (`10-bit-programs.md` §4.1): a plane's set bits as
/// byte positions, into a vector allocated once at its final size.
pub mod extract {
    /// The positions of `words`' set bits, `count` of them (the mask pass's popcount), by
    /// simdjson's flattening (`bit_indexer::write`): per word, eight positions written whether
    /// or not the word has them (trailing zeros, then clear the lowest bit), then eight more
    /// while the word has more; the slack past the last position is never read.
    pub fn positions(words: &[u64], count: usize) -> Vec<u32> {
        // A word writes at most seven entries past its own bits, which the next word overwrites;
        // eight entries of slack cover the last.
        let mut out = vec![0u32; count + 8];
        let mut at = 0usize;
        for (i, &word) in words.iter().enumerate() {
            let base = i as u32 * 64;
            let n = word.count_ones() as usize;
            let mut w = word;
            let mut write8 = |w: &mut u64, at: usize| {
                for j in 0..8 {
                    out[at + j] = base + w.trailing_zeros();
                    *w &= w.wrapping_sub(1);
                }
            };
            write8(&mut w, at);
            let mut written = 8;
            while written < n {
                write8(&mut w, at + written);
                written += 8;
            }
            at += n;
        }
        assert_eq!(at, count, "the extract's count is the plane's popcount");
        out.truncate(count);
        out
    }
}

/// The source as whole 64-byte blocks, zero past its end.
pub fn padded(bytes: &[u8]) -> Vec<[u8; 64]> {
    bytes
        .chunks(64)
        .map(|chunk| {
            let mut block = [0u8; 64];
            block[..chunk.len()].copy_from_slice(chunk);
            block
        })
        .collect()
}

/// The blocks of `bytes` and `extra` zero bytes after them.
pub fn padded_by(bytes: &[u8], extra: usize) -> Vec<[u8; 64]> {
    let mut blocks = padded(bytes);
    let want = (bytes.len() + extra).div_ceil(64);
    blocks.resize(want.max(blocks.len()), [0u8; 64]);
    blocks
}

/// `within`'s word for block `at` of a stream of `len` bytes: the bits below the stream's end.
/// Two shifts, so 64 bits is all ones with no shift by 64.
pub fn within(at: usize, len: usize) -> u64 {
    let t = len.saturating_sub(at * 64).min(64) as u32;
    ((1u64 << (t >> 1)) << (t - (t >> 1))).wrapping_sub(1)
}

/// `within` at delay `d` for block `at`: the bits `d <= i < len + d`.
/// Multiples of `k` below 64 (`k` in 1..=63): `phase`'s word at a position that is one.
pub fn period_bits(k: u8) -> u64 {
    (0..64).step_by(usize::from(k)).fold(0u64, |acc, i| acc | 1 << i)
}

/// `phase k r`'s word for the 64 bytes from position `at`: bit `j` where `(at + j) % k == r`,
/// the period's bits shifted up by the first such `j`.
pub fn phase(at: usize, k: u8, r: u8) -> u64 {
    let k = usize::from(k);
    let first = (usize::from(r) + k - at % k) % k;
    period_bits(k as u8) << first
}

/// Plane `j` of the positions of word `at`'s bytes: bit `i` is bit `j` of `64 at + i`.
pub fn position(at: usize, j: u8) -> u64 {
    const LOW: [u64; 6] = [0xAAAA_AAAA_AAAA_AAAA, 0xCCCC_CCCC_CCCC_CCCC, 0xF0F0_F0F0_F0F0_F0F0, 0xFF00_FF00_FF00_FF00, 0xFFFF_0000_FFFF_0000, 0xFFFF_FFFF_0000_0000];
    match j {
        0..=5 => LOW[usize::from(j)],
        _ if (at as u64) >> (j - 6) & 1 == 1 => u64::MAX,
        _ => 0,
    }
}

pub fn within_at(at: usize, len: usize, d: u8) -> u64 {
    within(at, len + usize::from(d)) & !within(at, usize::from(d))
}

/// An executor's output words as the stream's mask: shifted down by the program's `delay` (bit
/// `i + delay` is byte `i`'s), cut to `len` bytes. Shared by every executor's host side.
pub fn undelay(words: &[u64], delay: u8, len: usize) -> Vec<u64> {
    let d = u32::from(delay);
    let mut out: Vec<u64> = (0..len.div_ceil(64))
        .map(|j| {
            let w = words.get(j).copied().unwrap_or(0);
            if d == 0 {
                return w;
            }
            (w >> d) | (words.get(j + 1).copied().unwrap_or(0) << (64 - d))
        })
        .collect();
    mask_tail(&mut out, len);
    out
}

/// Clear the bits past `len` bytes.
pub fn mask_tail(words: &mut [u64], len: usize) {
    if let Some(last) = words.last_mut()
        && len % 64 != 0
    {
        *last &= (1u64 << (len % 64)) - 1;
    }
}

/// `eq` over one block: four 16-byte compares and simdjson's arm64 `to_bitmask`.
#[cfg(target_arch = "aarch64")]
pub fn eq(block: &[u8; 64], byte: u8) -> u64 {
    use core::arch::aarch64::*;
    // SAFETY: NEON is in the aarch64 baseline; each load reads 16 bytes of the 64-byte block.
    unsafe {
        let c = vdupq_n_u8(byte);
        let at = |offset: usize| vceqq_u8(vld1q_u8(block.as_ptr().add(offset)), c);
        to_bitmask(at(0), at(16), at(32), at(48))
    }
}

/// Off aarch64, the SWAR form.
#[cfg(not(target_arch = "aarch64"))]
pub fn eq(block: &[u8; 64], byte: u8) -> u64 {
    eq_swar(block, byte)
}

/// `class` over one block: two nibble lookups per 16 bytes (`vqtbl1q_u8`, as simdjson's arm64
/// `classify`), an AND, a test against the class bits, and the bitmask.
#[cfg(target_arch = "aarch64")]
pub fn class(block: &[u8; 64], table: &ClassTable, bits: u8) -> u64 {
    use core::arch::aarch64::*;
    // SAFETY: NEON is in the aarch64 baseline; each load reads 16 bytes of the block or a table.
    unsafe {
        let lo = vld1q_u8(table.lo.as_ptr());
        let hi = vld1q_u8(table.hi.as_ptr());
        let nibble = vdupq_n_u8(0x0f);
        let sel = vdupq_n_u8(bits);
        let at = |offset: usize| {
            let d = vld1q_u8(block.as_ptr().add(offset));
            let l = vqtbl1q_u8(lo, vandq_u8(d, nibble));
            let h = vqtbl1q_u8(hi, vshrq_n_u8::<4>(d));
            vtstq_u8(vandq_u8(l, h), sel)
        };
        to_bitmask(at(0), at(16), at(32), at(48))
    }
}

/// Off aarch64, the same lookups by SWAR are not available; the portable form reads the tables
/// a byte at a time. It is not a measured path (x86 would use `pshufb`).
#[cfg(not(target_arch = "aarch64"))]
pub fn class(block: &[u8; 64], table: &ClassTable, bits: u8) -> u64 {
    block.iter().enumerate().fold(0, |m, (j, &b)| m | (u64::from(table.member(b, bits)) << j))
}

/// Each block's bytes one back: byte `j` of block `i`'s is stream byte `64 i + j - 1`, 0x00
/// before the stream.
pub fn before_blocks(blocks: &[[u8; 64]]) -> Vec<[u8; 64]> {
    let mut last = 0u8;
    blocks
        .iter()
        .map(|block| {
            let mut out = [0u8; 64];
            out[0] = last;
            out[1..].copy_from_slice(&block[..63]);
            last = block[63];
            out
        })
        .collect()
}

/// `pair` over one block, given the block's bytes one back: two lookups at each, an AND of the
/// class bytes, a test against the class bits, and the bitmask.
#[cfg(target_arch = "aarch64")]
pub fn pair(prev: &[u8; 64], block: &[u8; 64], before: &ClassTable, table: &ClassTable, bits: u8) -> u64 {
    use core::arch::aarch64::*;
    // SAFETY: NEON is in the aarch64 baseline; each load reads 16 bytes of a block or a table.
    unsafe {
        let classes = |t: &ClassTable, d: uint8x16_t| {
            let l = vqtbl1q_u8(vld1q_u8(t.lo.as_ptr()), vandq_u8(d, vdupq_n_u8(0x0f)));
            let h = vqtbl1q_u8(vld1q_u8(t.hi.as_ptr()), vshrq_n_u8::<4>(d));
            vandq_u8(l, h)
        };
        let sel = vdupq_n_u8(bits);
        let at = |offset: usize| {
            let p = classes(before, vld1q_u8(prev.as_ptr().add(offset)));
            let b = classes(table, vld1q_u8(block.as_ptr().add(offset)));
            vtstq_u8(vandq_u8(p, b), sel)
        };
        to_bitmask(at(0), at(16), at(32), at(48))
    }
}

/// Off aarch64, a byte at a time (not a measured path).
#[cfg(not(target_arch = "aarch64"))]
pub fn pair(prev: &[u8; 64], block: &[u8; 64], before: &ClassTable, table: &ClassTable, bits: u8) -> u64 {
    (0..64).fold(0, |m, j| {
        let both = before.lo[usize::from(prev[j] & 15)] & before.hi[usize::from(prev[j] >> 4)]
            & table.lo[usize::from(block[j] & 15)] & table.hi[usize::from(block[j] >> 4)];
        m | (u64::from(both & bits != 0) << j)
    })
}

/// simdjson's arm64 `simd8x64<bool>::to_bitmask`: each byte weighted by its bit, then three
/// rounds of pairwise adds.
#[cfg(target_arch = "aarch64")]
unsafe fn to_bitmask(
    m0: core::arch::aarch64::uint8x16_t,
    m1: core::arch::aarch64::uint8x16_t,
    m2: core::arch::aarch64::uint8x16_t,
    m3: core::arch::aarch64::uint8x16_t,
) -> u64 {
    use core::arch::aarch64::*;
    const WEIGHTS: [u8; 16] = [1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128];
    // SAFETY: NEON is in the aarch64 baseline; the load reads the 16-byte constant.
    unsafe {
        let w = vld1q_u8(WEIGHTS.as_ptr());
        let s0 = vpaddq_u8(vandq_u8(m0, w), vandq_u8(m1, w));
        let s1 = vpaddq_u8(vandq_u8(m2, w), vandq_u8(m3, w));
        let s = vpaddq_u8(s0, s1);
        let s = vpaddq_u8(s, s);
        vgetq_lane_u64::<0>(vreinterpretq_u64_u8(s))
    }
}

/// `eq` over one block by SWAR: eight 8-byte words, each a zero-byte test and a multiply that
/// gathers the eight high bits.
pub fn eq_swar(block: &[u8; 64], byte: u8) -> u64 {
    const LOW: u64 = 0x7f7f_7f7f_7f7f_7f7f;
    const HIGH: u64 = 0x8080_8080_8080_8080;
    const GATHER: u64 = 0x0002_0408_1020_4081;
    let c = u64::from(byte) * 0x0101_0101_0101_0101;
    let word = |at: usize| {
        let w = u64::from_le_bytes(block[at * 8..at * 8 + 8].try_into().unwrap()) ^ c;
        // High bit of each byte that is zero, exactly (no borrow between bytes).
        let zero = !(((w & LOW) + LOW) | w) & HIGH;
        // Byte i's high bit (bit 8i + 7) to bit i.
        ((zero >> 7).wrapping_mul(GATHER) >> 49) & 0xff
    };
    word(0)
        | word(1) << 8
        | word(2) << 16
        | word(3) << 24
        | word(4) << 32
        | word(5) << 40
        | word(6) << 48
        | word(7) << 56
}

/// Block program `at`'s rows: tables, operations, outputs, extracts, delay, and the tree section (`tree_rows`).
pub fn block_rows(out: &mut String, at: usize, block: &BlockProgram) {
    use std::fmt::Write as _;
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    for (t, table) in block.tables.iter().enumerate() {
        let _ = writeln!(out, "  block {at} table {t} lo={} hi={}", hex(&table.lo), hex(&table.hi));
    }
    // `block N rows C P S`: a row program's columns, parameters and sets (`RowShape`).
    if let Some(shape) = block.rows {
        let _ = writeln!(out, "  block {at} rows {} {} {}", shape.columns, shape.params, shape.sets);
    }
    for (value, op) in block.ops.iter().enumerate() {
        let _ = writeln!(out, "  block {at} %{value} = {}", op.text());
    }
    // `block N rounds %c %n …`: one fixed point's carries, each with its next.
    for group in &block.rounds {
        let pairs: Vec<String> = group.iter().map(|(c, x)| format!("%{c} %{x}")).collect();
        let _ = writeln!(out, "  block {at} rounds {}", pairs.join(" "));
    }
    for (k, value) in block.outputs.iter().enumerate() {
        match block.output_delay(k) {
            d if d == block.delay => {
                let _ = writeln!(out, "  block {at} output %{value}");
            }
            d => {
                let _ = writeln!(out, "  block {at} output %{value} delay {d}");
            }
        }
    }
    for value in &block.extracts {
        let _ = writeln!(out, "  block {at} extract %{value}");
    }
    for value in &block.totals {
        let _ = writeln!(out, "  block {at} total %{value}");
    }
    for value in &block.masks {
        let _ = writeln!(out, "  block {at} mask %{value}");
    }
    if block.delay != 0 {
        let _ = writeln!(out, "  block {at} delay {}", block.delay);
    }
}
