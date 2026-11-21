//! The Bitlex compiler: a Bitlex file (`docs/parser-generators/03-bitlex.md`) to a block program
//! (`block`), printed as `rir` rows so it goes through the transducer's own assembler and
//! verifier. It holds no language: everything comes from the file.
//!
//! The parser is the reference evaluator's (`tools/bitlex`), so Bitlex is parsed in one place, and
//! the evaluator is the compiler's test oracle (`rerail_parser_tests`'s `compile_tests`).
//!
//! Built one construct at a time (`docs/parser-generators/13-tape-on-the-gpu.md` §7, step 2).
//!
//! - **Classes.** Each class is a union of products of a set of high nibbles and a set of low
//!   nibbles, packed eight to a nibble table. A class's products are placed together, in one
//!   table (`place`), so a class is one lookup, as simdjson's are; a product two classes share
//!   in one table serves both. A one-byte class is an `eq`.
//! - **Masks** over `~ & | ^`, `empty`, `prev`, `next`, `starts`, `lasts`, `after`, `span`,
//!   `aligned`, `toggle`, `after_odd_run` and `fill`.
//! - **Counts** as bit-planes, and `C.frame(a)` (below).
//! - **Numbers** (`number`) as bit-planes too: `NUMBER_BITS` of them, a byte's value mod 2^32
//!   (below).
//! - **Tokens**, `keywords` and `skip`: each token kind's start and last-byte masks, and the
//!   coverage and overlap checks.
//! - **The tree section** (`docs/parser-generators/18-tree-as-grammar.md`): the grammar's state
//!   machine (`grammar`), with grammar ids from `Symbols`, lowered to tier 2's words
//!   (`bitlex_tier2`, decision 0087); tier 1 outputs the stream they read, two extracts.
//! - **Errors and escapes.** Each mask joins the fallback output (`FALLBACK`); a file with a bit
//!   of it goes to tree-sitter.
//!
//! Anything else is refused by name.
//!
//! # Counts and frames
//!
//! A count `count(up, down) max M` is kept as `w` bit-planes of its value after each byte, with
//! `2^w >= M + 2`. Bit `k` flips at an up byte whose bits below `k` were all 1, and at a down
//! byte whose bits below `k` were all 0, so plane `k` is a `toggle` of a mask of the planes
//! below it one byte back: no new operation. The value is kept mod `2^w`; a count that leaves
//! `0..=M` passes through `M + 1` or `-1`, both above `M` as `w`-bit numbers, and one compare
//! escapes the file. (The evaluator holds a count at 0 instead: a file that goes below 0 escapes
//! here and is lexed there; such a file is malformed for JSON.)
//!
//! `C.frame(a)` is one `fill` per level `L` in `1..=M` (`10-bit-programs.md` §2): from the
//! openers at level `L` in `a` to the next opener at level `L` not in `a`, kept where the count
//! before the byte is `L`.
//!
//! # Numbers
//!
//! A number is `NUMBER_BITS` planes, plane `k` the bytes whose value has bit `k`. A file is
//! shorter than 2^32 bytes, so a place in it, or a distance, is exact.
//!
//! - `position`: plane `k` a pattern of the word below bit 6 (`const`), the block's place
//!   from bit 6 on (`position k`, a new leaf with no carry).
//! - `fill(s, e, v)`: plane `k` is `fill(s & v_k, e | s)`, a fill that restarts at every byte
//!   of `s` and runs where `v` had bit `k` there; `back_fill` likewise.
//! - `a - b`: a ripple of borrows over the planes, three operations a plane.
//! - `at_least(a, b)`: no borrow out of `a - b`; `above(a, b)` is `~at_least(b, a)`.
//!
//! No new operation but `position`: every plane is a mask, every step a word operation.
//!
//! # Delays: `next` without lookahead
//!
//! Every value is computed some bytes late: its bit `p` is byte `p - d`'s, for its delay `d`.
//! Bytes and classes have delay 0. An operation's operands are brought to the later delay by
//! `prev`, and `next(a, k)` is `a` itself with `k` more delay. The outputs are brought to the
//! largest delay `D`, the program's `delay`; each executor runs `D` bytes past the stream and
//! shifts the outputs back (`block::undelay`).
//!
//! So that no value has bits outside the stream (which `prev` and `next` would carry in):
//!
//! - before the stream every value is zero: `prev` shifts in zeros, `~a` is `within & ~a` with
//!   `within` (the stream's bytes) at the same delay, and `next` clears the bytes it brings in
//!   before the start;
//! - past the end a value may be *dirty*: a byte test that the zero padding passes (`eq 0x00`),
//!   a constant, or a carried operation whose state runs on. `next` cleans its operand with
//!   `within` when it is dirty.

use crate::block::{BlockOp, BlockProgram, ClassTable};
use crate::{bitlex_prove, bitlex_tier2, grammar};
use ::bitlex::syntax::{Arg, Expr, NExpr, Stmt, TokenElem};
use std::cell::RefCell;
/// The lowering's maps: hashed with a fixed seed (`foldhash`'s), so what iterates one (a proof's order, the
/// candidates' order) goes the same way each run: one program from one grammar.
type HashMap<K, V> = std::collections::HashMap<K, V, foldhash::fast::FixedState>;

/// A number's planes: a byte's value mod 2^32, exact for a place in a file or a distance.
pub const NUMBER_BITS: u8 = 32;

/// A compiled program: its `rir` rows (`block 0 …`), whose outputs are the requested masks in
/// order.
#[derive(Debug)]
pub struct Compiled {
    /// The block program itself, as its rows print it.
    pub program: BlockProgram,
    pub rows: String,
    /// Tier 2's words (decision 0087), with their strings and switches; `0000 STOP 0` without a
    /// tree section.
    pub words: String,
    /// The tree section's state machine as text (`grammar::text`), which the words are lowered from.
    pub machine: String,
    /// Each requested name, then each token row's masks (`token KIND start`, `token KIND end`)
    /// and `skip`, and its output: a value asked for twice is one output.
    pub labels: Vec<(String, u16)>,
    /// The fallback's outputs (`FALLBACK`): one per delay its masks have, none brought to
    /// another delay; the file goes to tree-sitter when any has a bit.
    pub fallbacks: Vec<u16>,
    /// Tier 1's cost by source: each statement (its kind and name) or lowering step, with the
    /// block operations it added and how many of them are `back_fill`'s (each a backward pass).
    pub costs: Vec<Cost>,
    /// Each operation of `rows`' block program: the statement (as in `costs`) that made it,
    /// empty for none. The profile names what is live where by it (`native_cpu_pressure`).
    pub origin: Vec<String>,
    /// The program's named values, at their places in `rows`' block program: each `mask` and
    /// class by its name, each mark as `mark KIND`, each token row's first and last bytes as
    /// `token KIND start` and `token KIND last`. A value no output reads is left out.
    pub values: Vec<Named>,
    /// The provers' answers this compile made (decision 0488): what a later compile is given to
    /// answer the same questions without their proofs (`compile_proved`).
    pub proofs: Vec<(Vec<u8>, bitlex_prove::Answer)>,
    /// Tier 2 as Rust, written from the tree rules (`bitlex_rust`), or why it is not yet.
    pub rust: Option<Result<String, String>>,
}

/// A named value of the block program (`Compiled::values`): its operation, and its delay (bit
/// `p + delay` of the operation's word is the value at byte `p`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Named {
    pub name: String,
    pub op: u16,
    pub delay: u8,
}

/// A statement's share of tier 1 (`Compiled::costs`).
#[derive(Clone, Debug)]
pub struct Cost {
    pub what: String,
    pub ops: u32,
    pub back_fills: u32,
}

impl Compiled {
    /// The whole program as `rir`: its block rows, then its words.
    pub fn text(&self) -> String {
        format!("program -\n  mode set=0 query=0\n{}{}", self.rows, self.words)
    }
}


/// The output names of the stream tier 2 reads (decision 0087): every token's first byte, and
/// the last byte of each token whose arm reads it.
pub const STREAM_START: &str = "stream start";
pub const STREAM_LAST: &str = "stream last";
/// A mark's bits at the token starts (`MARK`), one output a mark.
pub const STREAM_MARK: &str = "stream mark";

/// The output name for the fallback bit: set somewhere when the file goes to tree-sitter.
pub const FALLBACK: &str = "fallback";
/// An output of the fallback's the token loop checks (`Stmt::TokenError`): a total, which the
/// CPU does not make (`Records::untaken`).
pub const FALLBACK_TOKENS: &str = "fallback by tokens";

/// A product of nibble sets: bit `h` of `hi` and bit `l` of `lo` for byte `h << 4 | l`.
pub type Product = (u16, u16);

/// A compiled mask: its value, how many bytes late it is, and whether it may have bits past the
/// stream's end.
#[derive(Clone, Copy, Debug)]
pub struct Val {
    pub v: u16,
    pub d: u8,
    pub dirty: bool,
    /// The bytes it holds, when it is a byte class at delay 0 (a function of the byte alone):
    /// two classes combined are one class, a single bitmask (decision 0069).
    pub set: Option<ByteSet>,
}

/// A set of bytes, bit `b` for byte `b`.
pub type ByteSet = [u64; 4];

/// The largest delay: the outputs are brought to it by `prev`s, and the executors shift it out.
pub const MAX_DELAY: u8 = 63;

/// What a value's word is known to be, over a file's words end to end (bit `i` of word `w` is
/// position `64 w + i`; `len` the file's length):
/// - `zero_from`: no bit at a position from it on (`first_bits`);
/// - `ones`: `(g, e)`, a bit at every position from `g` up to `len + e` (`None`: past the end
///   too);
/// - `zero_past`: no bit at a position from `len + zero_past` on.
#[derive(Clone, Copy, Debug, Default)]
struct Bounds {
    zero_from: Option<u32>,
    ones: Option<(u32, Option<u32>)>,
    zero_past: Option<u32>,
}

/// The positions value `v`'s bits lie below, when they all lie in a file's first bytes: a
/// file's first byte, `~prev(~empty)`, has none from position 1 on, and so has an `and` with
/// it; a `prev` of such a value, and an `or` of two, are bounded too. A value bounded so moves
/// its bits down in the word with no carry (`BlockOp::Early`, `Lowering::early_to`). `None`
/// where the operations do not show it.
pub fn first_bits(ops: &[BlockOp], tables: &[ClassTable], v: u16) -> Option<u32> {
    fn go(ops: &[BlockOp], tables: &[ClassTable], v: u16, memo: &mut HashMap<u16, Bounds>, depth: u32) -> Bounds {
        if let Some(&b) = memo.get(&v) {
            return b;
        }
        if depth > 256 {
            return Bounds::default();
        }
        let mut get = |x: u16| go(ops, tables, x, memo, depth + 1);
        // The smaller of two bounds, either one known; the larger, both known.
        let min = |x: Option<u32>, y: Option<u32>| match (x, y) {
            (Some(x), Some(y)) => Some(x.min(y)),
            (x, None) | (None, x) => x,
        };
        let max = |x: Option<u32>, y: Option<u32>| Some(x?.max(y?));
        // The end of a run of ones, `None` past the end: the nearer of two.
        let end = |x: Option<u32>, y: Option<u32>| match (x, y) {
            (Some(x), Some(y)) => Some(x.min(y)),
            (x, None) | (None, x) => x,
        };
        let b = match ops[usize::from(v)] {
            BlockOp::Const(0) => Bounds { zero_from: Some(0), ones: None, zero_past: Some(0) },
            BlockOp::Const(u64::MAX) => Bounds { zero_from: None, ones: Some((0, None)), zero_past: None },
            // A test the padding's 0x00 fails is zero past the end; one every byte passes, all
            // ones.
            BlockOp::Eq(c) => Bounds { zero_from: None, ones: None, zero_past: (c != 0).then_some(0) },
            BlockOp::Class { table, bits } => {
                let t = &tables[usize::from(table)];
                let all = (0..=255u8).all(|b| t.member(b, bits));
                Bounds { zero_from: None, ones: all.then_some((0, None)), zero_past: (!t.member(0, bits)).then_some(0) }
            }
            BlockOp::Within(d) => Bounds { zero_from: None, ones: Some((u32::from(d), Some(u32::from(d)))), zero_past: Some(u32::from(d)) },
            BlockOp::Prefix(n, _) => Bounds { zero_from: Some(u32::from(n)), ones: None, zero_past: Some(0) },
            BlockOp::And(a, c) => {
                let (x, y) = (get(a), get(c));
                let ones = match (x.ones, y.ones) {
                    (Some((g, e)), Some((h, f))) => Some((g.max(h), end(e, f))),
                    _ => None,
                };
                Bounds { zero_from: min(x.zero_from, y.zero_from), ones, zero_past: min(x.zero_past, y.zero_past) }
            }
            BlockOp::Or(a, c) => {
                let (x, y) = (get(a), get(c));
                // Either run of ones is one of the result's.
                let ones = match (x.ones, y.ones) {
                    (Some(o), Some(p)) => Some(if o.0 <= p.0 { o } else { p }),
                    (o, None) | (None, o) => o,
                };
                Bounds { zero_from: max(x.zero_from, y.zero_from), ones, zero_past: max(x.zero_past, y.zero_past) }
            }
            BlockOp::Xor(a, c) => {
                let (x, y) = (get(a), get(c));
                Bounds { zero_from: max(x.zero_from, y.zero_from), ones: None, zero_past: max(x.zero_past, y.zero_past) }
            }
            // `a & ~c`: zero where `a` is, and where `c` is ones as far as `a` reaches; ones
            // where `a` is and `c` is zero.
            BlockOp::AndNot(a, c) => {
                let (x, y) = (get(a), get(c));
                let under = match y.ones {
                    Some((g, None)) => Some(g),
                    Some((g, Some(e))) if x.zero_past.is_some_and(|z| z <= e) => Some(g),
                    _ => None,
                };
                let ones = match (x.ones, y.zero_from) {
                    (Some((g, e)), Some(z)) => Some((g.max(z), e)),
                    _ => None,
                };
                Bounds { zero_from: min(x.zero_from, under), ones, zero_past: x.zero_past }
            }
            BlockOp::Not(a) => {
                let x = get(a);
                Bounds { zero_from: match x.ones {
                    Some((g, None)) => Some(g),
                    _ => None,
                }, ones: x.zero_from.map(|z| (z, None)), zero_past: None }
            }
            BlockOp::Prev(a, k) => {
                let (x, k) = (get(a), u32::from(k));
                Bounds { zero_from: x.zero_from.map(|z| z + k), ones: x.ones.map(|(g, e)| (g + k, e.map(|e| e + k))), zero_past: x.zero_past.map(|z| z + k) }
            }
            BlockOp::Early(a, k) => {
                let (x, k) = (get(a), u32::from(k));
                Bounds { zero_from: x.zero_from.map(|z| z.saturating_sub(k)), ones: None, zero_past: x.zero_past.map(|z| z.saturating_sub(k)) }
            }
            _ => Bounds::default(),
        };
        memo.insert(v, b);
        b
    }
    go(ops, tables, v, &mut HashMap::default(), 0).zero_from
}

pub struct Lowering {
    /// Tier 2's shape.
    pub shape: Shape,
    pub program: BlockProgram,
    /// Each operation's value, so an operation is emitted once.
    pub seen: HashMap<BlockOp, u16>,
    /// Each table's products, bit by bit (`place`).
    pub table_products: Vec<Vec<Product>>,
    /// Classes, lowered when first used.
    pub classes: HashMap<String, Vec<(u32, u32)>>,
    pub names: HashMap<String, Val>,
    /// The masks any of whose bits sends the file to tree-sitter: errors and escapes.
    pub fallbacks: Vec<Val>,
    /// The token loop's errors (`Stmt::TokenError`): each an output of its own (`FALLBACK_TOKENS`).
    pub untaken: Vec<Val>,
    pub counts: HashMap<String, Count>,
    /// The counts no value has read yet (`C.frame` lowers one when it reads it): a tree's whose
    /// ups and downs are tokens' starts are tier 2's (`COUNT`, `tree_section`), the rest lowered
    /// after it. Each its name, up, down and most.
    pub pending_counts: Vec<(String, Val, Val, u32)>,
    /// The escapes and errors `A & L` whose `L` is local, a few byte tests at fixed offsets
    /// (`local_ops`): a tree's whose `A` lies in one row's tokens are tier 2's (`CHECK`,
    /// `tree_section`), the rest joined to the fallback after it. With `true`, the error is
    /// `A & ~L` (decision 0399): the check's last op is `L`'s `not`, which a check of `A`'s
    /// bytes takes as it is, with no cut to the stream.
    pub pending_checks: Vec<(Val, Val, bool)>,
    /// A pending `A & ~L` whose `L` is not local (decision 0428): its value as the fallback's.
    pub back_fallbacks: HashMap<(u16, u16), Val>,
    /// Each number's planes, bit 0 first (`NUMBER_BITS`).
    pub numbers: HashMap<String, Vec<Val>>,
    pub tokens: Vec<(String, Val, Option<TokenEnd>)>,
    /// Each token kind's end rule (`run R`, `pair P`, decision 0177): whether a pair, its mask.
    pub token_rules: HashMap<String, (bool, Val)>,
    /// Each token kind's pairs and triples (`pair P`, `TokenRule::Pairs`): on the CPU, its
    /// first byte and the next, and the one after (`EndRule::Next`), not its mask.
    pub token_pairs: HashMap<String, Vec<Vec<u8>>>,
    /// The starts of a `pair P` kind's triples (`P_three`), where it has any.
    pub token_threes: HashMap<String, Val>,
    /// The tokens read by a reader (`read K | …`, `records::Reader`): each region's opener,
    /// closer, last byte's offset from the closer, and what no closer means.
    pub token_reads: HashMap<String, Vec<(Val, Val, i8, crate::records::ReadNone)>>,
    /// A region read in parts with `closes`: its closer's last (`ReadArm::last`).
    pub token_closer_last: HashMap<String, Val>,
    /// A kind read in parts (`read K parts { … }`): each part's start mask and bytes, in order.
    /// Each part's start, and its bytes or its last (`ReadPart`).
    pub token_parts: HashMap<String, Vec<(Val, u8, Option<Val>)>>,
    /// A region read in parts: its parts' kinds, in order, and its content's kind.
    pub token_part_names: HashMap<String, (Vec<String>, String)>,
    /// A region read in parts: its fill (`f_R`), by the reader's kind (`Reader::quiet`).
    pub token_fills: HashMap<String, Val>,
    /// Each token read whole by a region's reader (`read R`, `read R | S`): its regions' fills,
    /// its bytes (the blank marks' proof, decision 0430).
    pub read_fills: HashMap<String, Val>,
    /// Each token row's (`tokens`, in order): read inside a region by its reader (`Inside`),
    /// so no first byte ends it by a rule.
    pub inside: Vec<bool>,
    /// Each token's reader, where it is a token inside a region read in parts, and whether it
    /// closes the region (`TokenRule::Inside`): `row_modes`' reader modes.
    pub inside_of: Vec<Option<(String, bool)>>,
    pub keywords: Vec<(String, Vec<(Vec<u8>, Option<String>)>, bool)>,
    /// Each row's soft keywords: a word's kind where the grammar takes it, the row's elsewhere.
    pub soft: Vec<(String, Vec<Vec<u8>>)>,
    pub skip: Option<Val>,
    /// The prefix (`Stmt::Prefix`, decision 0335): its bytes, and its value, the bytes it covers
    /// where the file starts with it (a value of the file's first bytes, `first_bits`). The
    /// stream (`within`) is then the file's bytes past it, and a class that holds one of its
    /// bytes is cut to the stream.
    pub prefix: Option<(Vec<u8>, Val)>,
    /// The provers' answers, given and made (decision 0488).
    pub proofs: bitlex_prove::Proofs,
    /// Lowering a mask read only at and after a token's first byte (`ahead`): no cut by the
    /// prefix, whose bytes lie before every token's.
    pub ahead: bool,
    /// Each `mask`'s expression, for `ahead`, which lowers it again.
    pub mask_exprs: HashMap<String, Expr>,
    /// The tree section's rules (`node`, `choice`), in order, and its extra kinds.
    pub tree: Vec<Stmt>,
    pub extra: Vec<String>,
    /// Each `mark`, in order: its kind, its bits and whether the file's end has it; and the
    /// hidden ones' kinds.
    pub marks: Vec<(String, Val, bool)>,
    /// Each `mark`'s expression, as written (`tree_section` finds a word in it).
    pub mark_exprs: Vec<Expr>,
    /// The token marks (`mark K = tokens …`, decision 0129), by their index in `marks`: the token
    /// before, then the run. Tier 1 makes no mask for them.
    pub token_marks: HashMap<usize, (Vec<TokenElem>, Vec<Vec<TokenElem>>)>,
    /// A mark's `before`s and `unless before`s (decision 0490), by its index in `marks`: the
    /// tokens from the one it is before, each with whether it is an `unless`, tested by tier 2 at
    /// that token as a token mark's run is (`Mark::nexts`).
    pub mark_next: HashMap<usize, Vec<(bool, Vec<Vec<TokenElem>>)>>,
    /// A mark's `unless after` words, by its index in `marks` (`Records::unless`).
    pub unless: HashMap<usize, Vec<Vec<u8>>>,
    /// The marks with an `else escape`, by their index in `marks` (`Records::unplaced`).
    pub unplaced: Vec<usize>,
    /// The marks that settle the extras before their bit (`, settles`), by their index in
    /// `marks` (`program::SETTLE`).
    pub settles: Vec<usize>,
    /// The marks that drop the extra pending last (`, drops …`), by their index in `marks`: the
    /// questions (`GapPattern`, decision 0451).
    pub drops: HashMap<usize, Vec<::bitlex::syntax::GapPattern>>,
    /// The marks that lose the extra pending last where a level asks for them (`, loses …`), by
    /// their index in `marks`: the questions (`program::SETTLE` mode 2).
    pub loses: HashMap<usize, Vec<::bitlex::syntax::GapPattern>>,
    /// The marks that refuse the file where a level asks for them (`, refuses …`), by index.
    pub refuses: HashMap<usize, Vec<::bitlex::syntax::GapPattern>>,
    /// The marks that refuse the file where a level takes them (`, refuses taken …`), by index.
    pub refuses_taken: HashMap<usize, Vec<::bitlex::syntax::GapPattern>>,
    /// The marks a level that may end ends at (`, ends`), by their index in `marks`.
    pub ends: Vec<usize>,
    /// The marks each mark passes too where it is taken (`, passes`), by its index in `marks`.
    pub passes: HashMap<usize, Vec<String>>,
    /// Each token test (`Stmt::TokenEscape`): its kind, whether a count, its mask and count
    /// (`Records::checks`).
    pub token_escapes: Vec<(String, bool, Val, u32, crate::records::CheckThen)>,
    /// The marks with token tests and no mask (`Records::tested_only`).
    pub tested_only: Vec<usize>,
    /// The marks' token tests (`MarkCheck`), lowered once the tree says which marks are hidden
    /// and refused: the mark, `after`, the kind, `count`, the least, the mask.
    pub mark_tests: Vec<(usize, bool, String, bool, u32, Expr)>,
    /// Token tests that are tier 2's (`CHECK` at the kind's tokens): `count(A & L) at least 1`,
    /// `L` local, checked at each bit of `A` in the token, as a lexer checks an escape it reads.
    /// The kind, the mark whose refusing levels alone check it (none: everywhere), `A`, `L`.
    pub kind_checks: Vec<(String, Option<usize>, Val, Val)>,
    pub hidden: Vec<String>,
    /// Each `check`: a token kind and its grammar over the token's bytes.
    pub checks: Vec<(String, ::bitlex::syntax::Gram)>,
    /// Each `toggle(x)`: its value and `x`'s, the modes tier 2's states are read in
    /// (`row_modes`); each region's fill and its flips likewise.
    pub toggles: Vec<(Val, Val)>,
    /// Each cost's source and its operations' range (`Compiled::costs`).
    pub costs: Vec<(String, usize, usize)>,
    /// Tier 2 as Rust (`Compiled::rust`).
    pub rust: Option<Result<String, String>>,
    /// The tree section's tables tier 2 as Rust reads (`bitlex_rust`).
    pub rust_tables: Option<RustTables>,
}

/// The tables `tree_section` finds for tier 2 as Rust (`bitlex_rust::Input`): each record kind's
/// name, each mark's bit in the code and the word it needs, and the bits that tell an extra.
pub struct RustTables {
    pub tokens: Vec<(u8, String)>,
    pub mark_bits: Vec<(Option<u8>, u16)>,
    pub extra_bits: Vec<(u8, String)>,
    /// Tier 1's fallback totals (`EMPTY`): the file goes to tree-sitter unless each is 0.
    pub empties: u16,
}


/// How a token ends: through a run of a mask (a `fill`), or at the first bit of its last-byte
/// mask (tier 1: no carry beyond a byte).
#[derive(Clone, Copy)]
pub enum TokenEnd {
    Through(Val),
    Last(Val),
}

/// A count: its up and down bytes (an up wins at a byte that is both), its bound and its planes
/// after each byte.
#[derive(Clone)]
pub struct Count {
    pub up: Val,
    pub down: Val,
    pub max: u32,
    pub planes: Vec<Val>,
}


/// What tier 2 makes of a tree section: the grammar's state machine as text, tier 2's words,
/// whether each token row takes its last byte, and each mark's mask.
pub type Tier2 = (String, String, Vec<bool>, Vec<Val>, crate::records::Records);

/// The grammar's ids for the tree section's rows.
pub struct Symbols<'a> {
    /// A node kind's id (named first, then anonymous), or a supertype's.
    pub kind: &'a dyn Fn(&str) -> Option<u16>,
    pub field: &'a dyn Fn(&str) -> Option<u16>,
}

/// Compile `stmts`, with `outputs` (mask names, or `FALLBACK`) as the block program's first
/// outputs, then the token rows' masks. `symbols` names the grammar's ids; a program
/// with a tree section needs it.
pub fn compile(stmts: &[Stmt], outputs: &[&str], symbols: Option<&Symbols<'_>>) -> Result<Compiled, String> {
    compile_shaped(stmts, outputs, symbols, Shape::Auto)
}

/// Tier 2's shape (docs/parser-generators/33-compiled-against-lean-shape.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// One state machine, its states merged wherever they act alike: simdjson's stage 2, the
    /// smallest code for a small grammar.
    Machine,
    /// A function a grammar rule (`grammar::build_shaped`): a call of the grammar is a native
    /// call, as a recursive-descent parser's (`Words::rules`).
    Rules,
    /// The machine while it has at most `MACHINE_STATES` states; else a function a rule.
    Auto,
}

/// The most states `Shape::Auto` keeps as one machine: JSON's size, about simdjson's.
pub const MACHINE_STATES: usize = 64;

/// `compile`, with tier 2 in `shape`.
pub fn compile_shaped(stmts: &[Stmt], outputs: &[&str], symbols: Option<&Symbols<'_>>, shape: Shape) -> Result<Compiled, String> {
    compile_proved(stmts, outputs, symbols, shape, HashMap::default())
}

/// `compile_shaped`, with the provers' answers a compile made before (`Compiled::proofs`,
/// decision 0488): a question among them is answered without its proof. The program is the same.
pub fn compile_proved(stmts: &[Stmt], outputs: &[&str], symbols: Option<&Symbols<'_>>, shape: Shape, known: bitlex_prove::Known) -> Result<Compiled, String> {
    let mut low = Lowering {
        proofs: bitlex_prove::Proofs { known, made: Vec::new() },
        shape,
        program: BlockProgram::default(),
        seen: HashMap::default(),
        table_products: Vec::new(),
        classes: HashMap::default(),
        names: HashMap::default(),
        fallbacks: Vec::new(),
        untaken: Vec::new(),
        counts: HashMap::default(),
        pending_counts: Vec::new(),
        pending_checks: Vec::new(),
        back_fallbacks: HashMap::default(),
        numbers: HashMap::default(),
        tokens: Vec::new(),
        token_rules: HashMap::default(),
        token_pairs: HashMap::default(),
        token_threes: HashMap::default(),
        token_reads: HashMap::default(),
        token_closer_last: HashMap::default(),
        token_parts: HashMap::default(),
        token_part_names: HashMap::default(),
        token_fills: HashMap::default(),
        read_fills: HashMap::default(),
        inside: Vec::new(),
        inside_of: Vec::new(),
        keywords: Vec::new(),
        skip: None,
        prefix: None,
        ahead: false,
        mask_exprs: HashMap::default(),
        tree: Vec::new(),
        extra: Vec::new(),
        marks: Vec::new(),
        mark_exprs: Vec::new(),
        token_marks: HashMap::default(),
        mark_next: HashMap::default(),
        unless: HashMap::default(),
        unplaced: Vec::new(),
        settles: Vec::new(),
        drops: HashMap::default(),
        loses: HashMap::default(),
        refuses: HashMap::default(),
        refuses_taken: HashMap::default(),
        ends: Vec::new(),
        passes: HashMap::default(),
        token_escapes: Vec::new(),
        tested_only: Vec::new(),
        mark_tests: Vec::new(),
        kind_checks: Vec::new(),
        hidden: Vec::new(),
        checks: Vec::new(),
        toggles: Vec::new(),
        soft: Vec::new(),
        costs: Vec::new(),
        rust: None,
        rust_tables: None,
    };
    // Every class's products first: the tables come before the operations.
    for stmt in stmts {
        if let Stmt::Class(name, ranges) = stmt {
            low.place(&products(ranges));
            low.classes.insert(name.clone(), ranges.clone());
        }
    }
    if let Some(bytes) = ::bitlex::syntax::prefix_of(stmts)? {
        let v = low.prefix_value(&bytes)?;
        low.prefix = Some((bytes, v));
    }
    low.stmts(stmts)?;
    let from = low.program.ops.len();
    let token_rows = low.tokens()?;
    low.cost("the token rows".into(), from);
    let from = low.program.ops.len();
    // The tree section: its state machine, its checks and tier 2's words (decision 0087), before
    // the outputs, since the words' guards join the fallback.
    let has_tree = !low.tree.is_empty();
    let mut tier2: Option<Tier2> = None;
    if has_tree {
        let symbols = symbols.ok_or("a tree section needs the grammar's ids")?;
        tier2 = Some(tree_section(symbols, &mut low, &token_rows)?);
    }
    // The checks left: the fallback's.
    for (a, l, not) in std::mem::take(&mut low.pending_checks) {
        let x = match low.back_fallbacks.get(&(a.v, l.v)) {
            Some(&x) if not => x,
            _ if not => low.andnot(a, l)?,
            _ => low.and(a, l)?,
        };
        low.fallbacks.push(x);
    }
    // The counts left: tier 1's, as bit-planes.
    for (name, up, down, max) in std::mem::take(&mut low.pending_counts) {
        let count = low.count(up, down, max)?;
        low.counts.insert(name, count);
    }
    let mut vals: Vec<(String, Val)> = Vec::new();
    // The words read the fallback's totals (`EMPTY`), so a tree's program always has them.
    if has_tree && !outputs.contains(&FALLBACK) {
        for x in low.fallback()? {
            vals.push((FALLBACK.to_owned(), x));
        }
    }
    for x in low.untaken.clone() {
        vals.push((FALLBACK_TOKENS.to_owned(), x));
    }
    for name in outputs {
        if *name == FALLBACK {
            for x in low.fallback()? {
                vals.push((FALLBACK.to_owned(), x));
            }
        } else {
            let x = low.name(name)?;
            vals.push(((*name).to_owned(), x));
        }
    }
    // The records tier 2 reads (`records`): every token's first byte and every token's last
    // byte, two extracts zipped by index, as simdjson's structural indexes.
    let mut stream = None;
    if let Some((_, _, _, mark_masks, records)) = &tier2 {
        // The rows tier 1 records (page 40): every row, but where tier 2 reads a region in parts
        // (`Split::Tier2`), its parts' rows (`Inside`) are tier 2's, found from the region's
        // opener, and tier 1 records the region's opener alone.
        let late = records.readers.iter().any(|rd| !rd.parts.is_empty() && rd.split == crate::records::Split::Tier2);
        let recorded: Vec<&(String, Val, Val)> = token_rows.iter().enumerate().filter(|&(r, _)| !(late && low.inside.get(r).copied().unwrap_or(false))).map(|(_, t)| t).collect();
        let mut starts = recorded[0].1;
        for (_, s, _) in &recorded[1..] {
            starts = low.or(starts, *s)?;
        }
        let mut lasts = recorded[0].2;
        for (_, _, l) in &recorded[1..] {
            lasts = low.or(lasts, *l)?;
        }
        vals.push((STREAM_START.to_owned(), starts));
        vals.push((STREAM_LAST.to_owned(), lasts));
        stream = Some((vals.len() - 2, vals.len() - 1));
        for &m in mark_masks {
            vals.push((STREAM_MARK.to_owned(), m));
        }
    }
    let mut delay = vals.iter().map(|x| x.1.d).max().unwrap_or(0);
    // Each output once, at one delay: its value's (`outputs`, the masks and the records find an
    // output by its value).
    let mut index: HashMap<u16, u16> = HashMap::default();
    let mut labels = Vec::new();
    let mut at = Vec::new();
    for (label, x) in &vals {
        // Outputs hold no bits past the end: a file falls back on a bit of its own. The stream's
        // need not: the extract pass reads only the file's bytes, on every executor.
        let stream_out = label == STREAM_START || label == STREAM_LAST || label == STREAM_MARK;
        let x = if x.dirty && !stream_out {
            let inside = low.within_val(x.d)?;
            low.and(*x, inside)?
        } else {
            *x
        };
        // One value at two delays (`x` and `prev(x)`, a `prev` of a delayed value being its
        // delay less one): the second its own value, a `prev` by one at a delay one more.
        let mut x = x;
        while let Some(&k) = index.get(&x.v) {
            if low.program.output_delays[usize::from(k)] == x.d {
                break;
            }
            if x.d >= MAX_DELAY {
                return Err("an output at two delays, the second past the largest".into());
            }
            x = Val { v: low.prev_op(x.v, 1), d: x.d + 1, ..x };
            delay = delay.max(x.d);
        }
        // At its own delay: the host shifts each output by its own (decision 0069).
        let v = x.v;
        let k = *index.entry(v).or_insert_with(|| {
            low.program.outputs.push(v);
            low.program.output_delays.push(x.d);
            (low.program.outputs.len() - 1) as u16
        });
        labels.push((label.clone(), k));
        at.push(low.program.outputs[usize::from(k)]);
    }
    let mut words = "  0000 STOP 0\n".to_owned();
    let mut machine = String::new();
    if let Some((text, lowered, _, mark_masks, records)) = tier2 {
        let (s, l) = stream.expect("a tree section's stream");
        low.program.extracts = vec![at[s], at[l]];
        low.program.records = Some(records);
        // Each mark's mask: `MARK`'s operand.
        low.program.masks = (0..mark_masks.len()).map(|k| at[l + 1 + k]).collect();
        if let Some(r) = &mut low.program.records {
            // Its output (`Records::marks`): where the mask's value is among the outputs.
            for m in r.marks.iter_mut().filter(|m| m.0 != crate::records::WORD_ALONE) {
                let v = low.program.masks[usize::from(m.0)];
                m.0 = low.program.outputs.iter().position(|&o| o == v).ok_or("a mark's mask not an output")? as u16;
            }
            // The skip's and each end rule's likewise.
            for k in r.skip.iter_mut().chain(r.gap.iter_mut()) {
                let v = low.program.masks[usize::from(*k)];
                *k = low.program.outputs.iter().position(|&o| o == v).ok_or("the skip's mask not an output")? as u16;
            }
            for rule in r.rules.iter_mut() {
                let (crate::records::EndRule::Run(k) | crate::records::EndRule::Pair(k) | crate::records::EndRule::Last(k)) = rule else { continue };
                let v = low.program.masks[usize::from(*k)];
                *k = low.program.outputs.iter().position(|&o| o == v).ok_or("an end rule's mask not an output")? as u16;
            }
            for t in r.retags.iter_mut() {
                let v = low.program.masks[usize::from(t.o)];
                t.o = low.program.outputs.iter().position(|&o| o == v).ok_or("a reader's start mask not an output")? as u16;
            }
            for a in r.readers.iter_mut().flat_map(|rd| rd.arms.iter_mut()) {
                for k in [&mut a.opener, &mut a.closer].into_iter().chain(a.last.as_mut()) {
                    let v = low.program.masks[usize::from(*k)];
                    *k = low.program.outputs.iter().position(|&o| o == v).ok_or("a reader's mask not an output")? as u16;
                }
            }
            for p in r.readers.iter_mut().flat_map(|rd| rd.parts.iter_mut()) {
                let v = low.program.masks[usize::from(p.start)];
                p.start = low.program.outputs.iter().position(|&o| o == v).ok_or("a part's start mask not an output")? as u16;
                if let Some(l) = &mut p.last {
                    let v = low.program.masks[usize::from(*l)];
                    *l = low.program.outputs.iter().position(|&o| o == v).ok_or("a part's last mask not an output")? as u16;
                }
            }
            for c in r.checks.iter_mut() {
                let v = low.program.masks[usize::from(c.o)];
                c.o = low.program.outputs.iter().position(|&o| o == v).ok_or("a token test's mask not an output")? as u16;
            }
            for b in r.blanks.iter_mut() {
                let v = low.program.masks[usize::from(b.1)];
                b.1 = low.program.outputs.iter().position(|&o| o == v).ok_or("a blank mark's class not an output")? as u16;
            }
        }
        // The fallback's outputs, one per delay, are the totals: one flag, which `EMPTY 0` reads.
        low.program.totals = vals.iter().zip(&at).filter(|((label, _), _)| label == FALLBACK || label == FALLBACK_TOKENS).map(|(_, &v)| v).collect();
        // The token loop's errors' outputs (`Records::untaken`).
        let untaken: Vec<u16> = vals.iter().zip(&at).filter(|((label, _), _)| label == FALLBACK_TOKENS).map(|(_, &v)| v).collect();
        if let Some(r) = &mut low.program.records {
            r.untaken = untaken.iter().map(|v| low.program.outputs.iter().position(|o| o == v).map(|k| k as u16).ok_or("an error by tokens not an output")).collect::<Result<_, _>>()?;
        }
        words = lowered;
        machine = text;
        // Tier 2 as Rust, from the tree rules and the tables (`bitlex_rust`), the records' outputs the program's now.
        if let (Some(t), Some(r), Some(symbols)) = (&low.rust_tables, &low.program.records, symbols) {
            let marks = (0..low.marks.len())
                .map(|i| {
                    let (name, _, end) = &low.marks[i];
                    crate::bitlex_rust::Mark {
                        name: name.clone(),
                        bit: t.mark_bits[i].0,
                        word: t.mark_bits[i].1,
                        end: *end,
                        tokens: low.token_marks.get(&i).map(|(b, s)| (b.as_slice(), s.as_slice())),
                        next: low.mark_next.get(&i).map_or(&[][..], |v| v.as_slice()),
                        unless_after: low.unless.get(&i).map_or(&[][..], |v| v.as_slice()),
                        ends: low.ends.contains(&i),
                        settles: low.settles.contains(&i),
                        hidden: low.hidden.contains(name),
                        passes: low.passes.get(&i).into_iter().flatten().filter_map(|n| low.marks.iter().position(|x| &x.0 == n)).fold(0, |a, m| a | 1 << m),
                        drops: low.drops.get(&i).map_or(&[][..], |v| v.as_slice()),
                        loses: low.loses.get(&i).map_or(&[][..], |v| v.as_slice()),
                        refuses: low.refuses.get(&i).map_or(&[][..], |v| v.as_slice()),
                        refuses_taken: low.refuses_taken.get(&i).map_or(&[][..], |v| v.as_slice()),
                    }
                })
                .collect();
            let x = crate::bitlex_rust::Input { tree: &low.tree, kind: symbols.kind, field: symbols.field, tokens: &t.tokens, words: &r.words, keywords: &low.keywords, soft: &low.soft, marks, extras: &low.extra, extra_bits: t.extra_bits.clone(), empties: t.empties, readers: &r.readers, kinds: &r.kinds };
            let rust = crate::bitlex_rust::tier2(&x);
            low.rust = Some(rust);
        }
    }
    low.program.delay = delay;
    low.cost("the tree section's checks and the outputs".into(), from);
    // Only the operations kept count: a rounds group's first tries are dead.
    let keep = kept(&low.program);
    let costs = low
        .costs
        .iter()
        .map(|(what, from, to)| {
            let live = (*from..*to).filter(|&k| keep[k]);
            let back_fills = live.clone().filter(|&k| matches!(low.program.ops[k], BlockOp::BackFill(..))).count() as u32;
            Cost { what: what.clone(), ops: live.count() as u32, back_fills }
        })
        .collect();
    let program = live(&low.program);
    // Each kept operation's statement, at its place in the live program.
    let mut origin = vec![String::new(); program.ops.len()];
    let mut place = vec![usize::MAX; keep.len()];
    let mut next = 0;
    for (k, &kept) in keep.iter().enumerate() {
        if kept {
            place[k] = next;
            next += 1;
        }
    }
    for (what, from, to) in &low.costs {
        for k in (*from..*to).filter(|&k| keep[k]) {
            if origin[place[k]].is_empty() {
                origin[place[k]] = what.clone();
            }
        }
    }
    let mut named: Vec<(String, Val)> = low.names.iter().map(|(n, &x)| (n.clone(), x)).collect();
    named.sort_by(|a, b| a.0.cmp(&b.0));
    named.extend(low.marks.iter().map(|(kind, x, _)| (format!("mark {kind}"), *x)));
    for (kind, start, last) in &token_rows {
        named.push((format!("token {kind} start"), *start));
        named.push((format!("token {kind} last"), *last));
    }
    let values = named
        .into_iter()
        .filter(|(_, x)| keep[usize::from(x.v)])
        .map(|(name, x)| Named { name, op: place[usize::from(x.v)] as u16, delay: x.d })
        .collect();
    let fallbacks = labels.iter().filter(|(l, _)| l == FALLBACK).map(|&(_, k)| k).collect();
    let proofs = std::mem::take(&mut low.proofs.made);
    let rust = low.rust.take();
    Ok(Compiled { rows: rows(&program), program, words, machine, labels, fallbacks, costs, origin, values, proofs, rust })
}

/// The operations the provers read: the prefix's value zero (it is stripped before the passes,
/// 0389).
fn proof_ops<'a>(program: &'a BlockProgram, prefix: &Option<(Vec<u8>, Val)>) -> std::borrow::Cow<'a, [BlockOp]> {
    match prefix {
        Some((_, p)) => {
            let mut ops = program.ops.clone();
            ops[usize::from(p.v)] = BlockOp::Const(0);
            std::borrow::Cow::Owned(ops)
        }
        None => std::borrow::Cow::Borrowed(&program.ops),
    }
}

impl Lowering {
    /// The operations added since `from`, as a cost of `what` (`Compiled::costs`).
    pub fn cost(&mut self, what: String, from: usize) {
        let to = self.program.ops.len();
        if to > from {
            self.costs.push((what, from, to));
        }
    }

    pub fn stmts(&mut self, stmts: &[Stmt]) -> Result<(), String> {
        for stmt in stmts {
            let from = self.program.ops.len();
            // A tree and its rounds are costed by their statements.
            let nested = matches!(stmt, Stmt::Tree { .. } | Stmt::Rounds { .. });
            self.stmt(stmt)?;
            if !nested {
                self.cost(cost_name(stmt), from);
            }
        }
        Ok(())
    }

    pub fn stmt(&mut self, stmt: &Stmt) -> Result<(), String> {
        {
            match stmt {
                Stmt::Lexer(_) | Stmt::Class(..) => {}
                Stmt::Mask(name, expr) => {
                    let value = self.expr(expr)?;
                    self.names.insert(name.clone(), value);
                    if self.prefix.is_some() {
                        self.mask_exprs.insert(name.clone(), expr.clone());
                    }
                }
                // The token loop's error (decision 0177): its mask an output of its own, among the
                // totals; the CPU's loop refuses its bytes instead.
                Stmt::TokenError(_, expr) => {
                    let value = self.expr(expr)?;
                    self.untaken.push(value);
                }
                // A token test (decision 0177): the token loop's, at its kind's end rule.
                Stmt::TokenEscape { kind, test, expr, least, .. } => {
                    let count = *test == ::bitlex::syntax::TokenTest::Count;
                    match self.local_split(count, *least, expr)? {
                        Some((a, l)) => self.kind_checks.push((kind.clone(), None, a, l)),
                        None => {
                            let value = self.expr(expr)?;
                            self.token_escapes.push((kind.clone(), count, value, *least, crate::records::CheckThen::Escape));
                        }
                    }
                }
                // Both send the file to tree-sitter.
                Stmt::Error(_, expr) | Stmt::Escape(_, expr) => {
                    // Worth a check of its own only where `L` is more than a test or two.
                    let local = |low: &Self, x: Val| low.local_ops(x).is_some_and(|ops| ops.len() >= 8);
                    // `A & ~L`: `L`'s own ops, then its `not` (decision 0399). The `~` as a mask
                    // is cut to the stream (`within`), which a check at `A`'s bytes needs not.
                    let negated = match expr {
                        Expr::And(a, b) => match &**b {
                            Expr::Not(l) => Some((a, l)),
                            _ => None,
                        },
                        _ => None,
                    };
                    if let Some((a, l)) = negated
                        && let (av, lv) = (self.expr(a)?, self.expr(l)?)
                        && self.local_ops_in_stream(lv).is_some_and(|ops| ops.len() + 1 >= 8)
                    {
                        self.pending_checks.push((av, lv, true));
                    } else if let Some((a, l)) = negated
                        && let (av, lv) = (self.expr(a)?, self.expr(l)?)
                        && self.local_ops_in_stream(lv).is_none()
                        && !local(self, av)
                    {
                        // `L` not local: perhaps it reads the tokens before `A`'s, a check that
                        // reads them back (`token_ops_back`, decision 0428), decided where the
                        // rows are known. Else the fallback's, as it was.
                        let nl = self.expr(&Expr::Not(l.clone()))?;
                        let value = self.and(av, nl)?;
                        self.back_fallbacks.insert((av.v, lv.v), value);
                        self.pending_checks.push((av, lv, true));
                    } else if let Expr::And(a, b) = expr {
                        let (a, b) = (self.expr(a)?, self.expr(b)?);
                        if local(self, b) {
                            self.pending_checks.push((a, b, false));
                        } else if local(self, a) {
                            self.pending_checks.push((b, a, false));
                        } else {
                            let value = self.and(a, b)?;
                            self.fallbacks.push(value);
                        }
                    } else {
                        let value = self.expr(expr)?;
                        self.fallbacks.push(value);
                    }
                }
                Stmt::Number(name, e) => {
                    let planes = self.number(e)?;
                    self.numbers.insert(name.clone(), planes);
                }
                Stmt::Count { name, up, down, reset, max } => {
                    if reset.is_some() {
                        return Err(format!("count {name}: `reset` is not compiled yet"));
                    }
                    let (up, down) = (self.expr(up)?, self.expr(down)?);
                    self.pending_counts.push((name.clone(), up, down, *max));
                }
                Stmt::Token { kind, start, through, last, rule } => {
                    match rule {
                        // A region read in parts: its reader's arm, and each part's start and
                        // bytes; the closer and the parts are rows of their own (`Inside`).
                        Some(::bitlex::syntax::TokenRule::ReadParts(rs, parts)) => {
                            // One region: its opener and closer. Several (`read K | L parts`):
                            // one arm, at their openers taken (`o_K`), to each one's closer
                            // outside the others' fills (a `"` in a raw string closes no
                            // string), each closer's last by `closer_last`. The first closer
                            // after the opener is its own region's: the bytes before it are
                            // its fill's.
                            let (opener, closer) = match rs.as_slice() {
                                [r] => (self.expr(&r.opener)?, self.expr(&r.closers())?),
                                _ => {
                                    let name = |x: String| Expr::Name(x);
                                    let or = |a: Expr, b: Expr| Expr::Or(Box::new(a), Box::new(b));
                                    let o = rs.iter().map(|r| name(format!("o_{}", r.kind))).reduce(or).expect("a region");
                                    let c = rs
                                        .iter()
                                        .map(|r| {
                                            rs.iter().filter(|q| q.kind != r.kind).fold(r.closers(), |c, q| Expr::And(Box::new(c), Box::new(Expr::Not(Box::new(name(format!("f_{}", q.kind)))))))
                                        })
                                        .reduce(or)
                                        .expect("a region");
                                    (self.expr(&o)?, self.expr(&c)?)
                                }
                            };
                            self.token_reads.insert(kind.clone(), vec![(opener, closer, rs[0].bytes as i8 - 1, crate::records::ReadNone::Refuse)]);
                            if let Some(l) = parts.closer_last(rs) {
                                let l = self.expr(&l)?;
                                self.token_closer_last.insert(kind.clone(), l);
                            }
                            let mut ps = Vec::new();
                            for (_, e, len) in &parts.parts {
                                let start = self.expr(e)?;
                                ps.push(match len {
                                    ::bitlex::syntax::PartLen::Bytes(n) => (start, *n, None),
                                    ::bitlex::syntax::PartLen::Last(l) => (start, 0, Some(self.expr(l)?)),
                                });
                            }
                            self.token_parts.insert(kind.clone(), ps);
                            self.token_part_names.insert(kind.clone(), (parts.parts.iter().map(|p| p.0.clone()).collect(), parts.content.clone()));
                            let fill = rs.iter().map(|r| Expr::Name(format!("f_{}", r.kind))).reduce(|a, b| Expr::Or(Box::new(a), Box::new(b))).expect("a region");
                            let fill = self.expr(&fill)?;
                            self.token_fills.insert(kind.clone(), fill);
                        }
                        Some(::bitlex::syntax::TokenRule::Inside(..)) => {}
                        Some(::bitlex::syntax::TokenRule::Read(regions)) => {
                            let mut arms = Vec::new();
                            for r in regions {
                                if r.or.is_some() {
                                    return Err(format!("token {kind}: a `read` of region {} with a second closer: read it in parts", r.kind));
                                }
                                let (opener, closer) = (self.expr(&r.opener)?, self.expr(&r.closer)?);
                                let off = if r.outside { -1 } else { r.bytes as i8 - 1 };
                                let none = match (&r.empty, r.outside) {
                                    (Some((_, m)), _) => crate::records::ReadNone::Bytes(*m),
                                    (None, true) => crate::records::ReadNone::End,
                                    (None, false) => crate::records::ReadNone::Refuse,
                                };
                                arms.push((opener, closer, off, none));
                            }
                            self.token_reads.insert(kind.clone(), arms);
                            let fill = regions.iter().map(|r| Expr::Name(format!("f_{}", r.kind))).reduce(|a, b| Expr::Or(Box::new(a), Box::new(b))).expect("a region");
                            let fill = self.expr(&fill)?;
                            self.read_fills.insert(kind.clone(), fill);
                        }
                        Some(r) => {
                            let (pair, e) = match r {
                                ::bitlex::syntax::TokenRule::Run(e) => (false, e),
                                // `pair empty`: no byte pairs, a table of none (`EndRule::Next`), as
                                // `pair P` with no spellings. Its tokens are one byte, read from the
                                // first byte's entry (the fused pair rule's), not a mask's word that is
                                // always zero, by a call a token (0355).
                                ::bitlex::syntax::TokenRule::Pair(e @ ::bitlex::syntax::Expr::Empty) => {
                                    self.token_pairs.insert(kind.clone(), Vec::new());
                                    (true, e)
                                }
                                ::bitlex::syntax::TokenRule::Pair(e) => (true, e),
                                ::bitlex::syntax::TokenRule::Pairs { pairs, mask, three } => {
                                    self.token_pairs.insert(kind.clone(), pairs.clone());
                                    if let Some(t) = three {
                                        let v = self.ahead(t)?;
                                        self.token_threes.insert(kind.clone(), v);
                                    }
                                    (true, mask)
                                }
                                ::bitlex::syntax::TokenRule::Read(_) | ::bitlex::syntax::TokenRule::ReadParts(..) | ::bitlex::syntax::TokenRule::Inside(..) => unreachable!(),
                            };
                            let v = self.ahead(e)?;
                            self.token_rules.insert(kind.clone(), (pair, v));
                        }
                        None => {}
                    }
                    let start = self.expr(start)?;
                    let through = match through {
                        Some(t) => Some(TokenEnd::Through(self.expr(t)?)),
                        None => match last {
                            Some(l) => Some(TokenEnd::Last(self.expr(l)?)),
                            None => None,
                        },
                    };
                    self.tokens.push((kind.clone(), start, through));
                    self.inside.push(matches!(rule, Some(::bitlex::syntax::TokenRule::Inside(..))));
                    self.inside_of.push(match rule {
                        Some(::bitlex::syntax::TokenRule::Inside(_, reader, closes)) => Some((reader.clone(), *closes)),
                        _ => None,
                    });
                }
                Stmt::Keywords { kind, words, soft, required } => {
                    self.keywords.push((kind.clone(), words.clone(), *required));
                    if !soft.is_empty() {
                        self.soft.push((kind.clone(), soft.clone()));
                    }
                }
                // Made first (`prefix_value`).
                Stmt::Prefix(_) => {}
                Stmt::Skip(expr) => {
                    let x = self.expr(expr)?;
                    self.skip = Some(match self.skip {
                        Some(s) => self.or(s, x)?,
                        None => x,
                    });
                }
                Stmt::Tree { body, .. } => self.stmts(body)?,
                // The grammar's rules: the state machine is built once the token rows are known.
                Stmt::Node { .. } | Stmt::Choice { .. } | Stmt::Refuse { .. } | Stmt::Reading { .. } | Stmt::Provisional(_) | Stmt::Remember(_) | Stmt::Flags(_) => self.tree.push(stmt.clone()),
                // Applied by the parser: each token it names marked `A as IMPLIED` in its rule.
                Stmt::Implied { .. } => {}
                Stmt::Check { kind, body } => self.checks.push((kind.clone(), body.clone())),
                Stmt::Extra(kinds) => self.extra.extend(kinds.iter().cloned()),
                Stmt::Hidden(kinds) => self.hidden.extend(kinds.iter().cloned()),
                Stmt::Mark { kind, at, end, unless, unplaced, checks, settles, ends, next, drops, loses, refuses, refuses_taken, passes } => {
                    if !passes.is_empty() {
                        self.passes.insert(self.marks.len(), passes.clone());
                    }
                    if !refuses_taken.is_empty() {
                        self.refuses_taken.insert(self.marks.len(), refuses_taken.clone());
                    }
                    if !refuses.is_empty() {
                        self.refuses.insert(self.marks.len(), refuses.clone());
                    }
                    if !drops.is_empty() {
                        self.drops.insert(self.marks.len(), drops.clone());
                    }
                    if !loses.is_empty() {
                        self.loses.insert(self.marks.len(), loses.clone());
                    }
                    if !next.is_empty() {
                        self.mark_next.insert(self.marks.len(), next.clone());
                    }
                    if *settles {
                        self.settles.push(self.marks.len());
                    }
                    if *ends {
                        self.ends.push(self.marks.len());
                    }
                    // Its token tests (decision 0177): the token loop's, each a bit of the mark.
                    let k = self.marks.len();
                    for c in checks {
                        if !unless.is_empty() {
                            return Err(format!("mark {kind}: token tests and `unless after`"));
                        }
                        self.mark_tests.push((k, c.after, c.kind.clone(), c.test == ::bitlex::syntax::TokenTest::Count, c.least, c.expr.clone()));
                    }
                    if !checks.is_empty() && matches!(at, Expr::Empty) {
                        self.tested_only.push(k);
                    }
                    if !unless.is_empty() {
                        self.unless.insert(self.marks.len(), unless.clone());
                    }
                    if unplaced.is_some() {
                        self.unplaced.push(self.marks.len());
                    }
                    let x = self.expr(at)?;
                    self.marks.push((kind.clone(), x, *end));
                    self.mark_exprs.push(at.clone());
                }
                Stmt::TokenMark { kind, before, seq, next } => {
                    self.token_marks.insert(self.marks.len(), (before.clone(), seq.clone()));
                    // Its `unless before`s, entries of its order as a mark's (`Mark::nexts`).
                    if !next.is_empty() {
                        self.mark_next.insert(self.marks.len(), next.clone());
                    }
                    let none = self.leaf(BlockOp::Const(0), false);
                    self.marks.push((kind.clone(), none, false));
                    self.mark_exprs.push(Expr::Empty);
                }
                Stmt::Rounds { n: None, regions: Some(regions), .. } => {
                    let (from, costs) = (self.program.ops.len(), self.costs.len());
                    self.regions(regions)?;
                    self.costs.truncate(costs);
                    self.cost(format!("regions {}", regions.hidden), from);
                }
                Stmt::Rounds { n: None, body, .. } => {
                    ::bitlex::syntax::strictly_causal(body)?;
                    // One cost for the group: its body is lowered once per try.
                    let (from, costs) = (self.program.ops.len(), self.costs.len());
                    self.rounds(body)?;
                    self.costs.truncate(costs);
                    self.cost("rounds until settled (every try)".into(), from);
                }
                Stmt::Rounds { n: Some(n), .. } => return Err(format!("`rounds {n}` is not compiled: only `rounds until settled`, whose fixed point is exact")),
                other => return Err(format!("not compiled yet: {}", statement(other))),
            }
        }
        Ok(())
    }

    /// `rounds until settled { … }` (`docs/parser-generators/03-bitlex.md` §2.7): a group of the
    /// block program (`BlockProgram::rounds`), each carry a `carry` of its initial value, paired
    /// with its next. A carry and its next are one word, so they are at one delay: the carry's,
    /// with the next brought to it by `prev`. A next later than its carry (an operand outside the
    /// group later still) or dirty where the carry is clean lowers the body again with the carry
    /// at that delay or dirty; the first try's operations are dead (`live`). A strictly causal
    /// body (`strictly_causal`) has a `prev` on every way from a carry to a next, so each way
    /// still holds one once the delays are evened out: the block verifier checks it.
    pub fn rounds(&mut self, body: &[Stmt]) -> Result<(), String> {
        let carries: Vec<(&String, &Expr)> = body.iter().filter_map(|s| if let Stmt::Carry(a, e) = s { Some((a, e)) } else { None }).collect();
        let nexts: Vec<(&String, &Expr)> = body.iter().filter_map(|s| if let Stmt::Next(a, e) = s { Some((a, e)) } else { None }).collect();
        if let Some((name, _)) = nexts.iter().find(|(n, _)| !carries.iter().any(|(c, _)| c == n)) {
            return Err(format!("next {name}: no carry of that name"));
        }
        if carries.is_empty() {
            return Err("rounds with no carry".into());
        }
        let inits: Vec<Val> = carries.iter().map(|(_, e)| self.expr(e)).collect::<Result<_, _>>()?;
        let mut delay: Vec<u8> = inits.iter().map(|x| x.d).collect();
        let mut dirty: Vec<bool> = inits.iter().map(|x| x.dirty).collect();
        let rest: Vec<Stmt> = body.iter().filter(|s| !matches!(s, Stmt::Carry(..) | Stmt::Next(..))).cloned().collect();
        // Each try raises a delay or makes a carry dirty, and neither falls: a bounded number.
        for _ in 0..=usize::from(MAX_DELAY) + 2 {
            let mut vs = Vec::new();
            for (k, (name, _)) in carries.iter().enumerate() {
                // Zero is zero at any delay.
                let init = if self.is_zero(inits[k]) { inits[k].v } else { self.at(inits[k], delay[k])? };
                // Never shared: each carry is its own value, from try to try.
                let v = self.program.ops.len() as u16;
                self.program.ops.push(BlockOp::Carry(init));
                vs.push(v);
                self.names.insert((*name).clone(), Val { v, d: delay[k], dirty: dirty[k], set: None });
            }
            self.stmts(&rest)?;
            let mut pairs = Vec::new();
            let mut again = false;
            for (k, (name, _)) in carries.iter().enumerate() {
                let Some((_, e)) = nexts.iter().find(|(n, _)| n == name) else {
                    return Err(format!("carry {name}: no next"));
                };
                let x = self.expr(e)?;
                if x.d > delay[k] || (x.dirty && !dirty[k]) {
                    delay[k] = delay[k].max(x.d);
                    dirty[k] |= x.dirty;
                    again = true;
                } else {
                    pairs.push((vs[k], self.at(x, delay[k])?));
                }
            }
            if !again {
                self.program.rounds.push(pairs);
                return Ok(());
            }
        }
        Err("rounds: the carries' delays do not settle".into())
    }

    /// `regions H { … }` (`::bitlex::syntax::Regions`, decision 0139): a rounds group whose
    /// every value is at one delay `D`, the largest of its openers' and closers' (one more with
    /// an `else`), so a word's bit is one byte throughout and the group is a walk
    /// (`block::Walk`) as well as a fixed point. Each opener and closer brought to `D` and cut to
    /// the stream; the carry `H`; per kind `o_K = O & ~H & ~(an earlier kind's O)` (a byte two
    /// kinds open is the first's, as the walk takes it), `f_K = fill(o_K, C)`, and the hidden
    /// bytes after them by `prev`s at `D`. An `else`'s "a closer after the byte" is the closer's
    /// `back_fill` at `D - 1`: the same bits, a byte on.
    pub fn regions(&mut self, regions: &::bitlex::syntax::Regions) -> Result<(), String> {
        let mut inputs = Vec::new();
        let mut seconds = Vec::new();
        for r in &regions.kinds {
            inputs.push((self.expr(&r.opener)?, self.expr(&r.closer)?));
            seconds.push(match &r.or {
                Some((c2, m)) => Some((self.expr(c2)?, *m)),
                None => None,
            });
        }
        let later = regions.kinds.iter().any(|r| r.empty.is_some());
        let d = inputs.iter().flat_map(|(o, c)| [o.d, c.d]).chain(seconds.iter().flatten().map(|(c, _)| c.d)).max().unwrap_or(0) + u8::from(later);
        let zero = self.op(BlockOp::Const(0));
        let w = self.within(d)?;
        let clean = |me: &mut Self, x: Val, d: u8| -> Result<u16, String> {
            let v = me.at(x, d)?;
            let w = me.within(d)?;
            Ok(me.op(BlockOp::And(v, w)))
        };
        // One kind whose opener is its closer, one byte, with no `else`: simdjson's string,
        // `prefix_xor(quote)`. Its fill is the toggle of its quotes, with no fixed point and no
        // walk: by induction, with `t = toggle(q)` and the fill `t` a byte before, the hidden
        // byte is `prev(t)`, the opener `q & ~prev(t)`, and the fill
        // `open | (prev(t) & ~q) = q ^ prev(t) = t`.
        if let ([r], [(o, c)]) = (regions.kinds.as_slice(), inputs.as_slice()) {
            if r.empty.is_none() && r.or.is_none() && !r.outside && r.bytes == 1 && (o.v, o.d) == (c.v, c.d) {
                let q = Val { v: clean(self, *o, d)?, d, dirty: false, set: None };
                let fill = self.carried(q, BlockOp::Toggle);
                let hidden = self.op(BlockOp::Prev(fill.v, 1));
                let open = self.op(BlockOp::AndNot(q.v, hidden));
                for (name, v) in [(regions.hidden.clone(), hidden), (format!("o_{}", r.kind), open), (format!("f_{}", r.kind), fill.v)] {
                    self.names.insert(name, Val { v, d, dirty: true, set: None });
                }
                // A mode of tier 2's states, as a toggle is (`row_modes`).
                self.toggles.push((fill, q));
                return Ok(());
            }
        }
        // Each kind's opener and closer at `D`; with `or C2`, the closer `C | C2` and `C2`.
        let mut ins = Vec::new();
        for (&(o, c), second) in inputs.iter().zip(&seconds) {
            let later = if later { Some(clean(self, c, d - 1)?) } else { None };
            let (mut closer, mut or) = (clean(self, c, d)?, None);
            if let Some((c2, m)) = *second {
                let c2 = clean(self, c2, d)?;
                closer = self.op(BlockOp::Or(closer, c2));
                or = Some((c2, m));
            }
            ins.push((clean(self, o, d)?, closer, later, or));
        }
        let hidden = self.program.ops.len() as u16;
        self.program.ops.push(BlockOp::Carry(zero));
        let code = self.op(BlockOp::AndNot(w, hidden));
        let mut hide: Vec<u16> = Vec::new();
        let mut kinds = Vec::new();
        // The earlier kinds' openers: a byte one of them opens is not a later kind's.
        let mut earlier: Option<u16> = None;
        let named = |me: &mut Self, name: String, v: u16| {
            me.names.insert(name, Val { v, d, dirty: true, set: None });
        };
        // In the walk's order, which is the regions' (`after` kinds last, verified): a kind with
        // `after` reads the plain kinds' openers taken. Where two kinds open a byte the walk opens
        // the first listed of those that may open there, so each kind's opener excludes the
        // earlier kinds' taken (an `after` kind's with its condition: e3927f1d7, 0332 step 35).
        let mut opens: Vec<u16> = Vec::with_capacity(regions.kinds.len());
        for (at, (r, &(opener, closer, c_before, or))) in regions.kinds.iter().zip(&ins).enumerate() {
            let mut taken = self.op(BlockOp::And(opener, code));
            let mut after = 0u16;
            if !r.after.is_empty() {
                // Taken where the last plain kind's opener taken before is one `after` names.
                let mut named = None;
                let mut others = None;
                for (k, q) in regions.kinds[..at].iter().enumerate().filter(|(_, q)| q.after.is_empty()) {
                    let o = opens[k];
                    let side = if r.after.contains(&q.kind) {
                        after |= 1 << k;
                        &mut named
                    } else {
                        &mut others
                    };
                    *side = Some(match *side {
                        Some(x) => self.op(BlockOp::Or(x, o)),
                        None => o,
                    });
                }
                let last = self.op(BlockOp::Fill(named.expect("verified: a kind named"), others.unwrap_or(zero)));
                let before = self.op(BlockOp::Prev(last, 1));
                taken = self.op(BlockOp::And(taken, before));
            }
            if let Some(e) = earlier {
                taken = self.op(BlockOp::AndNot(taken, e));
            }
            earlier = Some(match earlier {
                Some(e) => self.op(BlockOp::Or(e, taken)),
                None => taken,
            });
            let (open, empty) = match &r.empty {
                Some((e, m)) => {
                    let later = self.op(BlockOp::BackFill(c_before.expect("made with an else"), zero));
                    let open = self.op(BlockOp::And(taken, later));
                    let ev = self.op(BlockOp::AndNot(taken, later));
                    named(self, e.clone(), ev);
                    for k in 1..*m {
                        hide.push(self.op(BlockOp::Prev(ev, k)));
                    }
                    (open, Some((ev, *m)))
                }
                None => (taken, None),
            };
            opens.push(open);
            let fill = self.op(BlockOp::Fill(open, closer));
            if r.outside {
                let p = self.op(BlockOp::Prev(fill, 1));
                hide.push(self.op(BlockOp::AndNot(p, closer)));
            } else if let Some((c2, m)) = or {
                // From the closer's first byte (`c`): `N - 1` bytes more, `M - 1` at a `C2`.
                let p = self.op(BlockOp::Prev(fill, 1));
                hide.push(p);
                let c = self.op(BlockOp::AndNot(p, fill));
                let (one, two) = (self.op(BlockOp::AndNot(c, c2)), self.op(BlockOp::And(c, c2)));
                for k in 1..r.bytes {
                    hide.push(self.op(BlockOp::Prev(one, k)));
                }
                for k in 1..m {
                    hide.push(self.op(BlockOp::Prev(two, k)));
                }
            } else {
                for k in 1..=r.bytes {
                    hide.push(self.op(BlockOp::Prev(fill, k)));
                }
            }
            named(self, format!("o_{}", r.kind), open);
            named(self, format!("f_{}", r.kind), fill);
            // A region is a mode as a toggle is: `f` the mode, `f ^ prev(f)` its flips, the
            // opener's first byte and the closer's (`row_modes`).
            let f = Val { v: fill, d, dirty: true, set: None };
            let before = self.prev(f, 1)?;
            let flips = self.xor(f, before)?;
            self.toggles.push((f, flips));
            kinds.push(crate::block::WalkKind { opener, closer, open, fill, bytes: r.bytes, outside: r.outside, empty, or, after });
        }
        let mut next = *hide.first().ok_or("regions that hide nothing")?;
        for &h in &hide[1..] {
            next = self.op(BlockOp::Or(next, h));
        }
        named(self, regions.hidden.clone(), hidden);
        self.program.rounds.push(vec![(hidden, next)]);
        self.program.walks.push(crate::block::Walk { group: (self.program.rounds.len() - 1) as u16, hidden, kinds });
        Ok(())
    }

    /// The fallback masks, ORed per delay (`empty` without any): a mask at one delay is never
    /// brought to another, since the fallback asks only whether any bit is set.
    pub fn fallback(&mut self) -> Result<Vec<Val>, String> {
        let mut by_delay: Vec<Val> = Vec::new();
        for x in self.fallbacks.clone() {
            match by_delay.iter_mut().find(|y| y.d == x.d) {
                Some(y) => *y = self.or(*y, x)?,
                None => by_delay.push(x),
            }
        }
        if by_delay.is_empty() {
            by_delay.push(self.leaf(BlockOp::Const(0), false));
        }
        by_delay.sort_by_key(|x| x.d);
        Ok(by_delay)
    }

    /// A class's products placed together, and the bits of each table it reads: a table that
    /// holds them all, else the one with room for the fewest missing, else a new one. So a
    /// class of eight products or fewer is one lookup, as each of simdjson's classes is, though
    /// a product may then be in two tables. More than eight: eight to a table.
    pub fn place(&mut self, ps: &[Product]) -> Vec<(u16, u8)> {
        if ps.len() > 8 {
            return ps.chunks(8).flat_map(|c| self.place(c)).collect();
        }
        let missing = |held: &Vec<Product>| ps.iter().filter(|p| !held.contains(p)).count();
        let room = self.table_products.iter().enumerate().filter(|(_, held)| held.len() + missing(held) <= 8).min_by_key(|(t, held)| (missing(held), *t)).map(|(t, _)| t);
        let t = match room {
            Some(t) => t,
            None => {
                self.table_products.push(Vec::new());
                self.program.tables.push(ClassTable::default());
                self.table_products.len() - 1
            }
        };
        let mut bits = 0u8;
        for &p in ps {
            let bit = match self.table_products[t].iter().position(|&q| q == p) {
                Some(bit) => bit,
                None => {
                    self.table_products[t].push(p);
                    let (bit, table) = (self.table_products[t].len() - 1, &mut self.program.tables[t]);
                    for h in 0..16 {
                        if p.0 >> h & 1 == 1 {
                            table.hi[h] |= 1 << bit;
                        }
                        if p.1 >> h & 1 == 1 {
                            table.lo[h] |= 1 << bit;
                        }
                    }
                    bit
                }
            };
            bits |= 1 << bit;
        }
        vec![(t as u16, bits)]
    }

    pub fn op(&mut self, op: BlockOp) -> u16 {
        if let Some(&v) = self.seen.get(&op) {
            return v;
        }
        let v = self.program.ops.len() as u16;
        self.program.ops.push(op);
        self.seen.insert(op, v);
        v
    }

    /// An operation on bytes or constants: delay 0.
    pub fn leaf(&mut self, op: BlockOp, dirty: bool) -> Val {
        Val { v: self.op(op), d: 0, dirty, set: None }
    }

    /// `x`'s value brought to delay `d` (at least `x.d`), by `prev`s of at most 63 bytes.
    pub fn at(&mut self, x: Val, d: u8) -> Result<u16, String> {
        if d > MAX_DELAY {
            return Err(format!("a delay of {d} bytes: more than {MAX_DELAY} (`next` reaches too far)"));
        }
        let mut v = x.v;
        let mut left = d - x.d;
        while left > 0 {
            let k = left.min(63);
            v = self.prev_op(v, k);
            left -= k;
        }
        Ok(v)
    }

    /// `prev(v, k)`, folded into `v`'s own `prev` when it has one: `prev(prev(x, a), k)` is
    /// `prev(x, a + k)`, on `x`'s carried word.
    pub fn prev_op(&mut self, v: u16, k: u8) -> u16 {
        match self.program.ops[usize::from(v)] {
            BlockOp::Prev(x, a) if a + k <= MAX_DELAY => self.op(BlockOp::Prev(x, a + k)),
            _ => self.op(BlockOp::Prev(v, k)),
        }
    }

    /// `x` at delay `d`, below its own, when every bit of it lies in a file's first
    /// `block::EARLY_BITS` bits (`first_bits`): the same bytes' bits, `x.d - d` earlier in the
    /// word (`BlockOp::Early`). So a test of a file's start that looks ahead, as a byte order
    /// mark's, delays nothing it is combined with (decision 0330). Else `x` as it is.
    pub fn early_to(&mut self, x: Val, d: u8) -> Val {
        if x.d <= d || !first_bits(&self.program.ops, &self.program.tables, x.v).is_some_and(|s| s <= crate::block::EARLY_BITS) {
            return x;
        }
        let k = x.d - d;
        let v = match self.program.ops[usize::from(x.v)] {
            // A `prev` of at least `k`: its first `k` bits are the zeros it shifted in.
            BlockOp::Prev(p, j) if j > k => self.op(BlockOp::Prev(p, j - k)),
            BlockOp::Prev(p, j) if j == k => p,
            BlockOp::Early(p, j) if u16::from(j) + u16::from(k) <= 63 => self.op(BlockOp::Early(p, j + k)),
            _ => self.op(BlockOp::Early(x.v, k)),
        };
        Val { v, d, set: None, ..x }
    }

    /// The stream's bytes at delay `d`: `within`, a leaf, never a carry; with a prefix, less
    /// its bytes (`prefix`), which a full block knows are none.
    pub fn within(&mut self, d: u8) -> Result<u16, String> {
        if d > MAX_DELAY {
            return Err(format!("a delay of {d} bytes: more than {MAX_DELAY}"));
        }
        let w = self.op(BlockOp::Within(d));
        match self.prefix.as_ref().map(|p| p.1).filter(|_| !self.ahead) {
            Some(p) => {
                let p = self.at(p, d)?;
                Ok(self.op(BlockOp::AndNot(w, p)))
            }
            None => Ok(w),
        }
    }

    /// A rule's mask (`run R`, `pair P`): read at the bytes from a token's first on, never
    /// before it, so never at the prefix's, which lie before every token's. With a prefix, the
    /// mask without its cut where it is a byte class, which reads its own byte alone; else as
    /// any value is, since a `prev` in it reaches back from the stream's first bytes into the
    /// prefix's (decision 0335).
    pub fn ahead(&mut self, e: &Expr) -> Result<Val, String> {
        if self.prefix.is_none() || !self.of_classes(e, 0) {
            return self.expr(e);
        }
        self.ahead = true;
        let x = self.expr(e);
        self.ahead = false;
        match x? {
            x if x.set.is_some() && x.d == 0 => Ok(x),
            _ => self.expr(e),
        }
    }

    /// Whether `e` is made of byte classes alone (`~`, `&`, `|`, `^` of classes and of masks
    /// that are), so that lowering it again (`ahead`) makes nothing but byte tests.
    fn of_classes(&self, e: &Expr, depth: u32) -> bool {
        depth < 64 && match e {
            Expr::Empty => true,
            Expr::Name(n) => self.classes.contains_key(n) || self.mask_exprs.get(n).is_some_and(|x| self.of_classes(x, depth + 1)),
            Expr::Not(a) => self.of_classes(a, depth + 1),
            Expr::And(a, b) | Expr::Or(a, b) | Expr::Xor(a, b) => self.of_classes(a, depth + 1) && self.of_classes(b, depth + 1),
            _ => false,
        }
    }

    /// A prover over this program (`bitlex_prove::Induction`) that knows what the rounds' equations do not show
    /// alone: a walk's regions (`block::Walk`) hide each other, as the walk opens a region only past the last
    /// one's closer, so no two of its kinds' fills hold at one place (`NO_WALK_LEMMAS`: not told).
    fn induction(&self, horizon: u32, conflicts: u32) -> bitlex_prove::Induction {
        let mut settled = bitlex_prove::Induction::new(&self.program.rounds, horizon, conflicts);
        #[cfg(any(test, feature = "test-hooks"))]
        let told = !NO_WALK_LEMMAS.get();
        #[cfg(not(any(test, feature = "test-hooks")))]
        let told = true;
        if told {
            for walk in &self.program.walks {
                for (i, a) in walk.kinds.iter().enumerate() {
                    for b in &walk.kinds[i + 1..] {
                        // One group's values are at one delay: a place is one lag in both.
                        settled.add_lemma(vec![(a.fill, 0, true), (b.fill, 0, true)]);
                    }
                }
            }
        }
        settled
    }

    /// The operations as the prover reads them (`bitlex_prove`): with a prefix, its value zero.
    /// The prover's positions are the stream's, and every bit of the prefix's lies before the
    /// stream, where its model has no byte in any class and every carried value 0, as the cut
    /// classes and `within` are there. Without a prefix, the operations as they are.
    pub fn proof_ops(&self) -> std::borrow::Cow<'_, [BlockOp]> {
        proof_ops(&self.program, &self.prefix)
    }

    /// Whether `x` is zero everywhere, by the plain search (`bitlex_prove::never`), or answered
    /// before (`Proofs`).
    pub fn never(&mut self, x: Val) -> bool {
        let ops = proof_ops(&self.program, &self.prefix);
        let tables = &self.program.tables;
        let key = bitlex_prove::question(&ops, tables, &self.program.rounds, x.v, x.d, "plain never");
        self.proofs.never(key, || bitlex_prove::never(&ops, tables, x.v, x.d))
    }

    /// The bytes `x` can be at, by the plain search (`bitlex_prove::first_bytes`).
    pub fn first_bytes(&mut self, x: Val) -> ByteSet {
        let ops = proof_ops(&self.program, &self.prefix);
        let tables = &self.program.tables;
        let key = bitlex_prove::question(&ops, tables, &self.program.rounds, x.v, x.d, "plain first");
        self.proofs.bytes(key, || bitlex_prove::first_bytes(&ops, tables, x.v, x.d))
    }

    /// Whether `x` is zero everywhere, by `prover` (with the fixed points' equations).
    pub fn settled_never(&mut self, prover: &mut bitlex_prove::Induction, x: Val) -> bool {
        let ops = proof_ops(&self.program, &self.prefix);
        let tables = &self.program.tables;
        let key = bitlex_prove::question(&ops, tables, &self.program.rounds, x.v, x.d, &prover.asker("never"));
        self.proofs.never(key, || prover.never(&ops, tables, x.v, x.d))
    }

    /// The bytes `x` can be at, by `prover`.
    pub fn settled_first_bytes(&mut self, prover: &mut bitlex_prove::Induction, x: Val) -> ByteSet {
        let ops = proof_ops(&self.program, &self.prefix);
        let tables = &self.program.tables;
        let key = bitlex_prove::question(&ops, tables, &self.program.rounds, x.v, x.d, &prover.asker("first"));
        self.proofs.bytes(key, || prover.first_bytes(&ops, tables, x.v, x.d))
    }

    /// Whether `x` is zero everywhere, by the plain search and else `prover`: one question, the
    /// streams `prover` found refuting it first (`Induction::refutes`).
    pub fn either_never(&mut self, prover: &mut bitlex_prove::Induction, x: Val) -> bool {
        let ops = proof_ops(&self.program, &self.prefix);
        let tables = &self.program.tables;
        let key = bitlex_prove::question(&ops, tables, &self.program.rounds, x.v, x.d, &prover.asker("either never"));
        self.proofs.never(key, || !prover.refutes(&ops, tables, x.v, x.d) && (bitlex_prove::never(&ops, tables, x.v, x.d) || prover.never(&ops, tables, x.v, x.d)))
    }

    /// A prefix's value (`Stmt::Prefix`): the file's first `n` positions, where it starts with
    /// the prefix's bytes. One leaf (`BlockOp::Prefix`), which an executor makes once a file
    /// from the file's first word, as simdjson strips a byte order mark before its loop: a full
    /// block knows it is zero (`native_cpu_lanes`' `full_words`), and a window past the file's
    /// first bytes makes none of it (`native_cpu_bytes`).
    pub fn prefix_value(&mut self, bytes: &[u8]) -> Result<Val, String> {
        debug_assert!(self.prefix.is_none());
        if !(1..=8).contains(&bytes.len()) {
            return Err(format!("a prefix of {} bytes: from 1 to 8", bytes.len()));
        }
        let word = bytes.iter().enumerate().fold(0u64, |a, (j, &b)| a | u64::from(b) << (8 * j));
        let v = self.op(BlockOp::Prefix(bytes.len() as u8, word));
        Ok(Val { v, d: 0, dirty: false, set: None })
    }

    pub fn two(&mut self, a: Val, b: Val, f: fn(u16, u16) -> BlockOp, dirty: bool) -> Result<Val, String> {
        let (a, b) = (self.early_to(a, b.d), self.early_to(b, a.d));
        let d = a.d.max(b.d);
        let (x, y) = (self.at(a, d)?, self.at(b, d)?);
        Ok(Val { v: self.op(f(x, y)), d, dirty, set: None })
    }

    /// Two byte classes combined by `f`, bit by bit over their sets: one class.
    pub fn sets(&mut self, a: Val, b: Val, f: fn(u64, u64) -> u64) -> Option<Val> {
        let (x, y) = (a.set?, b.set?);
        Some(self.class_set(std::array::from_fn(|i| f(x[i], y[i]))))
    }

    pub fn and(&mut self, a: Val, b: Val) -> Result<Val, String> {
        // Zero is zero at any delay.
        if self.is_zero(a) || self.is_zero(b) {
            return Ok(if self.is_zero(a) { a } else { b });
        }
        if let Some(c) = self.sets(a, b, |x, y| x & y) {
            return Ok(c);
        }
        self.two(a, b, BlockOp::And, a.dirty && b.dirty)
    }

    pub fn or(&mut self, a: Val, b: Val) -> Result<Val, String> {
        if self.is_zero(a) || self.is_zero(b) {
            return Ok(if self.is_zero(a) { b } else { a });
        }
        if let Some(c) = self.sets(a, b, |x, y| x | y) {
            return Ok(c);
        }
        self.two(a, b, BlockOp::Or, a.dirty || b.dirty)
    }

    pub fn xor(&mut self, a: Val, b: Val) -> Result<Val, String> {
        if let Some(c) = self.sets(a, b, |x, y| x ^ y) {
            return Ok(c);
        }
        self.two(a, b, BlockOp::Xor, a.dirty || b.dirty)
    }

    /// `a & ~b`: no bits outside the stream that `a` has none of.
    pub fn andnot(&mut self, a: Val, b: Val) -> Result<Val, String> {
        if self.is_zero(a) || self.is_zero(b) {
            return Ok(a);
        }
        if let Some(c) = self.sets(a, b, |x, y| x & !y) {
            return Ok(c);
        }
        self.two(a, b, BlockOp::AndNot, a.dirty)
    }

    /// `~a`, inside the stream only. A class's complement is a class, dirty when it holds the
    /// padding's 0x00.
    pub fn not(&mut self, a: Val) -> Result<Val, String> {
        if let Some(x) = a.set {
            return Ok(self.class_set(x.map(|w| !w)));
        }
        let w = self.within(a.d)?;
        Ok(Val { v: self.op(BlockOp::AndNot(w, a.v)), d: a.d, dirty: false, set: None })
    }

    /// `prev(a, k)`. When `a` is `k` or more bytes late, its bits already stand for the bytes
    /// `k` earlier at a delay `k` smaller: no operation. Its bits for the bytes before the stream
    /// are clean (`next` cleans them), so the bytes shifted in are 0, as a carry's.
    /// `prev(a, k)`: dirty, whatever `a` is. Its last `k` bits land past the stream's end, by a
    /// shift or, when `a` is `k` or more bytes late, by `a` read `k` bytes earlier.
    pub fn prev(&mut self, a: Val, k: u64) -> Result<Val, String> {
        let dirty = a.dirty || !self.is_zero(a);
        if u64::from(a.d) >= k {
            return Ok(Val { d: a.d - k as u8, set: None, dirty, ..a });
        }
        let mut v = a.v;
        let mut left = k;
        while left > 0 {
            let step = left.min(63) as u8;
            v = self.prev_op(v, step);
            left -= u64::from(step);
        }
        Ok(Val { v, set: None, dirty, ..a })
    }

    /// `x`'s operations as a local check's (`program::LocalOp`), when it is made of byte tests,
    /// constants and `prev`s alone: no carried value. Its last op is `x`.
    /// A token test that is a local check (`kind_checks`): `count(A & L) at least 1`, `L` its
    /// terms that are local look-ahead (`local_ops`), `A` the rest; none, a test of the loop.
    pub fn local_split(&mut self, count: bool, least: u32, e: &Expr) -> Result<Option<(Val, Val)>, String> {
        fn terms<'e>(e: &'e Expr, out: &mut Vec<&'e Expr>) {
            match e {
                Expr::And(a, b) => {
                    terms(a, out);
                    terms(b, out);
                }
                e => out.push(e),
            }
        }
        if !count || least != 1 {
            return Ok(None);
        }
        let mut ts = Vec::new();
        terms(e, &mut ts);
        if ts.len() < 2 {
            return Ok(None);
        }
        let (mut a, mut l): (Option<Val>, Option<Val>) = (None, None);
        for t in ts {
            let x = self.expr(t)?;
            let side = if x.d > 0 && self.local_ops(x).is_some() { &mut l } else { &mut a };
            *side = Some(match *side {
                Some(y) => self.and(y, x)?,
                None => x,
            });
        }
        Ok(match (a, l) {
            (Some(a), Some(l)) if self.local_ops(l).is_some() => Some((a, l)),
            _ => None,
        })
    }

    pub fn local_ops(&self, x: Val) -> Option<Vec<crate::program::LocalOp>> {
        self.local_ops_with(x, false, None)
    }

    /// `local_ops` of a check that reads no byte before its place (decision 0399): its `prev`s
    /// and `within`s reach back no further than its delay, so from a place in a token every
    /// byte it reads is in the stream, past a prefix, where the prefix's value is 0. A `next`'s
    /// cut to the stream (`within` less the prefix) is then the bytes' `within`.
    pub fn local_ops_in_stream(&self, x: Val) -> Option<Vec<crate::program::LocalOp>> {
        use crate::program::LocalOp as L;
        let ops = self.local_ops_with(x, true, None)?;
        let mut reach = vec![0u32; ops.len()];
        for (k, op) in ops.iter().enumerate() {
            let at = |o: u16| reach[usize::from(o)];
            reach[k] = match *op {
                L::Prev(a, sh) => at(a) + u32::from(sh),
                L::Within(d) => u32::from(d),
                _ => op.operands().into_iter().map(at).max().unwrap_or(0),
            };
        }
        (reach.iter().all(|&r| r <= u32::from(x.d))).then_some(ops)
    }

    /// An error `A & ~L` at a row's tokens, with `A` at their last bytes (`last`), as a token's
    /// check (decision 0399): tier 2 tests it on the token's bytes and `L`'s after them, and
    /// needs no mask. Within the token `A` is `last` at its last byte and the rest of `A`'s ops
    /// there: `A` with `last` all ones, at the place `last` is `A`'s. Its ops: that, `L`, and
    /// `Last`, each at the check's delay (the most of `A`'s and `L`'s), then `A & Last & ~L`. `None`
    /// where `A` or `L` reads before the token's last byte, or reaches past 63 bytes.
    pub fn token_ops_past(&self, a: Val, last: Val, l: Val) -> Option<(Vec<crate::program::LocalOp>, u8)> {
        use crate::program::LocalOp as L;
        let reach = |ops: &[L]| {
            let mut r = vec![0u32; ops.len()];
            for (k, op) in ops.iter().enumerate() {
                r[k] = match *op {
                    L::Prev(x, sh) => r[usize::from(x)] + u32::from(sh),
                    L::Within(d) => u32::from(d),
                    _ => op.operands().into_iter().map(|o| r[usize::from(o)]).max().unwrap_or(0),
                };
            }
            r.into_iter().max().unwrap_or(0)
        };
        let mut ops = self.local_ops_with(a, true, Some((last.v, L::Const(u64::MAX), 0)))?;
        // `last`, if read, at `A`'s own place only: the depth from `A`'s top is `a.d - last.d`.
        let sub = ops.iter().position(|op| *op == L::Const(u64::MAX));
        if let Some(sub) = sub {
            let mut depths: Vec<Vec<u32>> = vec![Vec::new(); ops.len()];
            depths[ops.len() - 1].push(0);
            for k in (0..ops.len()).rev() {
                let ds = std::mem::take(&mut depths[k]);
                for &dd in &ds {
                    match ops[k] {
                        L::Prev(x, sh) => depths[usize::from(x)].push(dd + u32::from(sh)),
                        ref op => op.operands().into_iter().for_each(|o| depths[usize::from(o)].push(dd)),
                    }
                }
                depths[k] = ds;
            }
            let want = u32::from(a.d).checked_sub(u32::from(last.d))?;
            if depths[sub].iter().any(|&dd| dd != want) {
                return None;
            }
        }
        if reach(&ops) > u32::from(a.d) {
            return None;
        }
        let lops = self.local_ops_in_stream(l)?;
        let d = a.d.max(l.d);
        if d > 63 {
            return None;
        }
        let ta = ops.len() as u16 - 1;
        let base = ops.len() as u16;
        ops.extend(lops.iter().map(|op| offset_op(op, base)));
        let tl = ops.len() as u16 - 1;
        let at = |ops: &mut Vec<L>, x: u16, k: u8| {
            if k == 0 {
                x
            } else {
                ops.push(L::Prev(x, k));
                ops.len() as u16 - 1
            }
        };
        let sa = at(&mut ops, ta, d - a.d);
        let sl = at(&mut ops, tl, d - l.d);
        ops.push(L::Last);
        let last_op = ops.len() as u16 - 1;
        // `Last` is the token's last byte at its own place: at the check's delay, `d` on.
        let sz = at(&mut ops, last_op, d);
        ops.push(L::And(sa, sz));
        let both = ops.len() as u16 - 1;
        ops.push(L::AndNot(both, sl));
        Some((ops, d))
    }

    /// `x` with value `from` read as `to`. Where `to` is later than `from` by `k`, all of `x` is
    /// `k` later: each of its leaves read `k` bytes back (`prev`), so the ops between, carries
    /// too, are the same `k` bytes later. Else `to` is brought to `from`'s delay. Its ops on the
    /// way rebuilt, the rest shared. `None` past the most delay.
    pub fn substituted(&mut self, x: Val, from: Val, to: Val) -> Option<Val> {
        fn rebuild(low: &mut Lowering, v: u16, from: u16, to: u16, k: u8, memo: &mut HashMap<u16, u16>) -> u16 {
            if v == from {
                return to;
            }
            if let Some(&r) = memo.get(&v) {
                return r;
            }
            let op = low.program.ops[usize::from(v)];
            let olds = op.operands();
            let r = if olds.is_empty() {
                if k == 0 { v } else { low.op(BlockOp::Prev(v, k)) }
            } else {
                let news: Vec<u16> = olds.iter().map(|&o| rebuild(low, o, from, to, k, memo)).collect();
                if news == olds { v } else { low.op(op.renumbered(|o| news[olds.iter().position(|&p| p == o).expect("an operand")])) }
            };
            memo.insert(v, r);
            r
        }
        let k = to.d.saturating_sub(from.d);
        if x.d + k > MAX_DELAY {
            return None;
        }
        let to = if k == 0 { self.at(to, from.d).ok()? } else { to.v };
        let v = rebuild(self, x.v, from.v, to, k, &mut HashMap::default());
        Some(Val { v, d: x.d + k, ..x })
    }

    /// An error `A & ~L` at a row's tokens, with `A` at their last bytes (`last`) and `L` reading
    /// the row's last bytes before the token's (decision 0428): a token's check that reads the
    /// tokens before it. Tier 2's records are every row's tokens (`any`, their last bytes), so
    /// `L` is read with `any` for `last` where the prover shows that refuses the same bytes of
    /// `A`. Its ops: `token_ops_past`'s, `any` the records' `Ended`; and the bytes read back.
    /// `None` where it is not shown, `any` is read at or after the token's last byte, or an op read
    /// before the token is other than a byte's test (none of the prefix's), an `Ended`, an and,
    /// an or or a `prev`: before the file each is 0, as tier 1's `prev` reads it.
    pub fn token_ops_back(&mut self, a: Val, last: Val, l: Val, any: Val) -> Result<Option<(Vec<crate::program::LocalOp>, u8, u8)>, String> {
        use crate::program::LocalOp as L;
        let Some(sub) = self.substituted(l, last, any) else { return Ok(None) };
        if sub.v == l.v {
            return Ok(None);
        }
        // The same refusals: `A & (L ^ L')` never holds.
        let differ = self.xor(l, sub)?;
        let differ = self.and(a, differ)?;
        let ops = self.proof_ops();
        if !(bitlex_prove::never(&ops, &self.program.tables, differ.v, differ.d) || bitlex_prove::never_unrolled(&ops, &self.program.tables, &[], differ.v, differ.d, 10, 4000)) {
            return Ok(None);
        }
        let Some(mut ops) = self.local_ops_with(a, true, Some((last.v, L::Const(u64::MAX), 0))) else { return Ok(None) };
        let ta = ops.len() as u16 - 1;
        // `any` at its delay: `Ended` read that many bytes back.
        let Some(lops) = self.local_ops_with(sub, true, Some((any.v, L::Ended, any.d))) else { return Ok(None) };
        let l = sub;
        let d = a.d.max(l.d);
        let base = ops.len() as u16;
        ops.extend(lops.iter().map(|op| offset_op(op, base)));
        let tl = ops.len() as u16 - 1;
        let at = |ops: &mut Vec<L>, x: u16, k: u8| {
            if k == 0 {
                x
            } else {
                ops.push(L::Prev(x, k));
                ops.len() as u16 - 1
            }
        };
        let sa = at(&mut ops, ta, d - a.d);
        let sl = at(&mut ops, tl, d - l.d);
        ops.push(L::Last);
        let last_op = ops.len() as u16 - 1;
        let sz = at(&mut ops, last_op, d);
        ops.push(L::And(sa, sz));
        let both = ops.len() as u16 - 1;
        ops.push(L::AndNot(both, sl));
        // Each op's depths from the test (its `prev`s on the way): the test is at the token's
        // last byte and `d` on, so an op at depth `t` reads `d - t` from the last byte, before
        // the token's first where `t > d` (a token of one byte).
        let mut depths: Vec<std::collections::BTreeSet<u32>> = vec![Default::default(); ops.len()];
        depths[ops.len() - 1].insert(0);
        for k in (0..ops.len()).rev() {
            let ds: Vec<u32> = depths[k].iter().copied().collect();
            for dd in ds {
                match ops[k] {
                    L::Prev(x, sh) => {
                        depths[usize::from(x)].insert(dd + u32::from(sh));
                    }
                    ref op => op.operands().into_iter().for_each(|o| {
                        depths[usize::from(o)].insert(dd);
                    }),
                }
            }
        }
        // `A`'s ops read at its own place and after; `last` only there (`token_ops_past`).
        if depths[..usize::from(ta) + 1].iter().flatten().any(|&t| t > u32::from(d)) {
            return Ok(None);
        }
        // Read before the token, an op is tier 1's value there: 0 before the file. On a prefix
        // (a byte order mark), tier 1's are cut to the stream and these are not: so an op there
        // is 0 on the prefix's bytes (`zero`), or a `within` (`next`'s cut at the end) that only
        // an and with such an op reads.
        let prefix: Vec<u8> = self.prefix.as_ref().map_or(Vec::new(), |p| p.0.clone());
        let mut zero = vec![false; ops.len()];
        let mut within = vec![false; ops.len()];
        for (k, op) in ops.iter().enumerate() {
            (zero[k], within[k]) = match *op {
                L::Bytes(set) => (prefix.iter().all(|&b| set[usize::from(b) / 64] >> (b % 64) & 1 == 0), false),
                L::Ended => (true, false),
                L::Within(_) => (prefix.is_empty(), true),
                L::And(x, y) => (zero[usize::from(x)] || zero[usize::from(y)], false),
                L::Or(x, y) => (zero[usize::from(x)] && zero[usize::from(y)], false),
                L::Prev(x, _) => (zero[usize::from(x)], within[usize::from(x)]),
                _ => (false, false),
            };
        }
        let mut back = 0u32;
        for (k, op) in ops.iter().enumerate() {
            let Some(&t) = depths[k].iter().next_back() else { continue };
            if matches!(op, L::Ended) && depths[k].iter().any(|&t| t <= u32::from(d)) {
                return Ok(None);
            }
            if t <= u32::from(d) {
                continue;
            }
            back = back.max(t - u32::from(d));
            let ok = match *op {
                L::Bytes(_) | L::Ended => zero[k],
                L::Within(_) | L::Prev(..) => true,
                L::And(x, y) => {
                    let (x, y) = (usize::from(x), usize::from(y));
                    (!within[x] || zero[y]) && (!within[y] || zero[x])
                }
                L::Or(x, y) => !within[usize::from(x)] && !within[usize::from(y)],
                _ => false,
            };
            if !ok {
                return Ok(None);
            }
        }
        if back == 0 || u32::from(d) + back >= 64 {
            return Ok(None);
        }
        Ok(Some((ops, d, back as u8)))
    }

    /// Whether `v` is the prefix's value, at some delay (`prev`s of `BlockOp::Prefix`).
    fn prefix_chain(&self, v: u16) -> bool {
        match self.program.ops[usize::from(v)] {
            BlockOp::Prefix(..) => true,
            BlockOp::Prev(x, _) => self.prefix_chain(x),
            _ => false,
        }
    }

    /// `x`'s ops, with value `ones.0` read as the op `ones.1` (`Const(!0)` at a token's own
    /// last byte, `token_ops_past`; `Ended`, `token_ops_back`).
    fn local_ops_with(&self, x: Val, prefix_zero: bool, ones: Option<(u16, crate::program::LocalOp, u8)>) -> Option<Vec<crate::program::LocalOp>> {
        use crate::program::LocalOp as L;
        fn walk(low: &Lowering, v: u16, at: &mut HashMap<u16, u16>, out: &mut Vec<L>, pz: bool, ones: Option<(u16, L, u8)>) -> Option<u16> {
            if let Some(&k) = at.get(&v) {
                return Some(k);
            }
            if let Some((_, op, late)) = ones.as_ref().filter(|o| o.0 == v) {
                out.push(op.clone());
                if *late > 0 {
                    out.push(L::Prev(out.len() as u16 - 1, *late));
                }
                at.insert(v, out.len() as u16 - 1);
                return Some(out.len() as u16 - 1);
            }
            if out.len() > 512 {
                return None;
            }
            let set_of = |table: u16, bits: u8| -> ByteSet {
                let t = &low.program.tables[usize::from(table)];
                let mut s = [0u64; 4];
                for b in 0..256usize {
                    if t.member(b as u8, bits) {
                        s[b / 64] |= 1 << (b % 64);
                    }
                }
                s
            };
            let op = match low.program.ops[usize::from(v)] {
                BlockOp::Eq(c) => {
                    let mut s = [0u64; 4];
                    s[usize::from(c) / 64] |= 1 << (c % 64);
                    L::Bytes(s)
                }
                BlockOp::Class { table, bits } => L::Bytes(set_of(table, bits)),
                BlockOp::Pair { before, table, bits } => {
                    // The byte before the stream is 0x00 to a pair, nothing to a `prev`.
                    let b = set_of(before, bits);
                    if b[0] & 1 != 0 {
                        return None;
                    }
                    out.push(L::Bytes(b));
                    let p = L::Prev(out.len() as u16 - 1, 1);
                    out.push(p);
                    let p = out.len() as u16 - 1;
                    out.push(L::Bytes(set_of(table, bits)));
                    L::And(p, out.len() as u16 - 1)
                }
                BlockOp::Const(c) => L::Const(c),
                BlockOp::Within(d) => L::Within(d),
                BlockOp::Prefix(..) if pz => L::Const(0),
                BlockOp::Not(a) => L::Not(walk(low, a, at, out, pz, ones.clone())?),
                BlockOp::Prev(a, k) => L::Prev(walk(low, a, at, out, pz, ones.clone())?, k),
                BlockOp::And(a, b) => L::And(walk(low, a, at, out, pz, ones.clone())?, walk(low, b, at, out, pz, ones.clone())?),
                BlockOp::Or(a, b) => L::Or(walk(low, a, at, out, pz, ones.clone())?, walk(low, b, at, out, pz, ones.clone())?),
                BlockOp::Xor(a, b) => L::Xor(walk(low, a, at, out, pz, ones.clone())?, walk(low, b, at, out, pz, ones.clone())?),
                // `x` less the prefix, where the prefix is 0: `x`.
                BlockOp::AndNot(a, b) if pz && low.prefix_chain(b) => return walk(low, a, at, out, pz, ones),
                BlockOp::AndNot(a, b) => L::AndNot(walk(low, a, at, out, pz, ones.clone())?, walk(low, b, at, out, pz, ones.clone())?),
                _ => return None,
            };
            out.push(op);
            let k = out.len() as u16 - 1;
            at.insert(v, k);
            Some(k)
        }
        let mut out = Vec::new();
        walk(self, x.v, &mut HashMap::default(), &mut out, prefix_zero, ones)?;
        Some(out)
    }

    /// Whether `b` holds at every byte `a` holds at, as their operations show it: a conjunct
    /// of `a`'s, or each of its disjuncts, implies `b`; `a` implies a disjunct of `b`'s, or
    /// each of its conjuncts; a `prev` is its operand at a delay as much smaller; `within` at
    /// one delay is `within` at another, each at its own. False where the operations do not
    /// show it (a carry, a fill), and past a bound on the steps.
    pub fn implies(&self, a: Val, b: Val) -> bool {
        let mut steps = 4096;
        self.implied(a.v, i32::from(a.d), b.v, i32::from(b.d), &mut steps)
    }

    fn implied(&self, a: u16, da: i32, b: u16, db: i32, steps: &mut u32) -> bool {
        if a == b && da == db {
            return true;
        }
        if *steps == 0 {
            return false;
        }
        *steps -= 1;
        let (x, y) = (self.program.ops[usize::from(a)], self.program.ops[usize::from(b)]);
        let by_a = match x {
            BlockOp::Const(0) => true,
            BlockOp::Prev(p, k) => self.implied(p, da - i32::from(k), b, db, steps),
            // The same bytes' bits as its operand's, at a delay `k` smaller.
            BlockOp::Early(p, k) => self.implied(p, da + i32::from(k), b, db, steps),
            BlockOp::And(p, q) => self.implied(p, da, b, db, steps) || self.implied(q, da, b, db, steps),
            BlockOp::AndNot(p, _) => self.implied(p, da, b, db, steps),
            BlockOp::Or(p, q) => self.implied(p, da, b, db, steps) && self.implied(q, da, b, db, steps),
            BlockOp::Within(k) => matches!(y, BlockOp::Within(j) if da - i32::from(k) == db - i32::from(j)),
            _ => false,
        };
        by_a || match y {
            // Its first `k` bits are the bytes before the stream's: only at a delay of `k` or more.
            BlockOp::Prev(q, k) if db >= i32::from(k) => self.implied(a, da, q, db - i32::from(k), steps),
            BlockOp::Early(q, k) => self.implied(a, da, q, db + i32::from(k), steps),
            BlockOp::Or(p, q) => self.implied(a, da, p, db, steps) || self.implied(a, da, q, db, steps),
            BlockOp::And(p, q) => self.implied(a, da, p, db, steps) && self.implied(a, da, q, db, steps),
            _ => false,
        }
    }

    /// Whether `a` is the constant zero.
    pub fn is_zero(&self, a: Val) -> bool {
        self.program.ops.get(usize::from(a.v)) == Some(&BlockOp::Const(0))
    }

    /// `next(a, k)`: `a` itself, `k` bytes later. Two cleanings: past the end in `a`'s own
    /// delay, if it is dirty; and before the start in the new delay, where `a`'s first `k`
    /// bytes now stand for the bytes before the stream.
    pub fn next(&mut self, a: Val, k: u64) -> Result<Val, String> {
        let d = u64::from(a.d) + k;
        if d > u64::from(MAX_DELAY) {
            return Err(format!("next by {k} at a delay of {}: more than {MAX_DELAY} bytes", a.d));
        }
        let d = d as u8;
        let mut v = a.v;
        if a.dirty {
            let w = self.within(a.d)?;
            v = self.op(BlockOp::And(v, w));
        }
        if k > 0 {
            let w = self.within(d)?;
            v = self.op(BlockOp::And(v, w));
        }
        Ok(Val { v, d, dirty: false, set: None })
    }

    /// A carried operation of one operand: its state may run on past the end.
    pub fn carried(&mut self, a: Val, f: fn(u16) -> BlockOp) -> Val {
        Val { v: self.op(f(a.v)), d: a.d, dirty: true, set: None }
    }

    pub fn fill(&mut self, s: Val, e: Val) -> Result<Val, String> {
        self.two(s, e, BlockOp::Fill, true)
    }

    /// `back_fill(s, e)`: `s` cleaned past the end first, so that no fill comes back from there;
    /// the result cleaned before the start, where a fill may run back into the bits that stand
    /// for the bytes before the stream at its delay. Clean on both sides.
    pub fn back_fill(&mut self, s: Val, e: Val) -> Result<Val, String> {
        let s = if s.dirty {
            let w = self.within(s.d)?;
            Val { v: self.op(BlockOp::And(s.v, w)), dirty: false, set: None, ..s }
        } else {
            s
        };
        let b = self.two(s, e, BlockOp::BackFill, false)?;
        let w = self.within(b.d)?;
        Ok(Val { v: self.op(BlockOp::And(b.v, w)), ..b })
    }

    /// A pattern over the stream's byte positions (bit `i % 64` for byte `i`).
    pub fn pattern(&mut self, c: u64) -> Val {
        self.leaf(BlockOp::Const(c), c != 0)
    }

    pub fn name(&mut self, name: &str) -> Result<Val, String> {
        if self.ahead {
            if let Some(e) = self.mask_exprs.get(name).cloned() {
                return self.expr(&e);
            }
            if let Some(ranges) = self.classes.get(name).cloned() {
                return Ok(self.class(&ranges));
            }
        }
        if let Some(&v) = self.names.get(name) {
            return Ok(v);
        }
        let ranges = self.classes.get(name).cloned().ok_or_else(|| format!("undefined: {name}"))?;
        let value = self.class(&ranges);
        self.names.insert(name.to_owned(), value);
        Ok(value)
    }

    /// A class of byte ranges (`class_set`).
    pub fn class(&mut self, ranges: &[(u32, u32)]) -> Val {
        let mut set = [0u64; 4];
        for &(lo, hi) in ranges {
            for b in lo..=hi.min(255) {
                set[b as usize / 64] |= 1 << (b % 64);
            }
        }
        self.class_set(set)
    }

    /// A class (`class_bytes`), cut to the stream where it holds a byte of the prefix
    /// (`prefix`): the prefix's bytes are no class's. Still a class to `sets`, which makes the
    /// combined class here again.
    pub fn class_set(&mut self, set: ByteSet) -> Val {
        let x = self.class_bytes(set);
        match &self.prefix {
            Some((bytes, p)) if !self.ahead && bytes.iter().any(|&b| set[usize::from(b) / 64] >> (b % 64) & 1 == 1) => {
                let p = p.v;
                Val { v: self.op(BlockOp::AndNot(x.v, p)), ..x }
            }
            _ => x,
        }
    }

    /// A class: an `eq` for one byte, otherwise one `class` per table its products use, ORed.
    /// Dirty when the class holds the padding's byte, 0x00.
    fn class_bytes(&mut self, set: ByteSet) -> Val {
        let has = |b: u32| set[b as usize / 64] >> (b % 64) & 1 == 1;
        let bytes: Vec<u8> = (0..=255u8).filter(|&b| has(u32::from(b))).collect();
        let dirty = has(0);
        match bytes.as_slice() {
            [] => return Val { set: Some(set), ..self.leaf(BlockOp::Const(0), false) },
            [b] => return Val { set: Some(set), ..self.leaf(BlockOp::Eq(*b), dirty) },
            _ => {}
        }
        // The set as ranges of consecutive bytes, for its products.
        let mut ranges: Vec<(u32, u32)> = Vec::new();
        for &b in &bytes {
            match ranges.last_mut() {
                Some((_, hi)) if *hi + 1 == u32::from(b) => *hi = u32::from(b),
                _ => ranges.push((u32::from(b), u32::from(b))),
            }
        }
        let per_table = self.place(&products(&ranges));
        let mut value = None;
        for (table, bits) in per_table {
            let v = self.op(BlockOp::Class { table, bits });
            value = Some(match value {
                None => v,
                Some(acc) => self.op(BlockOp::Or(acc, v)),
            });
        }
        Val { v: value.expect("a class of two bytes or more has a product"), d: 0, dirty, set: Some(set) }
    }

    pub fn expr(&mut self, e: &Expr) -> Result<Val, String> {
        match e {
            Expr::Name(name) => self.name(name),
            Expr::Empty => Ok(self.leaf(BlockOp::Const(0), false)),
            Expr::Not(a) => {
                let a = self.expr(a)?;
                self.not(a)
            }
            // `a & ~b` is one `andnot`.
            Expr::And(a, b) => match (&**a, &**b) {
                (x, Expr::Not(y)) | (Expr::Not(y), x) => {
                    let (x, y) = (self.expr(x)?, self.expr(y)?);
                    self.andnot(x, y)
                }
                _ => {
                    let (a, b) = (self.expr(a)?, self.expr(b)?);
                    self.and(a, b)
                }
            },
            Expr::Or(a, b) => {
                let (a, b) = (self.expr(a)?, self.expr(b)?);
                self.or(a, b)
            }
            Expr::Xor(a, b) => {
                let (a, b) = (self.expr(a)?, self.expr(b)?);
                self.xor(a, b)
            }
            Expr::Call(f, args) => self.call(f, args),
            Expr::Test { target, method, arg: Some(arg), .. } if method == "frame" => {
                if let Some(i) = self.pending_counts.iter().position(|c| &c.0 == target) {
                    let (name, up, down, max) = self.pending_counts.remove(i);
                    let count = self.count(up, down, max)?;
                    self.counts.insert(name, count);
                }
                let count = self.counts.get(target).cloned().ok_or_else(|| format!("{target}.frame: `{target}` is not a count"))?;
                let a = self.expr(arg)?;
                self.frame(&count, a)
            }
            Expr::Test { target, method, .. } => Err(format!("not compiled yet: {target}.{method}")),
        }
    }

    pub fn call(&mut self, f: &str, args: &[Arg]) -> Result<Val, String> {
        let mask = |low: &mut Self, i: usize| -> Result<Val, String> {
            match args.get(i) {
                Some(Arg::E(e)) => low.expr(e),
                _ => Err(format!("{f}: argument {i} must be a mask")),
            }
        };
        let number = |i: usize, default: u64| -> Result<u64, String> {
            match args.get(i) {
                Some(Arg::N(v)) => Ok(*v),
                None => Ok(default),
                _ => Err(format!("{f}: argument {i} must be a number")),
            }
        };
        match f {
            "prev" => {
                let (a, k) = (mask(self, 0)?, number(1, 1)?);
                self.prev(a, k)
            }
            "next" => {
                let (a, k) = (mask(self, 0)?, number(1, 1)?);
                self.next(a, k)
            }
            "starts" => {
                let a = mask(self, 0)?;
                let p = self.prev(a, 1)?;
                self.andnot(a, p)
            }
            "lasts" => {
                let a = mask(self, 0)?;
                let n = self.next(a, 1)?;
                self.andnot(a, n)
            }
            "after" => {
                let a = mask(self, 0)?;
                let p = self.prev(a, 1)?;
                self.andnot(p, a)
            }
            "span" => {
                let (a, k) = (mask(self, 0)?, number(1, 1)?);
                let mut out = a;
                for j in 1..k {
                    let p = self.prev(a, j)?;
                    out = self.or(out, p)?;
                }
                Ok(out)
            }
            "aligned" => {
                let (a, k) = (mask(self, 0)?, number(1, 1)?);
                self.aligned(a, k)
            }
            "toggle" => {
                let a = mask(self, 0)?;
                let t = self.carried(a, BlockOp::Toggle);
                self.toggles.push((t, a));
                Ok(t)
            }
            // simdjson's escaped bytes; `after_odd_run` is the ones outside the run.
            "escaped" => {
                let a = mask(self, 0)?;
                Ok(self.carried(a, BlockOp::Escaped))
            }
            "after_odd_run" => {
                let a = mask(self, 0)?;
                let escaped = self.carried(a, BlockOp::Escaped);
                self.andnot(escaped, a)
            }
            "fill" => {
                let (start, end) = (mask(self, 0)?, mask(self, 1)?);
                self.fill(start, end)
            }
            "back_fill" => {
                let (start, end) = (mask(self, 0)?, mask(self, 1)?);
                self.back_fill(start, end)
            }
            "at_least" | "above" => {
                let operand = |me: &mut Self, i: usize| -> Result<Vec<Val>, String> {
                    match args.get(i) {
                        Some(Arg::E(Expr::Name(n))) if n == "position" => me.number(&NExpr::Position),
                        Some(Arg::E(Expr::Name(n))) => me.numbers.get(n).cloned().ok_or_else(|| format!("{f}: {n} is not a number")),
                        Some(Arg::N(k)) if *k <= u64::from(u32::MAX) => Ok(me.constant(*k as u32)),
                        _ => Err(format!("{f}: argument {i} must be a number's name or a constant below 2^32")),
                    }
                };
                let (a, b) = (operand(self, 0)?, operand(self, 1)?);
                if f == "at_least" {
                    self.at_least(&a, &b)
                } else {
                    let ge = self.at_least(&b, &a)?;
                    self.not(ge)
                }
            }
            // The bytes at `i`, `i + 1`, …: each byte's class, from the byte that far ahead.
            "text" => {
                let Some(Arg::S(bytes)) = args.first() else { return Err("text: a string".into()) };
                let mut out: Option<Val> = None;
                for (k, &b) in bytes.iter().enumerate() {
                    let mut one: ByteSet = [0; 4];
                    one[usize::from(b) / 64] |= 1 << (b % 64);
                    let c = self.class_set(one);
                    let c = if k == 0 { c } else { self.next(c, k as u64)? };
                    out = Some(match out {
                        Some(o) => self.and(o, c)?,
                        None => c,
                    });
                }
                out.ok_or_else(|| "text: an empty string".into())
            }
            _ => Err(format!("not compiled yet: {f}")),
        }
    }

    /// A number's planes (the module's notes).
    pub fn number(&mut self, e: &NExpr) -> Result<Vec<Val>, String> {
        match e {
            NExpr::Position => Ok((0..NUMBER_BITS)
                .map(|j| if j < 6 { self.pattern(crate::block::position(0, j)) } else { self.leaf(BlockOp::Position(j), true) })
                .collect()),
            NExpr::Name(n) => self.numbers.get(n).cloned().ok_or_else(|| format!("{n} is not a number")),
            NExpr::Fill { s, e, v, back } => {
                let (s, e, v) = (self.expr(s)?, self.expr(e)?, self.number(v)?);
                let stop = self.or(e, s)?;
                let mut out = Vec::new();
                for p in v {
                    let from = self.and(s, p)?;
                    out.push(match () {
                        _ if self.is_zero(from) => from,
                        _ if *back => self.back_fill(from, stop)?,
                        _ => self.fill(from, stop)?,
                    });
                }
                Ok(out)
            }
            NExpr::Sub(a, b) => {
                let (a, b) = (self.number(a)?, self.number(b)?);
                let mut borrow: Option<Val> = None;
                let mut out = Vec::new();
                for (&x, &y) in a.iter().zip(&b) {
                    let differ = self.xor(x, y)?;
                    let under = self.andnot(y, x)?;
                    match borrow {
                        None => {
                            out.push(differ);
                            borrow = Some(under);
                        }
                        Some(br) => {
                            out.push(self.xor(differ, br)?);
                            let kept = self.andnot(br, differ)?;
                            borrow = Some(self.or(under, kept)?);
                        }
                    }
                }
                Ok(out)
            }
        }
    }

    /// A constant's planes: `within` where its bit is 1.
    pub fn constant(&mut self, k: u32) -> Vec<Val> {
        (0..NUMBER_BITS).map(|j| if k >> j & 1 == 1 { self.within_val(0).expect("delay 0") } else { self.leaf(BlockOp::Const(0), false) }).collect()
    }

    /// The bytes where `a >= b`: no borrow out of `a - b`, the borrows alone.
    pub fn at_least(&mut self, a: &[Val], b: &[Val]) -> Result<Val, String> {
        let mut borrow = self.leaf(BlockOp::Const(0), false);
        for (&x, &y) in a.iter().zip(b) {
            let under = self.andnot(y, x)?;
            let differ = self.xor(x, y)?;
            let kept = self.andnot(borrow, differ)?;
            borrow = self.or(under, kept)?;
        }
        self.not(borrow)
    }

    /// A count's planes (see the module's notes), and its range check as a fallback.
    pub fn count(&mut self, up: Val, down: Val, max: u32) -> Result<Count, String> {
        let down = self.andnot(down, up)?;
        let mut w = 1u32;
        while (1u64 << w) < u64::from(max) + 2 {
            w += 1;
        }
        let mut planes: Vec<Val> = Vec::new();
        let mut before: Vec<Val> = Vec::new();
        for k in 0..w as usize {
            let flips = if k == 0 {
                self.or(up, down)?
            } else {
                // All of the planes below k were 1 (an up carries into k), or all 0 (a down
                // borrows from it).
                let mut ones = before[0];
                let mut any = before[0];
                for &b in &before[1..k] {
                    ones = self.and(ones, b)?;
                    any = self.or(any, b)?;
                }
                let carry = self.and(up, ones)?;
                let borrow = self.andnot(down, any)?;
                self.or(carry, borrow)?
            };
            let plane = self.carried(flips, BlockOp::Toggle);
            before.push(self.prev(plane, 1)?);
            planes.push(plane);
        }
        let over = self.above(&planes, u64::from(max))?;
        self.fallbacks.push(over);
        Ok(Count { up, down, max, planes })
    }

    /// The bytes where the value in `planes` is above `m` (as a `planes.len()`-bit number).
    pub fn above(&mut self, planes: &[Val], m: u64) -> Result<Val, String> {
        let mut out = self.leaf(BlockOp::Const(0), false);
        // From the top bit down: above at bit k when the bits above k equal m's, bit k is 1 and
        // m's bit k is 0.
        let mut equal: Option<Val> = None;
        for k in (0..planes.len()).rev() {
            let p = planes[k];
            if m >> k & 1 == 0 {
                let here = match equal {
                    Some(e) => self.and(e, p)?,
                    None => p,
                };
                out = self.or(out, here)?;
                equal = Some(match equal {
                    Some(e) => self.andnot(e, p)?,
                    None => self.not(p)?,
                });
            } else {
                equal = Some(match equal {
                    Some(e) => self.and(e, p)?,
                    None => p,
                });
            }
        }
        Ok(out)
    }

    /// The bytes where the value in `planes` is `m`.
    pub fn equals(&mut self, planes: &[Val], m: u64) -> Result<Val, String> {
        let mut out: Option<Val> = None;
        for (k, &p) in planes.iter().enumerate() {
            out = Some(match (out, m >> k & 1 == 1) {
                (None, true) => p,
                (None, false) => self.not(p)?,
                (Some(e), true) => self.and(e, p)?,
                (Some(e), false) => self.andnot(e, p)?,
            });
        }
        Ok(out.expect("a count has a plane"))
    }

    /// `C.frame(a)`: the frame open before each byte was opened by a byte of `a`. Per level
    /// `L`, the openers there (`up` where the count after is `L`) fill from those in `a` to the
    /// next one not in `a`; a byte reads its level's fill when the count before it is `L`.
    pub fn frame(&mut self, count: &Count, a: Val) -> Result<Val, String> {
        let before: Vec<Val> = count.planes.iter().map(|&p| self.prev(p, 1)).collect::<Result<_, _>>()?;
        let mut out = self.leaf(BlockOp::Const(0), false);
        for level in 1..=u64::from(count.max) {
            let after = self.equals(&count.planes, level)?;
            let openers = self.and(count.up, after)?;
            let from = self.and(openers, a)?;
            let to = self.andnot(openers, a)?;
            let open = self.fill(from, to)?;
            let here = self.equals(&before, level)?;
            let kept = self.and(here, open)?;
            out = self.or(out, kept)?;
        }
        Ok(out)
    }

    /// The tokens: each kind's start and last-byte masks (keywords retyped), and the overlap,
    /// coverage and keyword checks as fallbacks.
    pub fn tokens(&mut self) -> Result<Vec<(String, Val, Val)>, String> {
        if self.tokens.is_empty() {
            return Ok(Vec::new());
        }
        let tokens = std::mem::take(&mut self.tokens);
        // With a tree section tier 2 takes the tokens in order and retypes keywords; a token no
        // state takes sends the file to tree-sitter (doc 18 §2). Tier 1 keeps each token's start
        // and last byte only.
        let tier2 = !self.tree.is_empty();
        if tier2 {
            let mut out = Vec::new();
            for (kind, start, end) in tokens {
                let last = match end {
                    None => start,
                    Some(TokenEnd::Last(l)) => l,
                    Some(TokenEnd::Through(_)) => return Err(format!("token {kind}: `through` needs a fill; with a tree section write its last byte (`last`, decision 0071)")),
                };
                out.push((kind, start, last));
            }
            return Ok(out);
        }
        // Two kinds starting at one byte.
        let mut starts = tokens[0].1;
        let mut overlap = self.leaf(BlockOp::Const(0), false);
        for t in &tokens[1..] {
            let both = self.and(starts, t.1)?;
            overlap = self.or(overlap, both)?;
            starts = self.or(starts, t.1)?;
        }
        self.fallbacks.push(overlap);
        let mut covered = self.leaf(BlockOp::Const(0), false);
        let mut out = Vec::new();
        for (kind, start, through) in tokens {
            // A token runs from its start through `through`'s bytes, up to the next start.
            let span = match through.map(|t| match t {
                TokenEnd::Through(t) => t,
                TokenEnd::Last(l) => l,
            }) {
                Some(t) if matches!(through, Some(TokenEnd::Through(_))) => {
                    let outside = self.not(t)?;
                    let stop = self.or(outside, starts)?;
                    let run = self.fill(start, stop)?;
                    let inside = self.within_val(run.d)?;
                    self.and(run, inside)?
                }
                // A token with a last-byte mask: its bytes are not a mask here (no fill); it is
                // covered from its start only.
                _ => start,
            };
            covered = self.or(covered, span)?;
            // A token's last byte: the last of its span before a byte that does not continue
            // it. Without `through` the span is the start byte alone, which is the last.
            let last = match through {
                None => start,
                Some(TokenEnd::Last(l)) => l,
                Some(TokenEnd::Through(_)) => {
                    let inner = self.andnot(span, starts)?;
                    let continues = self.next(inner, 1)?;
                    self.andnot(span, continues)?
                }
            };
            match self.keywords.iter().position(|k| k.0 == kind) {
                None => out.push((kind, start, last)),
                Some(at) => {
                    let (_, words, required) = self.keywords[at].clone();
                    let mut any = self.leaf(BlockOp::Const(0), false);
                    for (word, alias) in words {
                        let here = self.keyword(start, last, &word)?;
                        any = self.or(any, here)?;
                        let back = self.prev(here, word.len() as u64 - 1)?;
                        let word_last = self.and(last, back)?;
                        let name = alias.unwrap_or_else(|| String::from_utf8_lossy(&word).into_owned());
                        out.push((name, here, word_last));
                    }
                    let other = self.andnot(start, any)?;
                    if !required {
                        return Err(format!("keywords {kind}: only `required` keywords are compiled yet"));
                    }
                    self.fallbacks.push(other);
                }
            }
        }
        let skipped = match self.skip {
            Some(s) => self.or(covered, s)?,
            None => covered,
        };
        let gap = self.not(skipped)?;
        self.fallbacks.push(gap);
        Ok(out)
    }

    /// A check's byte machine over the tokens from `start` to `last`, as masks over every byte
    /// at once (doc 18 §4), computed with the rest of the block program: the last bytes of the
    /// tokens it refuses. Bit `i` of state `q`'s mask is set where the machine is in `q` after a
    /// token's bytes up to byte `i`:
    ///
    /// - it enters `q` on bytes `B` from states `P` at `B & prev(P's masks, 1)`, not at a token's
    ///   first byte; from the start, at `B & start`. The arms into `q` on the same bytes share
    ///   one `prev` of their sources' masks ORed.
    /// - a self-loop on bytes `L` keeps it there over a run: `fill(entered, ~L | start)`.
    ///
    /// The machine cannot be re-entered inside a token once it has no state, so a checked token is
    /// refused where its last byte is outside every accepting state. The states are taken in the order of
    /// their arms: a cycle through two or more states is refused.
    ///
    /// A token beginning with a byte the machine does not take is tier 2's: its switch refuses
    /// the byte, or takes a keyword by its bytes (`first`, the row's check bytes). A keyword in
    /// `words` beginning with a byte the machine takes is accepted at its own last byte.
    pub fn check(&mut self, start: Val, last: Val, accept: &[bool], arms: &[(u16, ByteSet, u16)], words: &[Vec<u8>]) -> Result<Val, String> {
        let n = accept.len();
        let mut order: Vec<usize> = Vec::new();
        let mut ins: Vec<usize> = (0..n).map(|q| arms.iter().filter(|a| usize::from(a.2) == q && a.0 != a.2).count()).collect();
        if ins[0] != 0 {
            return Err("an arm back to the start: not compiled yet".into());
        }
        let mut ready = vec![0usize];
        while let Some(p) = ready.pop() {
            order.push(p);
            for a in arms.iter().filter(|a| usize::from(a.0) == p && a.0 != a.2) {
                let q = usize::from(a.2);
                ins[q] -= 1;
                if ins[q] == 0 {
                    ready.push(q);
                }
            }
        }
        if order.len() != n {
            return Err("a cycle through two or more states: not compiled yet".into());
        }
        let zero = self.leaf(BlockOp::Const(0), false);
        // The tokens the machine checks: those beginning with a byte it takes. Every mask below
        // is made from these starts, so a block with none (and no state carried in) has every
        // mask zero: the lowering may skip them (`native_cpu_lanes`, gates).
        let takes = arms.iter().filter(|a| a.0 == 0).fold([0u64; 4], |s, a| std::array::from_fn(|i| s[i] | a.1[i]));
        let taken = self.class_set(takes);
        let checked = self.and(start, taken)?;
        let mut masks: Vec<Option<Val>> = vec![None; n];
        for &q in &order {
            // The arms into `q`, by their bytes: the sources of each.
            let mut groups: Vec<(ByteSet, Vec<usize>)> = Vec::new();
            for a in arms.iter().filter(|a| usize::from(a.2) == q && (a.0 != a.2 || q == 0)) {
                match groups.iter_mut().find(|g| g.0 == a.1) {
                    Some(g) => g.1.push(usize::from(a.0)),
                    None => groups.push((a.1, vec![usize::from(a.0)])),
                }
            }
            let (mut first, mut later) = (zero, zero);
            for (bytes, from) in groups {
                let bytes = self.class_set(bytes);
                // From the start (a self-loop on the start is entered at the first byte only).
                if from.contains(&0) {
                    let here = self.and(checked, bytes)?;
                    first = self.or(first, here)?;
                }
                let mut sources = zero;
                for &p in from.iter().filter(|&&p| p != q) {
                    let m = masks[p].expect("a source before its target");
                    if !self.is_zero(m) {
                        sources = self.or(sources, m)?;
                    }
                }
                if !self.is_zero(sources) {
                    let back = self.prev(sources, 1)?;
                    let here = self.and(back, bytes)?;
                    later = self.or(later, here)?;
                }
            }
            let mut mask = if self.is_zero(later) {
                first
            } else {
                let inside = self.andnot(later, start)?;
                self.or(first, inside)?
            };
            if let Some(a) = arms.iter().find(|a| usize::from(a.0) == q && a.2 == a.0) {
                let stays = self.class_set(a.1);
                let leaves = self.not(stays)?;
                let stop = self.or(leaves, start)?;
                mask = self.fill(mask, stop)?;
            }
            masks[q] = Some(mask);
        }
        let mut good = zero;
        for q in (0..n).filter(|&q| accept[q]) {
            good = self.or(good, masks[q].expect("every state"))?;
        }
        // The checked tokens' bytes, up to the next start: a token beginning with a byte the
        // machine does not take is tier 2's (its switch takes a keyword or refuses the byte).
        let span = self.fill(checked, start)?;
        let ends = self.andnot(last, good)?;
        let mut refused = self.and(span, ends)?;
        for w in words.iter().filter(|w| takes[usize::from(w[0] / 64)] >> (w[0] % 64) & 1 == 1) {
            let here = self.keyword(start, last, w)?;
            let back = self.prev(here, w.len() as u64 - 1)?;
            let word_last = self.and(last, back)?;
            refused = self.andnot(refused, word_last)?;
        }
        Ok(refused)
    }

    /// `within` as a value at delay `d`.
    pub fn within_val(&mut self, d: u8) -> Result<Val, String> {
        Ok(Val { v: self.within(d)?, d, dirty: false, set: None })
    }

    /// The starts of tokens whose text is `word`: the word's bytes from the start, and the
    /// token's first last byte at its length.
    ///
    /// Each term is brought to the word's delay once, from its own value: the carried words are
    /// those of `start`, `last` and the letters, shared by every keyword (a chain, each step
    /// aligned in turn, would carry a new value per letter). The terms are not cleaned before
    /// the stream: `start`, ANDed with each, is 0 there.
    pub fn keyword(&mut self, start: Val, last: Val, word: &[u8]) -> Result<Val, String> {
        if word.is_empty() {
            return Err("an empty keyword".into());
        }
        let later = |x: Val, j: usize| Val { d: x.d + j as u8, set: None, ..x };
        let mut terms: Vec<(Val, bool)> = Vec::new();
        for (j, &b) in word.iter().enumerate() {
            terms.push((later(self.leaf(BlockOp::Eq(b), b == 0), j), true));
            terms.push((later(last, j), j + 1 == word.len()));
        }
        let d = terms.iter().map(|t| t.0.d).max().unwrap_or(0).max(start.d);
        if d > MAX_DELAY {
            return Err(format!("keyword {}: a delay of {d} bytes", String::from_utf8_lossy(word)));
        }
        let mut out = self.at(start, d)?;
        for (t, holds) in terms {
            let x = self.at(t, d)?;
            out = self.op(if holds { BlockOp::And(out, x) } else { BlockOp::AndNot(out, x) });
        }
        Ok(Val { v: out, d, dirty: start.dirty, set: None })
    }

    /// `aligned(a, k)`: the bytes of `a` at offsets 0, k, 2k, … in their run. For each residue
    /// `r` of a run's start mod `k`, a fill over the runs that start there, kept at the
    /// positions of the same residue. `k` divides 64, so the residues repeat every word.
    pub fn aligned(&mut self, a: Val, k: u64) -> Result<Val, String> {
        // Each residue's bytes: a 64-bit pattern repeated over the stream where `k` divides 64,
        // else `phase k r`, a residue of the position.
        if k == 0 || k > 63 && k != 64 {
            return Err(format!("aligned by {k}: from 1 to 63, or 64"));
        }
        if k == 1 {
            return Ok(a);
        }
        let p = self.prev(a, 1)?;
        let starts = self.andnot(a, p)?;
        let outside = self.not(a)?;
        let mut out = self.leaf(BlockOp::Const(0), false);
        for r in 0..k {
            let residue = if 64 % k == 0 {
                self.pattern((0..64).filter(|i| i % k == r).fold(0u64, |acc, i| acc | 1 << i))
            } else {
                self.leaf(BlockOp::Phase(k as u8, r as u8), true)
            };
            let here = self.and(starts, residue)?;
            let runs = self.fill(here, outside)?;
            let kept = self.and(runs, residue)?;
            out = self.or(out, kept)?;
        }
        self.and(out, a)
    }
}

/// The products whose union is the class `ranges`: high nibbles grouped by their set of low
/// nibbles.
pub fn products(ranges: &[(u32, u32)]) -> Vec<Product> {
    let mut out: Vec<Product> = Vec::new();
    for h in 0..16u32 {
        let lo = (0..16u32)
            .filter(|&l| ranges.iter().any(|&(a, b)| a <= (h << 4 | l) && (h << 4 | l) <= b))
            .fold(0u16, |acc, l| acc | 1 << l);
        if lo == 0 {
            continue;
        }
        match out.iter_mut().find(|p| p.1 == lo) {
            Some(p) => p.0 |= 1 << h,
            None => out.push((1 << h, lo)),
        }
    }
    out
}


/// The program with only the operations its outputs read, renumbered in order.
pub fn live(program: &BlockProgram) -> BlockProgram {
    let keep = kept(program);
    live_of(program, &keep)
}

/// Whether each operation reaches an output (`live`).
pub fn kept(program: &BlockProgram) -> Vec<bool> {
    let mut keep = vec![false; program.ops.len()];
    for &v in &program.outputs {
        keep[usize::from(v)] = true;
    }
    // A group is kept whole once one of its carries is: each next is read by the next round.
    loop {
        for at in (0..program.ops.len()).rev() {
            if keep[at] {
                for v in program.ops[at].operands() {
                    keep[usize::from(v)] = true;
                }
            }
        }
        let mut grew = false;
        // A walk's values are kept with its group: the walk makes them all.
        for walk in &program.walks {
            if walk.made().iter().any(|&v| keep[usize::from(v)]) {
                for v in walk.made().into_iter().chain(walk.reads()) {
                    grew |= !keep[usize::from(v)];
                    keep[usize::from(v)] = true;
                }
            }
        }
        for group in &program.rounds {
            if group.iter().any(|&(c, _)| keep[usize::from(c)]) {
                for &(c, x) in group {
                    for v in [c, x] {
                        grew |= !keep[usize::from(v)];
                        keep[usize::from(v)] = true;
                    }
                }
            }
        }
        if !grew {
            break;
        }
    }
    keep
}

pub fn live_of(program: &BlockProgram, keep: &[bool]) -> BlockProgram {
    let mut number = vec![u16::MAX; program.ops.len()];
    let mut out = BlockProgram { tables: program.tables.clone(), delay: program.delay, ..BlockProgram::default() };
    for (at, op) in program.ops.iter().enumerate() {
        if keep[at] {
            number[at] = out.ops.len() as u16;
            out.ops.push(op.renumbered(|v| number[usize::from(v)]));
        }
    }
    let n = |v: u16| number[usize::from(v)];
    out.outputs = program.outputs.iter().map(|&v| n(v)).collect();
    out.output_delays = program.output_delays.clone();
    out.extracts = program.extracts.iter().map(|&v| n(v)).collect();
    out.totals = program.totals.iter().map(|&v| n(v)).collect();
    out.masks = program.masks.iter().map(|&v| n(v)).collect();
    let kept_groups: Vec<usize> = (0..program.rounds.len()).filter(|&g| keep[usize::from(program.rounds[g][0].0)]).collect();
    out.rounds = kept_groups.iter().map(|&g| program.rounds[g].iter().map(|&(c, x)| (n(c), n(x))).collect()).collect();
    out.walks = program
        .walks
        .iter()
        .filter_map(|w| {
            let group = kept_groups.iter().position(|&g| g == usize::from(w.group))? as u16;
            let kinds = w.kinds.iter().map(|k| crate::block::WalkKind { opener: n(k.opener), closer: n(k.closer), open: n(k.open), fill: n(k.fill), empty: k.empty.map(|(e, m)| (n(e), m)), or: k.or.map(|(c, m)| (n(c), m)), ..*k }).collect();
            Some(crate::block::Walk { group, hidden: n(w.hidden), kinds })
        })
        .collect();
    out.records = program.records.clone();
    out
}

pub fn rows(program: &BlockProgram) -> String {
    let mut out = String::new();
    crate::block::block_rows(&mut out, 0, program);
    out
}

/// A statement's kind and name, for `Compiled::costs`.
pub fn cost_name(stmt: &Stmt) -> String {
    let name = match stmt {
        Stmt::Mask(n, _) | Stmt::Number(n, _) | Stmt::Error(n, _) | Stmt::TokenError(n, _) | Stmt::Escape(n, _) => n.clone(),
        Stmt::Count { name, .. } => name.clone(),
        Stmt::Token { kind, .. } | Stmt::Mark { kind, .. } | Stmt::TokenMark { kind, .. } => kind.clone(),
        _ => String::new(),
    };
    format!("{} {name}", statement(stmt).to_lowercase())
}

pub fn statement(stmt: &Stmt) -> String {
    let text = format!("{stmt:?}");
    text.split(['(', ' ', '{']).next().unwrap_or("?").to_owned()
}

/// Whether a keyword gives kind `name`.
fn is_keyword_kind(keywords: &[(String, Vec<(Vec<u8>, Option<String>)>, bool)], name: &str) -> bool {
    keywords.iter().any(|(_, words, _)| words.iter().any(|(w, alias)| alias.as_deref().map_or(w.as_slice() == name.as_bytes(), |a| a == name)))
}

/// The row whose tokens hold every bit of `a`, proved: `a` at its starts, or inside a
/// `fill(s, e)` whose runs are its tokens (its start `s`, its last the run's next byte).
fn token_row(low: &mut Lowering, a: Val, rows: &[bitlex_tier2::Row], token_rows: &[(String, Val, Val)]) -> Result<Option<usize>, String> {
    let proved = |low: &mut Lowering, x: Val| low.never(x) || low.settled_never(&mut bitlex_prove::Induction::new(&[], 4, 1000), x);
    let grammar_rows: Vec<usize> = (0..rows.len()).filter(|&r| !low.extra.contains(&token_rows[r].0) && !row_kinds(&rows[r]).is_empty()).collect();
    // At the row's first bytes, or at its last (a check reads `A` from a token's start to its
    // end: an escape's `u`, its last byte, decision 0399).
    for &r in &grammar_rows {
        for edge in [token_rows[r].1, token_rows[r].2] {
            let off = low.andnot(a, edge)?;
            if proved(low, off) {
                return Ok(Some(r));
            }
        }
    }
    let mut fills: Vec<Val> = low.names.values().copied().filter(|x| matches!(low.program.ops[usize::from(x.v)], BlockOp::Fill(..))).collect();
    fills.sort_by_key(|x| x.v);
    fills.dedup_by_key(|x| x.v);
    for fv in fills {
        let BlockOp::Fill(s, _) = low.program.ops[usize::from(fv.v)] else { continue };
        let inside = low.andnot(a, fv)?;
        if !proved(low, inside) {
            continue;
        }
        let sv = Val { v: s, d: fv.d, dirty: true, set: None };
        let pf = low.prev(fv, 1)?;
        let run_last = low.andnot(pf, fv)?;
        for &r in &grammar_rows {
            let (xs, xl) = (low.xor(token_rows[r].1, sv)?, low.xor(token_rows[r].2, run_last)?);
            if proved(low, xs) && proved(low, xl) {
                return Ok(Some(r));
            }
        }
    }
    Ok(None)
}

/// `op` with its operands `base` later: an op of a list appended to another.
fn offset_op(op: &crate::program::LocalOp, base: u16) -> crate::program::LocalOp {
    use crate::program::LocalOp as L;
    match *op {
        L::Not(x) => L::Not(x + base),
        L::Prev(x, k) => L::Prev(x + base, k),
        L::And(x, y) => L::And(x + base, y + base),
        L::Or(x, y) => L::Or(x + base, y + base),
        L::Xor(x, y) => L::Xor(x + base, y + base),
        L::AndNot(x, y) => L::AndNot(x + base, y + base),
        L::Fill(x, y) => L::Fill(x + base, y + base),
        ref other => other.clone(),
    }
}

/// Two token's checks of a row as one: `b`'s ops after `a`'s, each test at the most delay (a
/// `prev` by the difference; the places it shifts in lie below the delay, which are not
/// tested), ORed. Ops both make (byte sets, `start`, `last`) are made once.
fn merge_token_checks(a: &crate::program::LocalCheck, b: &crate::program::LocalCheck) -> crate::program::LocalCheck {
    use crate::program::LocalOp as L;
    let mut ops = a.ops.clone();
    let mut map: Vec<u16> = Vec::with_capacity(b.ops.len());
    for op in &b.ops {
        let op = match *op {
            L::Not(x) => L::Not(map[usize::from(x)]),
            L::Prev(x, k) => L::Prev(map[usize::from(x)], k),
            L::And(x, y) => L::And(map[usize::from(x)], map[usize::from(y)]),
            L::Or(x, y) => L::Or(map[usize::from(x)], map[usize::from(y)]),
            L::Xor(x, y) => L::Xor(map[usize::from(x)], map[usize::from(y)]),
            L::AndNot(x, y) => L::AndNot(map[usize::from(x)], map[usize::from(y)]),
            L::Fill(x, y) => L::Fill(map[usize::from(x)], map[usize::from(y)]),
            ref other => other.clone(),
        };
        let k = match ops.iter().position(|o| *o == op) {
            Some(k) => k as u16,
            None => {
                ops.push(op);
                ops.len() as u16 - 1
            }
        };
        map.push(k);
    }
    let d = a.delay.max(b.delay);
    let (ta, tb) = (a.ops.len() as u16 - 1, map[b.ops.len() - 1]);
    let at = |ops: &mut Vec<L>, x: u16, k: u8| {
        if k == 0 {
            x
        } else {
            ops.push(L::Prev(x, k));
            ops.len() as u16 - 1
        }
    };
    let sa = at(&mut ops, ta, d - a.delay);
    let sb = at(&mut ops, tb, d - b.delay);
    ops.push(L::Or(sa, sb));
    crate::program::LocalCheck { mask: 0, delay: d, ops, token: true, back: a.back.max(b.back) }
}

/// Whether two token checks' ops mean the same in a frame that begins earlier (decision 0428):
/// no `fill` (from the frame's first place) and no constant but none and all; each other op is
/// the same at the token's places wherever the frame begins.
fn mergeable(a: &crate::program::LocalCheck, b: &crate::program::LocalCheck) -> bool {
    use crate::program::LocalOp as L;
    a.back == b.back || a.ops.iter().chain(&b.ops).all(|op| !matches!(op, L::Fill(..)) && !matches!(op, L::Const(c) if *c != 0 && *c != u64::MAX))
}

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    /// Compile with no walk lemmas (`Lowering::induction`), for the tests' oracle: the program they prove smaller
    /// reads every file as this one does.
    pub static NO_WALK_LEMMAS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
thread_local! {
    /// Compile with tier 1's lasts as before decision 0452, for the tests' oracle.
    pub static NO_GAP_ENDS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether every token ends at the next token's start less the skip's bytes before it (or at
/// the file's end less them), as simdjson's stage 2 finds a scalar's end (decision 0452):
/// proved, the skip's value; tier 1 then makes no extract of the tokens' last bytes. With `K`
/// the skip, `S` every row's starts, it holds when no last of a row is in `K`, and after a last
/// of a row, and after a byte of `K`, comes a byte of `K`, a start, or the file's end.
fn gap_ends(low: &mut Lowering, token_rows: &[(String, Val, Val)]) -> Result<Option<Val>, String> {
    let (Some(skip), Some(first)) = (low.skip, token_rows.first()) else { return Ok(None) };
    #[cfg(test)]
    if NO_GAP_ENDS.get() {
        return Ok(None);
    }
    let mut starts = first.1;
    for t in &token_rows[1..] {
        starts = low.or(starts, t.1)?;
    }
    // Where the byte after is in the file, a skip byte or a start not.
    let w0 = low.within_val(0)?;
    let ok = low.or(skip, starts)?;
    let bad = low.andnot(w0, ok)?;
    let after_bad = low.next(bad, 1)?;
    // A place the fallback refuses (an error, an escape, a byte no token holds) refuses the
    // file, so the ends there are not read: the claims need hold only before the first byte
    // refused, up to the one after the claim's (a `fill` from a refusal on, to no end).
    let mut refused = low.leaf(BlockOp::Const(0), false);
    for f in low.fallbacks.clone() {
        refused = low.or(refused, f)?;
    }
    let refused_next = low.next(refused, 1)?;
    let none = low.leaf(BlockOp::Const(0), false);
    let refused = low.fill(refused_next, none)?;
    let after_bad = low.andnot(after_bad, refused)?;
    // The claims: no skip byte before a byte that is neither, no start in the skip, and each
    // row's last neither in the skip nor before such a byte.
    let from_skip = low.and(skip, after_bad)?;
    let skip_starts = low.and(skip, starts)?;
    let skip_starts = low.andnot(skip_starts, refused)?;
    let mut claims = vec![from_skip, skip_starts];
    for (_, _, l) in token_rows {
        let in_skip = low.and(*l, skip)?;
        claims.push(low.andnot(in_skip, refused)?);
        claims.push(low.and(*l, after_bad)?);
    }
    // One question (decision 0488): whether they all never hold, a function of their union's
    // cone alone (the lemmas below are made from the regions it reads), so keyed by it. The
    // prover's settings are in the key.
    let mut all = claims[0];
    for &c in &claims[1..] {
        all = low.or(all, c)?;
    }
    let prover = low.induction(8, 100_000);
    let key = bitlex_prove::question(&low.proof_ops(), &low.program.tables, &low.program.rounds, all.v, all.d, &prover.asker("gap ends"));
    let holds = match low.proofs.known.get(&key) {
        Some(&bitlex_prove::Answer::Never(b)) => b,
        _ => {
            let b = gap_claims(low, prover, &claims, all, refused, w0)?;
            low.proofs.made.push((key, bitlex_prove::Answer::Never(b)));
            b
        }
    };
    Ok(holds.then_some(skip))
}

/// `gap_ends`' claims, each zero everywhere: by the plain search, then `prover`, then by
/// induction on itself (`never_inductive`). A claim not proved so is asked again with lemmas of
/// two regions (decision 0453), made only then and only of the regions `all` (the claims'
/// union) reads: one prover for all, so the lemmas serve every claim after.
fn gap_claims(low: &mut Lowering, prover: bitlex_prove::Induction, claims: &[Val], all: Val, refused: Val, w0: Val) -> Result<bool, String> {
    let settled = RefCell::new(prover);
    let never = |low: &Lowering, x: Val| {
        let ops = low.proof_ops();
        bitlex_prove::never(&ops, &low.program.tables, x.v, x.d) || settled.borrow_mut().never(&ops, &low.program.tables, x.v, x.d) || settled.borrow_mut().never_inductive(&ops, &low.program.tables, x.v, x.d)
    };
    let mut left: Vec<Val> = Vec::new();
    for &c in claims {
        let ops = low.proof_ops();
        if bitlex_prove::never(&ops, &low.program.tables, c.v, c.d) || settled.borrow_mut().never(&ops, &low.program.tables, c.v, c.d) {
            continue;
        }
        // A real stream has a bit of it (the bounded solve's, `Witnesses`): no lemma or
        // induction proves it, and the claims fail.
        if settled.borrow_mut().refutes(&ops, &low.program.tables, c.v, c.d) {
            return Ok(false);
        }
        if !settled.borrow_mut().never_inductive(&ops, &low.program.tables, c.v, c.d) {
            left.push(c);
        }
    }
    if left.is_empty() {
        return Ok(true);
    }
    // A claim a stream that begins within the window breaks (decision 0479): no lemma rules a
    // stream out, so none is made.
    let ops = low.proof_ops();
    if left.iter().any(|c| settled.borrow_mut().refuted(&ops, &low.program.tables, c.v, c.d)) {
        return Ok(false);
    }
    // Lemmas: two regions (`fill`s, `toggle`s) at one byte only where the file is refused
    // before, as a comment opened inside another, or a quote inside a comment, escapes there. A
    // token's last inside both is then excused at any distance from that refusal.
    let carried = |low: &Lowering, x: u16| matches!(low.program.ops[usize::from(x)], BlockOp::Fill(..) | BlockOp::Toggle(..));
    let read = bitlex_prove::carried_in(&low.proof_ops(), all.v);
    let mut fills: Vec<Val> = low.names.values().copied().filter(|x| carried(low, x.v) && read.binary_search(&x.v).is_ok()).collect();
    fills.sort_by_key(|x| x.v);
    fills.dedup_by_key(|x| x.v);
    for (i, &a) in fills.iter().enumerate() {
        for &b in &fills[i + 1..] {
            let both = low.and(a, b)?;
            let both = low.andnot(both, refused)?;
            let both = low.and(both, w0)?;
            let proved = settled.borrow_mut().never_inductive(&low.proof_ops(), &low.program.tables, both.v, both.d);
            // Kept over the carries themselves, each at its lag in `both` (a value `k` bytes less
            // late is read `k` lags on), where the window's edge leaves them free: the claim as
            // proved, inside the stream (`within 0`) included.
            if proved && carried(low, refused.v) {
                let lag = |x: Val| u32::from(both.d - x.d);
                settled.borrow_mut().add_lemma(vec![(a.v, lag(a), true), (b.v, lag(b), true), (refused.v, lag(refused), false), (w0.v, lag(w0), true)]);
            }
        }
    }
    Ok(left.into_iter().all(|c| never(low, c)))
}

/// The grammar row whose tokens' last bytes hold every bit of `a`, as the prover shows it.
fn last_row(low: &mut Lowering, a: Val, rows: &[bitlex_tier2::Row], token_rows: &[(String, Val, Val)]) -> Result<Option<usize>, String> {
    let grammar_rows: Vec<usize> = (0..rows.len()).filter(|&r| !low.extra.contains(&token_rows[r].0) && !row_kinds(&rows[r]).is_empty()).collect();
    for r in grammar_rows {
        let off = low.andnot(a, token_rows[r].2)?;
        if bitlex_prove::never(&low.proof_ops(), &low.program.tables, off.v, off.d) || bitlex_prove::never_unrolled(&low.proof_ops(), &low.program.tables, &[], off.v, off.d, 4, 1000) {
            return Ok(Some(r));
        }
    }
    Ok(None)
}

/// The token kinds a row's tokens are taken as: its own, its keywords', its soft keywords'.
fn row_kinds(row: &bitlex_tier2::Row) -> Vec<u16> {
    let mut out: Vec<u16> = (row.kind != 0).then_some(row.kind).into_iter().chain(row.keywords.iter().map(|k| k.1)).chain(row.soft.iter().map(|k| k.1)).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// A check's byte machine (`grammar::byte_check`) as a token's check (`LocalCheck::token`,
/// decision 0398): `Lowering::check`'s masks, in the token's frame. Place 0 is the token's first
/// byte, so a state's mask is entered at `start` from the start state and at `prev(sources, 1)`
/// after it; a self-loop on bytes `L` is `fill(entered, ~L | start)`. Refused: the last byte in
/// no accepting state, of a token beginning with a byte the machine takes (another is tier 2's
/// switch's).
fn token_check_ops(accept: &[bool], arms: &[(u16, ByteSet, u16)]) -> Result<Vec<crate::program::LocalOp>, String> {
    use crate::program::LocalOp as L;
    let n = accept.len();
    let mut ins: Vec<usize> = (0..n).map(|q| arms.iter().filter(|a| usize::from(a.2) == q && a.0 != a.2).count()).collect();
    if ins[0] != 0 {
        return Err("an arm back to the start: not compiled yet".into());
    }
    let (mut order, mut ready) = (Vec::new(), vec![0usize]);
    while let Some(p) = ready.pop() {
        order.push(p);
        for a in arms.iter().filter(|a| usize::from(a.0) == p && a.0 != a.2) {
            let q = usize::from(a.2);
            ins[q] -= 1;
            if ins[q] == 0 {
                ready.push(q);
            }
        }
    }
    if order.len() != n {
        return Err("a cycle through two or more states: not compiled yet".into());
    }
    let mut ops: Vec<L> = Vec::new();
    let mut push = |ops: &mut Vec<L>, op: L| -> u16 {
        if let Some(k) = ops.iter().position(|o| *o == op) {
            return k as u16;
        }
        ops.push(op);
        ops.len() as u16 - 1
    };
    let or = |ops: &mut Vec<L>, push: &mut dyn FnMut(&mut Vec<L>, L) -> u16, a: Option<u16>, b: u16| match a {
        None => b,
        Some(a) => push(ops, L::Or(a, b)),
    };
    let start = push(&mut ops, L::Start);
    let takes = arms.iter().filter(|a| a.0 == 0).fold([0u64; 4], |s, a| std::array::from_fn(|i| s[i] | a.1[i]));
    let taken = push(&mut ops, L::Bytes(takes));
    let checked = push(&mut ops, L::And(start, taken));
    let mut masks: Vec<Option<Option<u16>>> = vec![None; n];
    for &q in &order {
        let mut groups: Vec<(ByteSet, Vec<usize>)> = Vec::new();
        for a in arms.iter().filter(|a| usize::from(a.2) == q && (a.0 != a.2 || q == 0)) {
            match groups.iter_mut().find(|g| g.0 == a.1) {
                Some(g) => g.1.push(usize::from(a.0)),
                None => groups.push((a.1, vec![usize::from(a.0)])),
            }
        }
        let (mut first, mut later): (Option<u16>, Option<u16>) = (None, None);
        for (bytes, from) in groups {
            let b = push(&mut ops, L::Bytes(bytes));
            if from.contains(&0) {
                let here = push(&mut ops, L::And(checked, b));
                first = Some(or(&mut ops, &mut push, first, here));
            }
            let mut sources: Option<u16> = None;
            for &p in from.iter().filter(|&&p| p != q) {
                if let Some(m) = masks[p].expect("a source before its target") {
                    sources = Some(or(&mut ops, &mut push, sources, m));
                }
            }
            if let Some(src) = sources {
                let back = push(&mut ops, L::Prev(src, 1));
                let here = push(&mut ops, L::And(back, b));
                later = Some(or(&mut ops, &mut push, later, here));
            }
        }
        let mut mask = match (first, later) {
            (f, None) => f,
            (f, Some(l)) => {
                let inside = push(&mut ops, L::AndNot(l, start));
                Some(or(&mut ops, &mut push, f, inside))
            }
        };
        if let (Some(m), Some(a)) = (mask, arms.iter().find(|a| usize::from(a.0) == q && a.2 == a.0)) {
            let stays = push(&mut ops, L::Bytes(a.1));
            let leaves = push(&mut ops, L::Not(stays));
            let stop = push(&mut ops, L::Or(leaves, start));
            mask = Some(push(&mut ops, L::Fill(m, stop)));
        }
        masks[q] = Some(mask);
    }
    let mut good: Option<u16> = None;
    for q in (0..n).filter(|&q| accept[q]) {
        if let Some(m) = masks[q].expect("every state") {
            good = Some(or(&mut ops, &mut push, good, m));
        }
    }
    let span = push(&mut ops, L::Fill(checked, start));
    let last = push(&mut ops, L::Last);
    let ends = match good {
        Some(g) => push(&mut ops, L::AndNot(last, g)),
        None => last,
    };
    // The last op is the check's value: pushed even where an equal op is made.
    ops.push(L::And(span, ends));
    Ok(ops)
}

/// The tree section lowered: the grammar's state machine (`grammar`), its checks added to the
/// fallback, and tier 2's words (`bitlex_tier2`), with the grammar's ids from `symbols`.
fn tree_section(symbols: &Symbols<'_>, low: &mut Lowering, token_rows: &[(String, Val, Val)]) -> Result<Tier2, String> {
    // The kinds' names, as the grammar asks for them (`Words::names`).
    let named: std::cell::RefCell<std::collections::BTreeMap<u16, String>> = Default::default();
    let id = |name: &str| {
        let k = (symbols.kind)(name).ok_or_else(|| format!("no kind `{name}` in the grammar"))?;
        named.borrow_mut().entry(k).or_insert_with(|| name.to_owned());
        Ok::<u16, String>(k)
    };
    let field_id = |name: &str| (symbols.field)(name).ok_or_else(|| format!("no field `{name}`"));
    // A token row whose every token is a keyword (`required`) has kind 0: the keyword's is its.
    let required: Vec<String> = low.keywords.iter().filter(|k| k.2).map(|k| k.0.clone()).collect();
    // The grammar's state machine (doc 18 §2). A token kind is a token row's that is not all
    // keywords, or a keyword's.
    let mark_kinds = low.marks.iter().map(|m| id(&m.0)).collect::<Result<Vec<u16>, String>>()?;
    if let Some(h) = low.hidden.iter().find(|h| !low.marks.iter().any(|m| &m.0 == *h)) {
        return Err(format!("hidden {h}: no mark of that kind"));
    }
    // A refused mark is a hidden one: skipped elsewhere.
    for s in &low.tree {
        if let Stmt::Refuse { mark, .. } = s
            && !low.hidden.contains(mark)
        {
            return Err(format!("refuse {mark}: no hidden mark of that kind"));
        }
    }
    // The readings (`reading R = …`, `reread R`): each kind's way, an extra's leaf staying as
    // it is; and the provisional fields.
    let mut reading_names: Vec<String> = Vec::new();
    let mut readings = crate::stream_ir::Readings::default();
    let remember: Vec<&String> = low.tree.iter().filter_map(|s| if let Stmt::Remember(n) = s { Some(n.iter()) } else { None }).flatten().collect();
    for s in &low.tree {
        match s {
            Stmt::Reading { name, reading } => {
                if reading_names.contains(name) {
                    return Err(format!("reading {name}: two of that name"));
                }
                reading_names.push(name.clone());
                let mut r = crate::stream_ir::Reading::default();
                for (k, what) in &reading.kinds {
                    let to = match what {
                        ::bitlex::syntax::ReadKind::Keep => crate::stream_ir::KEEP,
                        ::bitlex::syntax::ReadKind::Gone => crate::stream_ir::GONE,
                        ::bitlex::syntax::ReadKind::As(m) if remember.contains(&m) => crate::stream_ir::REMEMBERED,
                        ::bitlex::syntax::ReadKind::As(m) => id(m.as_str())?,
                    };
                    let k = id(k)?;
                    if r.kinds.iter().any(|x| x.0 == k) {
                        return Err(format!("reading {name}: a kind twice"));
                    }
                    r.kinds.push((k, to));
                }
                for e in &low.extra {
                    let e = id(e)?;
                    if !r.kinds.iter().any(|x| x.0 == e) {
                        r.kinds.push((e, e));
                    }
                }
                for (f, g) in &reading.fields {
                    r.fields.push((field_id(f)?, field_id(g)?));
                }
                r.root = match &reading.root {
                    None => None,
                    Some(::bitlex::syntax::ReadKind::Gone) => Some(crate::stream_ir::GONE),
                    Some(::bitlex::syntax::ReadKind::As(m)) => Some(id(m.as_str())?),
                    Some(::bitlex::syntax::ReadKind::Keep) => return Err(format!("reading {name}: a root kept")),
                };
                readings.maps.push(r);
            }
            Stmt::Provisional(fields) => {
                for f in fields {
                    readings.provisional.push(field_id(f)?);
                }
            }
            _ => {}
        }
    }
    let (states, mut arms, whole, by_rules) = {
        let is_token = |name: &str| {
            token_rows.iter().any(|t| t.0 == name && !required.contains(&t.0))
                || is_keyword_kind(&low.keywords, name)
                || low.soft.iter().any(|s| s.1.iter().any(|w| w.as_slice() == name.as_bytes()))
                || low.marks.iter().any(|m| m.0 == name)
        };
        let kind = |name: &str| id(name);
        // Every token kind: a token row's that is not all keywords, and each keyword's.
        let mut tokens: Vec<String> = token_rows.iter().filter(|t| !required.contains(&t.0)).map(|t| t.0.clone()).collect();
        for (_, words, _) in &low.keywords {
            for (w, alias) in words {
                let k = alias.clone().unwrap_or_else(|| String::from_utf8_lossy(w).into_owned());
                if !tokens.contains(&k) {
                    tokens.push(k);
                }
            }
        }
        tokens.retain(|t| !low.marks.iter().any(|m| &m.0 == t) && !low.extra.contains(t));
        let reading = |name: &str| reading_names.iter().position(|r| r == name).map(|r| r as u16).ok_or_else(|| format!("reread {name}: no reading of that name"));
        let ids = grammar::Ids { kind: &kind, field: &field_id, is_token: &is_token, tokens: &tokens, reading: &reading, marks: &mark_kinds };
        let machine = grammar::build_shaped(&low.tree, &ids, false)?;
        let rules = match low.shape {
            Shape::Machine => false,
            Shape::Rules => true,
            Shape::Auto => machine.0.len() > MACHINE_STATES,
        };
        let (st, ar, wh) = if rules { grammar::build_shaped(&low.tree, &ids, true)? } else { machine };
        (st, ar, wh, rules)
    };
    low.shape = if by_rules { Shape::Rules } else { Shape::Machine };
    let settling: Vec<u16> = low.settles.iter().map(|&k| mark_kinds[k]).collect();
    grammar::without_mark_leaves(&mut arms, &mark_kinds, &settling)?;
    // A node a reading drops has no supertype: tier 2 places the supertypes by their nodes.
    let gone: Vec<u16> = readings.maps.iter().flat_map(|r| r.kinds.iter().filter(|x| x.1 == crate::stream_ir::GONE).map(|x| x.0)).collect();
    for a in &arms {
        for act in &a.acts {
            let k = match act {
                grammar::Act::Open { kind, supers, .. } if !supers.is_empty() => *kind,
                grammar::Act::Leaf { kind, supers, .. } if !supers.is_empty() => if *kind != 0 { *kind } else { a.token },
                _ => continue,
            };
            if gone.contains(&k) {
                return Err(format!("a reading drops kind {k}, which has a supertype"));
            }
        }
    }
    // Each check, a byte machine over its tokens' bytes (doc 18 §4). Tier 2 runs it at each of
    // its tokens, over the token's own bytes, as simdjson's stage 2 parses a number (decision
    // 0398): a token's check. Where a keyword of the row begins with a byte the machine takes,
    // its tokens are told apart in tier 1: the machine is masks over the whole file there, with
    // the rest of the block program, and a token it refuses joins the fallback. Tier 2 keeps
    // only the bytes a token passing it begins with.
    let mut check_of: HashMap<usize, ByteSet> = HashMap::default();
    let mut token_checks: Vec<(usize, Vec<crate::program::LocalOp>)> = Vec::new();
    for (kind, g) in low.checks.clone() {
        let token = token_rows.iter().position(|t| t.0 == kind).ok_or_else(|| format!("check {kind}: no token row of that kind"))?;
        let (accept, arms) = grammar::byte_check(&g, &low.classes).map_err(|e| format!("check {kind}: {e}"))?;
        let first = arms.iter().filter(|a| a.0 == 0).fold([0u64; 4], |s, a| [s[0] | a.1[0], s[1] | a.1[1], s[2] | a.1[2], s[3] | a.1[3]]);
        let words: Vec<Vec<u8>> = low.keywords.iter().filter(|k| k.0 == kind).flat_map(|k| k.1.iter().map(|w| w.0.clone())).collect();
        let taken_word = words.iter().any(|w| first[usize::from(w[0] / 64)] >> (w[0] % 64) & 1 == 1);
        if taken_word {
            let (_, start, last) = token_rows[token].clone();
            let refused = low.check(start, last, &accept, &arms, &words).map_err(|e| format!("check {kind}: {e}"))?;
            low.fallbacks.push(refused);
        } else {
            token_checks.push((token, token_check_ops(&accept, &arms).map_err(|e| format!("check {kind}: {e}"))?));
        }
        check_of.insert(token, first);
    }
    let mut rows = Vec::new();
    for (r, (kind, start, last)) in token_rows.iter().enumerate() {
        let mut keywords = Vec::new();
        for (k, words, _) in low.keywords.iter().filter(|k| &k.0 == kind) {
            let _ = k;
            for (word, alias) in words {
                let name = alias.clone().unwrap_or_else(|| String::from_utf8_lossy(word).into_owned());
                keywords.push((word.clone(), id(&name)?));
            }
        }
        let mut soft = Vec::new();
        for (_, words) in low.soft.iter().filter(|s| &s.0 == kind) {
            for word in words {
                soft.push((word.clone(), id(&String::from_utf8_lossy(word))?));
            }
        }
        rows.push(bitlex_tier2::Row {
            kind: if required.contains(kind) { 0 } else { id(kind)? },
            soft,
            one_byte: start.v == last.v && start.d == last.d,
            first: low.first_bytes(*start),
            keywords,
            check: check_of.get(&r).copied(),
            mask: None,
            moved: false,
        });
    }
    // `RERAIL_TIER2_SIZES` (`bitlex_tier2::lower`): each row's first bytes, and each mark's, on
    // stderr, as what makes tier 2's tests.
    if std::env::var_os("RERAIL_TIER2_SIZES").is_some() {
        for (r, row) in rows.iter().enumerate() {
            eprintln!("row {r} {}: {} first bytes", token_rows[r].0, row.first.iter().map(|w| w.count_ones()).sum::<u32>());
        }
    }
    // Two rows that can begin with one byte and can start at one byte: that byte sends the
    // file to tree-sitter, as the evaluator's "both start at byte" does. A pair whose starts
    // are apart, proved (`never`: `inside` and `~inside`, say), costs nothing.
    let shares = |a: usize, b: usize| (0..4).any(|i| rows[a].first[i] & rows[b].first[i] != 0);
    let ops_before = low.program.ops.len();
    let mut pairs = Vec::new();
    // The fixed points' equations prove most apart: one row's tokens in a region, the other's
    // outside it (a string's content and a comment's opener).
    let mut settled = bitlex_prove::Induction::new(&low.program.rounds, 8, 100_000);
    // A walk's regions (`block::Walk`): a byte is one kind's at most, as the walk opens a region
    // only outside the others. That holds of all the kinds together, not of two alone (each
    // pair is apart because the others are): one value, any two kinds' places at one byte,
    // proved never by induction on itself; then each pair a lemma of the proofs after.
    let walks = low.program.walks.clone();
    for w in &walks {
        let val = |low: &Lowering, v: u16| low.names.values().copied().find(|x| x.v == v);
        let places: Vec<(usize, Val)> = w.kinds.iter().enumerate().flat_map(|(k, kind)| [(k, kind.open), (k, kind.fill)]).filter_map(|(k, v)| val(low, v).map(|x| (k, x))).collect();
        let mut pairs = Vec::new();
        for (i, &(ka, a)) in places.iter().enumerate() {
            for &(kb, b) in &places[i + 1..] {
                if ka != kb {
                    pairs.push((a, b, low.and(a, b)?));
                }
            }
        }
        let Some(mut any) = pairs.first().map(|p| p.2) else { continue };
        for p in &pairs[1..] {
            any = low.or(any, p.2)?;
        }
        if settled.never_inductive(&low.proof_ops(), &low.program.tables, any.v, any.d) {
            for (a, b, _) in pairs {
                let lag = |x: Val| u32::from(any.d - x.d);
                settled.add_lemma(vec![(a.v, lag(a), true), (b.v, lag(b), true)]);
            }
        }
    }
    for a in 0..rows.len() {
        for b in a + 1..rows.len() {
            if shares(a, b) {
                let both = low.and(token_rows[a].1, token_rows[b].1)?;
                if !low.either_never(&mut settled, both) {
                    low.fallbacks.push(both);
                    pairs.push((a, b));
                }
            }
        }
    }
    if std::env::var_os("RERAIL_TIER2_SIZES").is_some() {
        let names: Vec<String> = pairs.iter().map(|&(a, b)| format!("{a}{}/{b}{}", token_rows[a].0, token_rows[b].0)).collect();
        eprintln!("rows that share a first byte: {} pairs to the fallback, {} operations: {}", pairs.len(), low.program.ops.len() - ops_before, names.join(" "));
    }
    // The extras (grammar::with_extras): taken in the token step (`TOKEN_EXTRAS`), as the lean
    // parser places comments, where the records' codes have a mark's bit for them: one bit
    // every extra's token has (the extra's mark), and one more for each kind after the first,
    // which tells its leaf's kind (Kotlin's line and block comments); so a mark for each kind,
    // within `records::MARKS`. Else an arm in every state, which tells an extra from a token
    // beginning with the same byte by their rows' start masks (`ROW`).
    // The marks the code holds (decision 0407): each up to the last that tier 1 makes, or that
    // refuses wherever it holds (tested once a token by its bit, `TokenMark::refuses`). A token
    // mark after them holds no bit: tier 2 tests its tokens alone, as the lean parser peeks.
    let refuses_alone = |k: usize| !low.hidden.contains(&low.marks[k].0) && arms.iter().all(|a| a.token != mark_kinds[k]);
    // Without the token loop's records, every mark is a mask tier 2 reads (`any_mark` below).
    let records = (!low.token_rules.is_empty() || !low.token_reads.is_empty()) && low.skip.is_some();
    let code_marks = if records { (0..low.marks.len()).rev().find(|&k| !low.token_marks.contains_key(&k) || refuses_alone(k)).map_or(0, |k| k + 1) } else { low.marks.len() };
    let refusing_alone: Vec<bool> = (0..low.marks.len()).map(refuses_alone).collect();
    let token_extras = !low.extra.is_empty() && code_marks.min(crate::records::MARKS) + low.extra.len() <= crate::records::MARKS;
    let (states, arms) = {
        let extras = low.extra.iter().map(|k| id(k)).collect::<Result<Vec<u16>, String>>()?;
        let apart = |_: u16, _: u16| true;
        grammar::with_extras(states, arms, &extras, &apart, whole, &mark_kinds, token_extras)?
    };
    // Each mark's bits at the grammar's token starts (not the extras'): a token that starts
    // with a bit since the start before it, `starts & (m | prev(fill(m & ~starts, starts)))`.
    // Its mask is the program's `k`.
    // A row read inside a region (`inside`, a string's content) is left out: marks are of the
    // grammar's tokens in code, and none holds inside a region read in parts (`Reader::quiet`).
    let mut grammar_starts: Option<Val> = None;
    for (r, (kind, start, _)) in token_rows.iter().enumerate() {
        let is_extra = low.extra.contains(kind) || low.inside.get(r).copied().unwrap_or(false);
        if !is_extra {
            grammar_starts = Some(match grammar_starts {
                Some(g) => low.or(g, *start)?,
                None => *start,
            });
        }
    }
    // A mark of one word: a term of its `&`s that holds, at the grammar's token starts, exactly
    // where the token's text is one of the rows' words, and nowhere else (both proved,
    // `never`). That term is the token's record's word, which tier 2 compares (the lean
    // parser's `is_id(W)`), and leaves tier 1: its letters' masks go where nothing else reads
    // them. The words: every row's keywords and soft keywords.
    let mut candidates: Vec<Vec<u8>> = rows.iter().flat_map(|r| r.keywords.iter().map(|k| k.0.clone()).chain(r.soft.iter().map(|k| k.0.clone()))).collect();
    candidates.sort();
    candidates.dedup();
    fn terms<'e>(e: &'e Expr, out: &mut Vec<&'e Expr>) {
        match e {
            Expr::And(a, b) => {
                terms(a, out);
                terms(b, out);
            }
            e => out.push(e),
        }
    }
    let mut mark_masks = Vec::new();
    let mut marks = Vec::new();
    // The local checks (`CHECK`): each its token kinds, the mark whose refusing levels alone
    // check it (or none: everywhere), its mask `A` and its check.
    let mut checks: Vec<(Vec<u16>, Option<u16>, Val, crate::program::LocalCheck)> = Vec::new();
    // With the fixed points' equations and the lemmas proved on the way, kept for every mark.
    let mut settled = low.induction(8, 100_000);
    // Where the token loop finds the starts (decision 0177), a mark is no start mask: a mark of
    // one word is its other terms at the token's first byte (none: the word alone), a token mark
    // the word alone, any other its own mask, which the records lift to the next grammar token
    // (`Records::gaps`).
    let looped = (!low.token_rules.is_empty() || !low.token_reads.is_empty()) && low.skip.is_some();
    let mut gaps = 0u32;
    let mut alone = vec![false; low.marks.len()];
    let mut token_alone = 0u32;
    let mut settle_marks: Vec<usize> = Vec::new();
    for (k, (kind, m, end)) in low.marks.clone().into_iter().enumerate() {
        let gs = grammar_starts.ok_or("marks with no grammar token")?;
        // A token mark: its bit at every grammar token (its first element's word's, where it is
        // one word), the rest tier 2's; its bytes, its first element's.
        if let Some((before, seq)) = low.token_marks.get(&k).cloned() {
            if k >= code_marks {
                // Past the code's: no bit, no mask.
                let none = low.leaf(BlockOp::Const(0), false);
                mark_masks.push(none);
            } else if looped && matches!(seq[0].as_slice(), [TokenElem::Word(_)]) {
                alone[k] = true;
                let none = low.leaf(BlockOp::Const(0), false);
                mark_masks.push(none);
            } else if looped && !refusing_alone[k] {
                // In the code only because a mark after it is: no bit, its tokens tier 2's alone
                // (`Records::alone`), as a token mark's past the code's. A bit at every token
                // would be set on the CPU after the tokens, a search a bit (`late_marks`), and a
                // walk over every block's outputs.
                token_alone |= 1 << k;
                let none = low.leaf(BlockOp::Const(0), false);
                mark_masks.push(none);
            } else if seq[0].iter().all(|c| matches!(c, TokenElem::Kind(_))) {
                // A first element of kinds: its bit at those rows' grammar starts, not at every
                // token's (tier 2 tests the whole run either way).
                let mut own: Option<Val> = None;
                for c in &seq[0] {
                    let TokenElem::Kind(name) = c else { continue };
                    for (_, s, _) in token_rows.iter().filter(|t| &t.0 == name) {
                        own = Some(match own {
                            Some(o) => low.or(o, *s)?,
                            None => *s,
                        });
                    }
                }
                let own = match own {
                    Some(o) => low.and(o, gs)?,
                    None => low.leaf(BlockOp::Const(0), false),
                };
                mark_masks.push(own);
            } else {
                mark_masks.push(gs);
            }
            let mut first: ByteSet = [0; 4];
            for c in &seq[0] {
                match c {
                    TokenElem::Word(w) => first[usize::from(w[0]) / 64] |= 1 << (w[0] % 64),
                    TokenElem::Kind(name) => {
                        for r in (0..rows.len()).filter(|&r| &token_rows[r].0 == name) {
                            for i in 0..4 {
                                first[i] |= rows[r].first[i];
                            }
                        }
                    }
                    // A token's bytes are the same with or without the mark: tier 2 tests it.
                    TokenElem::Without(_) | TokenElem::With(_) => {}
                    // Refused by the syntax in a mark's first element.
                    TokenElem::Not(_) | TokenElem::End | TokenElem::Many(..) | TokenElem::Starting(_) => return Err("a `~`, an `end`, a run or a `starting` as a token mark's first token".into()),
                }
            }
            let word = match seq[0].as_slice() {
                [TokenElem::Word(w)] => Some(w.clone()),
                _ => None,
            };
            let words = seq[0].iter().map(|c| if let TokenElem::Word(w) = c { Some(w.clone()) } else { None }).collect();
            marks.push(bitlex_tier2::Mark { passes: 0, settle: None, drop: None, lose: None, refuse: None, refuse_taken: None, lose_flag: 0, ends: false, drops: vec![], loses: vec![], refuses: vec![], refuses_taken: vec![], kind: mark_kinds[k], mask: k as u16, hidden: low.hidden.contains(&kind), end, word, first, words, tokens: Some((before, seq)), nexts: low.mark_next.get(&k).cloned().unwrap_or_default(), unless: vec![] });
            continue;
        }
        let mut expr = low.mark_exprs[k].clone();
        let mut m = m;
        // A hidden mark only refused (`refuse M in L`): each of its alternatives `A & L`, `L` its
        // look-ahead (its terms `next` makes late) and local, `A` in one row's tokens, is a check
        // at the arms of the refusing levels that take those tokens (`CHECK`, gated). A bit in a
        // token lifts to the token or the next one, both inside the level (`grammar::refusing`).
        let refused = low.tree.iter().any(|s| matches!(s, Stmt::Refuse { mark, .. } if *mark == kind));
        // Its token tests (decision 0177): a hidden mark only refused checks a local test in
        // tier 2 at its kind's tokens, in its refusing levels (`kind_checks`); any other test is
        // the token loop's, a bit of the mark.
        for (_, after, tkind, count, least, e) in low.mark_tests.iter().filter(|t| t.0 == k).cloned().collect::<Vec<_>>() {
            let split = if low.hidden.contains(&kind) && refused { low.local_split(count, least, &e)? } else { None };
            match split {
                Some((a, l)) => low.kind_checks.push((tkind, Some(k), a, l)),
                None => {
                    let value = low.expr(&e)?;
                    // A count of one of a mask that holds where each of the kind's tokens
                    // starts: it holds at every token (`least` 0, `TokenCheck::always`), as a
                    // lexer knows a comment it reads has its opener, and no count is made.
                    // A read kind's tokens (`read`): its arms' openers, where a reader reads
                    // them; another's, those of its rows.
                    let rows: Vec<Val> = match low.token_reads.get(&tkind) {
                        Some(arms) => arms.iter().map(|a| a.0).collect(),
                        None => token_rows.iter().filter(|r| r.0 == tkind).map(|r| r.1).collect(),
                    };
                    let least = if count && least == 1 && !rows.is_empty() && rows.iter().all(|&s| low.implies(s, value)) { 0 } else { least };
                    let mark = u8::try_from(k).ok().filter(|&m| usize::from(m) < crate::records::MARKS).ok_or("a mark with token tests past the marks a code holds")?;
                    let then = if after { crate::records::CheckThen::After(mark) } else { crate::records::CheckThen::At(mark) };
                    low.token_escapes.push((tkind, count, value, least, then));
                }
            }
        }
        if low.hidden.contains(&kind) && refused && !matches!(low.mark_exprs[k], Expr::Empty) {
            fn ors<'e>(e: &'e Expr, out: &mut Vec<&'e Expr>) {
                match e {
                    Expr::Or(a, b) => {
                        ors(a, out);
                        ors(b, out);
                    }
                    e => out.push(e),
                }
            }
            let mut alts = Vec::new();
            ors(&expr, &mut alts);
            let mut kept: Vec<Expr> = Vec::new();
            let all = alts.len();
            for alt in alts {
                let mut ts = Vec::new();
                terms(alt, &mut ts);
                let (mut a, mut l): (Option<Val>, Option<Val>) = (None, None);
                for t in &ts {
                    let x = low.expr(t)?;
                    let side = if x.d > 0 && low.local_ops(x).is_some() { &mut l } else { &mut a };
                    *side = Some(match *side {
                        Some(y) => low.and(y, x)?,
                        None => x,
                    });
                }
                let moved = match (a, l) {
                    (Some(a), Some(l)) => match (token_row(low, a, &rows, token_rows)?, low.local_ops(l)) {
                        (Some(r), Some(ops)) => {
                            checks.push((row_kinds(&rows[r]), Some(mark_kinds[k]), a, crate::program::LocalCheck { mask: 0, delay: l.d, ops, token: false, back: 0 }));
                            true
                        }
                        _ => false,
                    },
                    _ => false,
                };
                if !moved {
                    kept.push(alt.clone());
                }
            }
            let kept_len = kept.len();
            if let Some(first) = kept.first().cloned() {
                let rest = kept.into_iter().skip(1).fold(first, |x, y| Expr::Or(Box::new(x), Box::new(y)));
                if kept_len != all {
                    m = low.expr(&rest)?;
                    expr = rest;
                }
            } else {
                m = low.leaf(BlockOp::Const(0), false);
                expr = Expr::Empty;
            }
        }
        let mut parts = Vec::new();
        terms(&expr, &mut parts);
        let mut word = None;
        'find: for (i, term) in parts.iter().enumerate() {
            let t = low.expr(term)?;
            // Only at the grammar's token starts.
            let elsewhere = low.andnot(t, gs)?;
            if !low.settled_never(&mut settled, elsewhere) {
                continue;
            }
            // The words whose first byte the term may be at.
            let firsts = low.settled_first_bytes(&mut settled, t);
            for w in candidates.iter().filter(|w| firsts[usize::from(w[0]) / 64] >> (w[0] % 64) & 1 == 1) {
                // A token's record pairs its start with its row's last: one row starts at a
                // byte (two that can go to the fallback), and the record pass checks the
                // streams agree. So its word is `w` where a row that can begin with `w`'s
                // first byte starts and its tokens' text is `w`.
                let mut e: Option<Val> = None;
                // The grammar's rows only: an extra's token starts no grammar token. The term holds
                // only at the grammar's starts, and so does each of these rows' start: they agree
                // everywhere, `gs` need not be read.
                let these: Vec<usize> = (0..rows.len()).filter(|&r| rows[r].first[usize::from(w[0]) / 64] >> (w[0] % 64) & 1 == 1 && !low.extra.contains(&token_rows[r].0)).collect();
                for r in these {
                    let x = low.keyword(token_rows[r].1, token_rows[r].2, w)?;
                    e = Some(match e {
                        Some(y) => low.or(y, x)?,
                        None => x,
                    });
                }
                let Some(e) = e else { continue };
                let differs = low.xor(t, e)?;
                let ok = low.settled_never(&mut settled, differs);
                if ok {
                    let mut rest = if looped { None } else { Some(gs) };
                    for (_, other) in parts.iter().enumerate().filter(|&(j, _)| j != i) {
                        let x = low.expr(other)?;
                        rest = Some(match rest {
                            Some(y) => low.and(y, x)?,
                            None => x,
                        });
                    }
                    m = match rest {
                        Some(r) => r,
                        None => {
                            alone[k] = true;
                            low.leaf(BlockOp::Const(0), false)
                        }
                    };
                    word = Some(w.clone());
                    break 'find;
                }
            }
        }
        let before = low.andnot(m, gs)?;
        // `unless after`: the gap pass's, from the grammar token before (`Records::unless`).
        if (low.unless.contains_key(&k) || low.unplaced.contains(&k)) && !(looped && word.is_none() && k < crate::records::MARKS) {
            return Err(format!("mark {kind}: `unless after` and `else escape` need the token loop and a mark of no word"));
        }
        if looped {
            // A mark of one word reads its terms at the token; any other, its own mask, lifted.
            if word.is_none() && k < crate::records::MARKS {
                gaps |= 1 << k;
            }
            mark_masks.push(m);
        } else {
            let since = low.fill(before, gs)?;
            let since = low.prev(since, 1)?;
            let any = low.or(m, since)?;
            let at = low.and(gs, any)?;
            mark_masks.push(at);
        }
        // The bytes its token can begin with: the first bytes of each row whose starts it may
        // meet (it holds only at the grammar's starts, so within those rows' starts; a row left
        // out is proved apart, `never`). A state that refuses it tests it only on those bytes.
        // Proved on `m`, the mark's own: where it holds only at the grammar's starts
        // (`m & ~starts` never), the lift to the next start adds nothing.
        let on_starts = low.settled_never(&mut settled, before);
        let mut first: ByteSet = [u64::MAX; 4];
        if on_starts {
            first = [0; 4];
            for r in 0..rows.len() {
                if low.extra.contains(&token_rows[r].0) {
                    continue;
                }
                let both = low.and(m, token_rows[r].1)?;
                if !low.settled_never(&mut settled, both) {
                    for i in 0..4 {
                        first[i] |= rows[r].first[i];
                    }
                }
            }
            // A row of many bytes (the operators'): each byte the mark is proved apart from left
            // out.
            for b in 0..=255u8 {
                if first[usize::from(b) / 64] >> (b % 64) & 1 == 0 {
                    continue;
                }
                let mut one: ByteSet = [0; 4];
                one[usize::from(b) / 64] |= 1 << (b % 64);
                let class = low.class_set(one);
                let both = low.and(m, class)?;
                if low.settled_never(&mut settled, both) {
                    first[usize::from(b) / 64] &= !(1 << (b % 64));
                }
            }
        }
        // With `before`s, it holds (as tier 2 tests it) only where one of them does from the
        // token: its first bytes are within their first tokens', and where each first token is
        // a word, those words are the only ones it holds at, as the lean parser asks `is_word`
        // where it reads the token.
        let nexts = low.mark_next.get(&k).cloned().unwrap_or_default();
        let mut words: Option<Vec<Vec<u8>>> = None;
        let positive: Vec<&Vec<TokenElem>> = nexts.iter().filter(|(unless, _)| !unless).filter_map(|(_, seqs)| seqs.first()).collect();
        let simple = |e: &Vec<TokenElem>| e.iter().all(|c| matches!(c, TokenElem::Word(_) | TokenElem::Kind(_)));
        if !positive.is_empty() && positive.iter().all(|e| simple(e)) {
            let mut by: ByteSet = [0; 4];
            let mut ws: Vec<Vec<u8>> = Vec::new();
            let mut all_words = true;
            for e in &positive {
                for c in e.iter() {
                    match c {
                        TokenElem::Word(w) => {
                            by[usize::from(w[0]) / 64] |= 1 << (w[0] % 64);
                            ws.push(w.clone());
                        }
                        TokenElem::Kind(name) => {
                            all_words = false;
                            for r in (0..rows.len()).filter(|&r| &token_rows[r].0 == name) {
                                for i in 0..4 {
                                    by[i] |= rows[r].first[i];
                                }
                            }
                        }
                        _ => unreachable!("a simple element"),
                    }
                }
            }
            for i in 0..4 {
                first[i] &= by[i];
            }
            if all_words {
                ws.sort();
                ws.dedup();
                words = Some(ws);
            }
        }
        if std::env::var_os("RERAIL_TIER2_SIZES").is_some() {
            eprintln!("mark {k} {kind}: on starts {on_starts}, word {:?}, {} first bytes, words {:?}", word.as_ref().map(|w| String::from_utf8_lossy(w).into_owned()), first.iter().map(|w| w.count_ones()).sum::<u32>(), words.as_ref().map(|ws| ws.iter().map(|w| String::from_utf8_lossy(w).into_owned()).collect::<Vec<_>>()));
        }
        let unless = low.unless.get(&k).cloned().unwrap_or_default();
        marks.push(bitlex_tier2::Mark { kind: mark_kinds[k], mask: k as u16, hidden: low.hidden.contains(&kind), end, word, first, words, tokens: None, nexts, unless, passes: 0, settle: None, drop: None, lose: None, refuse: None, refuse_taken: None, lose_flag: 0, ends: low.ends.contains(&k), drops: vec![], loses: vec![], refuses: vec![], refuses_taken: vec![] });
        // A settling mark: `SETTLE` reads its bit in the pending extras' codes, each the marks
        // of its own gap (`Records::own`), as the lean lexer's `skip` knows a line end before
        // each comment.
        if low.settles.contains(&k) {
            settle_marks.push(k);
        }
    }
    // Any mark at a token: the marks' masks joined, after them. Marks hold at few tokens (0.8%
    // of the tests of `.rr`'s corpus), so `TOKEN` reads this once and the marks are live only
    // where it holds (`TOKEN m=1+k`).
    let any_mark = match mark_masks.split_first() {
        // With the marks the records' (`looped`), tier 2 reads them from the codes.
        _ if looped => None,
        Some((&first, rest)) => {
            let mut any = first;
            for &m in rest {
                any = low.or(any, m)?;
            }
            mark_masks.push(any);
            Some(marks.len() as u16)
        }
        None => None,
    };
    for k in settle_marks {
        marks[k].settle = Some(k as u16);
    }
    // `, passes M`: each a mark after it, by order.
    for (&k, names) in &low.passes {
        for n in names {
            let m = low.marks.iter().position(|x| &x.0 == n).ok_or_else(|| format!("`passes {n}`: no such mark"))?;
            if m <= k {
                return Err(format!("`passes {n}`: a mark before it, which taking it passes already"));
            }
            marks[k].passes |= 1 << m;
        }
    }
    // The `drops`, `loses`, `refuses` and `refuses taken` questions (`Words::gap_patterns`), each
    // mark's in turn,
    // in that order: `SETTLE` names each by its first entry (`bitlex_tier2::lower` writes them
    // in this order).
    let mut entries = 0u16;
    for k in 0..marks.len() {
        if let Some(d) = low.drops.get(&k) {
            marks[k].drop = Some(entries);
            marks[k].drops = d.clone();
            entries += d.len() as u16;
        }
        if let Some(l) = low.loses.get(&k) {
            marks[k].lose = Some(entries);
            marks[k].loses = l.clone();
            entries += l.len() as u16;
        }
        if let Some(r) = low.refuses.get(&k) {
            marks[k].refuse = Some(entries);
            marks[k].refuses = r.clone();
            entries += r.len() as u16;
        }
        if let Some(r) = low.refuses_taken.get(&k) {
            marks[k].refuse_taken = Some(entries);
            marks[k].refuses_taken = r.clone();
            entries += r.len() as u16;
        }
        if marks[k].lose.is_some() || marks[k].refuse.is_some() {
            // Its flag: one past the grammar's (`flags`) and the marks' before it.
            let named = low.tree.iter().map(|s| if let Stmt::Flags(n) = s { n.len() } else { 0 }).sum::<usize>();
            let flag = named + marks[..k].iter().filter(|m| m.lose.is_some() || m.refuse.is_some()).count();
            if flag >= 16 {
                return Err(format!("mark {k}'s `loses`: more than 16 flags"));
            }
            marks[k].lose_flag = flag as u16;
        }
    }
    let row_masks = mark_masks.len() as u16;
    // The rows each state can meet, by the modes its tokens are read in (`row_modes`): tier 2
    // tells apart by their start masks (`ROW`) only the rows a state can meet that begin
    // with one byte. A first lowering finds them; their masks follow the marks'.
    // A reader's kind whose tokens share a first byte with another kind's (a comment and a `/`):
    // tier 2 meets its rows at a byte no token begins with, its own kind, which its reader gives
    // the record (`Reader::retag`), as the lean lexer's reader says what it read. No state
    // tells the two apart by a start mask (`ROW`).
    let real_first: Vec<ByteSet> = rows.iter().map(|x| x.first).collect();
    let mut moved: Vec<(usize, u8)> = Vec::new();
    // The parts' own bytes, by their kinds (`ReadPart::kind`).
    let mut part_bytes: HashMap<String, u8> = HashMap::default();
    // The closers moved (`Reader::closers`): each reader's kind, its row and its byte.
    let mut closer_rows: Vec<(String, usize, u8)> = Vec::new();
    if looped {
        // A row read inside a region (`inside`) is its reader's, met only in the region's mode
        // (`row_modes`): it begins no token at a byte's dispatch, so it takes no byte here.
        let inside = |r: usize| low.inside.get(r).copied().unwrap_or(false);
        let starts = |r: usize, b: usize| !inside(r) && real_first[r][b / 64] >> (b % 64) & 1 == 1;
        let mut free: Vec<u8> = (0..256usize).rev().filter(|&b| !(0..rows.len()).any(|r| starts(r, b))).map(|b| b as u8).collect();
        let mut byte_of: HashMap<String, u8> = HashMap::default();
        let width = |r: usize| real_first[r].iter().map(|w| w.count_ones()).sum::<u32>();
        for r in 0..rows.len() {
            let kind = &token_rows[r].0;
            let shares = |q: usize| token_rows[q].0 != *kind && (0..256).any(|b| starts(r, b) && starts(q, b));
            // A reader's kind, and a token's kind that shares a first byte with a wider one's (a
            // `0x` beside a number, a `.5` beside the operators): its own kind, which its start
            // gives the record, as the lean lexer's `T` (decision 0509). The widest keeps its bytes.
            let shared = if low.token_reads.contains_key(kind) {
                (0..rows.len()).any(shares)
            } else {
                (0..rows.len()).any(|q| shares(q) && !low.token_reads.contains_key(&token_rows[q].0) && (width(q), std::cmp::Reverse(q)) > (width(r), std::cmp::Reverse(r)))
            };
            if !shared {
                continue;
            }
            let f = match byte_of.get(kind) {
                Some(&f) => f,
                None => {
                    if free.is_empty() {
                        return Err(format!("`{kind}`: no byte left that no token begins with, for its reader's kind"));
                    }
                    let f = free.remove(0);
                    byte_of.insert(kind.clone(), f);
                    f
                }
            };
            moved.push((r, f));
        }
        // A part of a region read in parts that begins with a byte another of the region's
        // tokens of another kind begins with (a `$name` and the string's text): a kind of its own
        // (`ReadPart::kind`, `Retag::part`), as the lean lexer's string reader says what it read.
        // The region's closers keep their bytes' kinds.
        let mut readers_in_parts: Vec<&String> = low.token_part_names.keys().collect();
        readers_in_parts.sort();
        for reader in readers_in_parts {
            let (parts, content) = low.token_part_names[reader].clone();
            let of = |r: usize| low.inside_of.get(r).cloned().flatten().filter(|(k, _)| k == reader);
            let ins: Vec<usize> = (0..rows.len()).filter(|&r| of(r).is_some()).collect();
            // The content too, where it shares a first byte with a closer or a part (a string's text
            // and its `"`), in a token loop (decision 0509): the walk gives each token its kind.
            let mut names: Vec<&String> = parts.iter().filter(|p| **p != content).collect();
            if low.skip.is_some() {
                names.push(&content);
            }
            names.sort();
            names.dedup();
            for p in names {
                let these: Vec<usize> = ins.iter().copied().filter(|&r| token_rows[r].0 == *p && of(r).is_some_and(|(_, closes)| !closes)).collect();
                let meets = |a: usize, b: usize| (0..4).any(|i| real_first[a][i] & real_first[b][i] != 0);
                if !these.iter().any(|&r| ins.iter().any(|&q| token_rows[q].0 != *p && meets(r, q))) {
                    continue;
                }
                // One kind a part's name, whichever reader reads it.
                let f = match part_bytes.get(p) {
                    Some(&f) => f,
                    None if free.is_empty() => return Err(format!("`{p}`: no byte left that no token begins with, for its part's kind")),
                    None => free.remove(0),
                };
                part_bytes.insert(p.clone(), f);
                moved.extend(these.iter().map(|&r| (r, f)));
            }
            // A closer that begins with a byte a token outside the regions begins with (a
            // template's `"` and a string's opener), in a token loop: a kind of its own
            // (`Reader::closers`, decision 0509), as the lean lexer's string reader knows its end.
            if low.skip.is_some() {
                let outside = |q: usize| !low.inside.get(q).copied().unwrap_or(false);
                for r in ins.iter().copied().filter(|&r| of(r).is_some_and(|(_, closes)| closes)) {
                    if !(0..rows.len()).any(|q| outside(q) && (0..4).any(|i| real_first[r][i] & real_first[q][i] != 0)) {
                        continue;
                    }
                    let key = format!("{} closes", token_rows[r].0);
                    let f = match part_bytes.get(&key) {
                        Some(&f) => f,
                        None if free.is_empty() => return Err(format!("`{key}`: no byte left that no token begins with, for its closer's kind")),
                        None => free.remove(0),
                    };
                    part_bytes.insert(key, f);
                    moved.push((r, f));
                    closer_rows.push((reader.clone(), r, f));
                }
            }
        }
        for &(r, f) in &moved {
            rows[r].first = [0; 4];
            rows[r].first[usize::from(f) / 64] |= 1 << (f % 64);
            rows[r].moved = true;
        }
        // The rows of one kind end alike: at their records' last bytes (`Span::Last`), a one-byte
        // token's its first, so no state tells them apart by how they end (decision 0509).
        for &(r, f) in &moved {
            if moved.iter().any(|&(q, g)| g == f && !rows[q].one_byte) {
                rows[r].one_byte = false;
            }
        }
        // A moved kind's byte is its alone: tier 2 switches on a record's kind (its least
        // byte's target), so a row read inside a region, whose bytes may be any, never meets
        // a state there.
        for r in 0..rows.len() {
            if moved.iter().any(|&(m, _)| m == r) {
                continue;
            }
            for &(_, f) in &moved {
                rows[r].first[usize::from(f) / 64] &= !(1 << (f % 64));
            }
        }
    }
    let meets = row_modes(low, &token_rows, &rows, &states, &arms, &mark_kinds)?;
    // The totals are one flag (`BlockRun::totals`), tested once: `EMPTY 0`.
    let empties = 1;
    // The counts no value reads whose ups and downs are tokens' starts, each of tokens whose text
    // is one of a few words (each proved, `never`): tier 2 counts them at the arms that take the
    // tokens (`COUNT`), as the lean parser counts its depth as it parses. Tier 1 makes no planes.
    let mut counts: Vec<(Vec<u16>, Vec<u16>)> = Vec::new();
    let mut count_most: Vec<u16> = Vec::new();
    {
        let words: Vec<(usize, Vec<u8>)> = (0..rows.len())
            .filter(|&r| !low.extra.contains(&token_rows[r].0))
            .flat_map(|r| rows[r].keywords.iter().map(move |(w, _)| (r, w.clone())))
            .collect();
        let pending = std::mem::take(&mut low.pending_counts);
        for (name, up, down, max) in pending {
            let kinds_of = |x: Val, low: &mut Lowering| -> Result<Option<Vec<u16>>, String> {
                let (mut kinds, mut all): (Vec<u16>, Option<Val>) = (Vec::new(), None);
                for (r, w) in &words {
                    let k = low.keyword(token_rows[*r].1, token_rows[*r].2, w)?;
                    let both = low.and(x, k)?;
                    if low.never(both) {
                        continue;
                    }
                    kinds.push(id(&String::from_utf8_lossy(w))?);
                    all = Some(match all {
                        Some(a) => low.or(a, k)?,
                        None => k,
                    });
                }
                let Some(all) = all else { return Ok(None) };
                let differs = low.xor(x, all)?;
                Ok(low.never(differs).then_some(kinds))
            };
            let (u, d) = (kinds_of(up, low)?, kinds_of(down, low)?);
            match (u, d, u16::try_from(max)) {
                (Some(u), Some(d), Ok(m)) => {
                    counts.push((u, d));
                    count_most.push(m);
                }
                _ => low.pending_counts.push((name, up, down, max)),
            }
        }
    }
    // The checks `A & L` whose `A` lies in one row's tokens: `A` inside a `fill(s, e)` whose
    // runs are the row's tokens (its start `s`, its last the run's next byte), each proved.
    // Tier 2 checks `L` at `A`'s bits in each such token it takes (`CHECK`), as the lean lexer
    // checks a string's `\u` escapes; tier 1 makes `A` alone, a mask.
    // The token tests that are tier 2's: at their kind's rows.
    for (kind, mark, a, l) in std::mem::take(&mut low.kind_checks) {
        let mut kinds: Vec<u16> = (0..rows.len()).filter(|&r| token_rows[r].0 == kind).flat_map(|r| row_kinds(&rows[r])).collect();
        kinds.sort_unstable();
        kinds.dedup();
        let ops = low.local_ops(l).ok_or("a local check's ops")?;
        if kinds.is_empty() {
            return Err(format!("a token test of `{kind}`: no token of the kind"));
        }
        checks.push((kinds, mark.map(|k| mark_kinds[k]), a, crate::program::LocalCheck { mask: 0, delay: l.d, ops, token: false, back: 0 }));
    }
    // The tokens' checks: at their rows' tokens, over each token's bytes; no mask of tier 1's.
    for (r, ops) in token_checks {
        checks.push((row_kinds(&rows[r]), None, token_rows[r].1, crate::program::LocalCheck { mask: 0, delay: 0, ops, token: true, back: 0 }));
    }
    // Every row's tokens' last bytes: the records' (decision 0428).
    let mut any_last: Option<Val> = None;
    for r in 0..rows.len() {
        let x = token_rows[r].2;
        any_last = Some(match any_last {
            Some(y) => low.or(y, x)?,
            None => x,
        });
    }
    for (a, l, not) in std::mem::take(&mut low.pending_checks) {
        // `A & ~L` with `A` at a row's tokens' last bytes: a token's check (decision 0399),
        // reading the tokens before it where `L` reads the row's (decision 0428).
        if not && let Some(r) = last_row(low, a, &rows, token_rows)? {
            if let Some((ops, delay)) = low.token_ops_past(a, token_rows[r].2, l) {
                checks.push((row_kinds(&rows[r]), None, token_rows[r].1, crate::program::LocalCheck { mask: 0, delay, ops, token: true, back: 0 }));
                continue;
            }
            if let Some(any) = any_last
                && let Some((ops, delay, back)) = low.token_ops_back(a, token_rows[r].2, l, any)?
            {
                checks.push((row_kinds(&rows[r]), None, token_rows[r].1, crate::program::LocalCheck { mask: 0, delay, ops, token: true, back }));
                continue;
            }
        }
        let ops = if not { low.local_ops_in_stream(l) } else { low.local_ops(l) };
        match (token_row(low, a, &rows, token_rows)?, ops) {
            (Some(r), Some(mut ops)) => {
                if not {
                    ops.push(crate::program::LocalOp::Not(ops.len() as u16 - 1));
                }
                checks.push((row_kinds(&rows[r]), None, a, crate::program::LocalCheck { mask: 0, delay: l.d, ops, token: false, back: 0 }));
            }
            _ => {
                let x = match low.back_fallbacks.get(&(a.v, l.v)) {
                    Some(&x) if not => x,
                    _ if not => low.andnot(a, l)?,
                    _ => low.and(a, l)?,
                };
                low.fallbacks.push(x);
            }
        }
    }
    // A row's token's checks are one (decision 0399): one load of the token's bytes, its
    // classes and their masks made once, the tests ORed at the most delay.
    let mut merged: Vec<(Vec<u16>, Option<u16>, Val, crate::program::LocalCheck)> = Vec::new();
    for c in checks {
        match merged.iter_mut().find(|m| m.3.token && c.3.token && m.0 == c.0 && m.1 == c.1 && m.2.v == c.2.v && mergeable(&m.3, &c.3)) {
            Some(m) => m.3 = merge_token_checks(&m.3, &c.3),
            None => merged.push(c),
        }
    }
    let checks = merged;
    let check_kinds: Vec<(Vec<u16>, Option<u16>)> = checks.iter().map(|c| (c.0.clone(), c.1)).collect();
    let tested = bitlex_tier2::lower(&states, &arms, &rows, &marks, any_mark, &meets, empties, whole, true, by_rules, token_extras, &counts, &check_kinds)?.tested;
    for (j, &r) in tested.iter().enumerate() {
        rows[r].mask = Some(row_masks + j as u16);
        mark_masks.push(token_rows[r].1);
    }
    // The words, once the fallback's totals are known: each must be zero.
    let mut lowered = bitlex_tier2::lower(&states, &arms, &rows, &marks, any_mark, &meets, empties, whole, false, by_rules, token_extras, &counts, &check_kinds)?;
    crate::rir_text::count_rows(&mut lowered.text, &count_most);
    // The extras' kinds: the node table flags their nodes (comments).
    readings.extras = low.extra.iter().map(|k| id(k)).collect::<Result<_, _>>()?;
    crate::rir_text::reading_rows(&mut lowered.text, &readings);
    let takes_last: Vec<bool> = rows.iter().map(bitlex_tier2::Row::takes_last).collect();
    // The records: a byte's kind, the class of first bytes no switch on the first byte tells
    // apart; the words, the strings. (A switch on a later byte, a keyword's, reads its byte.)
    let mut kinds = vec![0u8; 256];
    let mut classes: HashMap<Vec<u16>, u8> = HashMap::default();
    let mut sigs: Vec<Vec<u16>> = Vec::new();
    // A token mark's kinds (decision 0129): a column each, so a kind of the records is one
    // kind's tokens or none of them. An element's kinds are one set: a byte that starts a kind
    // of the set and a kind outside it can not tell them apart, and the mark is not compiled
    // (`integer_literal | hex_literal | bin_literal` share `0`, and the set takes all three).
    let mut mark_kind_sets: Vec<Vec<String>> = lowered.token_marks.iter().flat_map(|(_, _, _, es)| es.iter().flat_map(|e| std::iter::once(e.1.clone()).chain(e.6.iter().map(|n| n.2.clone())).chain(e.7.iter().map(|x| x.0.clone())))).filter(|set: &Vec<String>| !set.is_empty()).collect();
    for set in &mut mark_kind_sets {
        set.sort();
        set.dedup();
    }
    let mut mark_kind_names: Vec<String> = mark_kind_sets.iter().flatten().cloned().collect();
    mark_kind_names.sort();
    mark_kind_names.dedup();
    // The moved rows' own first bytes again, for the token marks' kinds, the end rules and the marks below.
    for &(r, _) in &moved {
        rows[r].first = real_first[r];
    }
    let starts_kind = |name: &str, b: usize| (0..rows.len()).any(|r| token_rows[r].0 == name && rows[r].first[b / 64] >> (b % 64) & 1 == 1);
    // A kind's record kinds, for the token marks' and the gap patterns' sets: a moved row's own
    // (its records' kind, decision 0509), another row's first bytes'. A row inside a region is not
    // the token's.
    let inside_rows = low.inside.clone();
    let name_kinds = |kinds: &[u8], name: &str| -> Vec<u8> {
        let mut v: Vec<u8> = Vec::new();
        for r in (0..rows.len()).filter(|&r| token_rows[r].0 == name && !inside_rows.get(r).copied().unwrap_or(false)) {
            match moved.iter().find(|&&(m, _)| m == r) {
                Some(&(_, f)) => v.push(kinds[usize::from(f)]),
                None => v.extend((0..256).filter(|&b| rows[r].first[b / 64] >> (b % 64) & 1 == 1).map(|b| kinds[b])),
            }
        }
        v.sort_unstable();
        v.dedup();
        v
    };
    // A row read inside a region (`inside`, a string's content) is left out: its tokens are only
    // inside the region, whose rule takes no mark, so a mark that holds there is skipped (hidden)
    // or refuses the file, and never reads a token otherwise.
    let inside = |r: usize| low.inside.get(r).copied().unwrap_or(false);
    for set in &mark_kind_sets {
        for name in set {
            for b in 0..256 {
                if starts_kind(name, b) && (0..rows.len()).any(|r| !set.contains(&token_rows[r].0) && !inside(r) && !low.extra.contains(&token_rows[r].0) && rows[r].first[b / 64] >> (b % 64) & 1 == 1) {
                    return Err(format!("a token mark's kind `{name}` shares byte {b:#04x} with another token"));
                }
            }
        }
    }
    for b in 0..256 {
        let mut sig: Vec<u16> = lowered.switches.iter().filter(|s| s.offset == 0).map(|s| s.targets[b]).collect();
        // A moved row's tokens have their own kind (decision 0509): its first bytes are not its
        // column's, so a `0` is one class with `1`..`9`, as the lean lexer's one `Int`.
        sig.extend(mark_kind_names.iter().map(|n| u16::from((0..rows.len()).any(|r| token_rows[r].0 == *n && !moved.iter().any(|&(m, _)| m == r) && rows[r].first[b / 64] >> (b % 64) & 1 == 1))));
        let n = classes.len();
        let k = *classes.entry(sig.clone()).or_insert(n as u8);
        if usize::from(k) == sigs.len() {
            sigs.push(sig);
        }
        if classes.len() > 256 {
            return Err("more than 256 kinds of first byte".into());
        }
        kinds[b] = k;
    }
    // Numbered so a switch on the kind has few ranges of one target (its compare tree is
    // shallow, as a jump table on the lean lexer's kinds is small): from byte 0's class, each
    // next the class that the fewest switches tell apart from the one before (ties by least
    // byte).
    let differ = |a: &[u16], b: &[u16]| a.iter().zip(b).filter(|(x, y)| x != y).count();
    let mut order: Vec<usize> = vec![0];
    let mut left: Vec<usize> = (1..sigs.len()).collect();
    while !left.is_empty() {
        let last = *order.last().expect("one");
        let (i, _) = left.iter().enumerate().min_by_key(|&(_, &c)| (differ(&sigs[last], &sigs[c]), c)).expect("left");
        order.push(left.remove(i));
    }
    let mut renumber = vec![0u8; sigs.len()];
    for (new, &old) in order.iter().enumerate() {
        renumber[old] = new as u8;
    }
    for k in &mut kinds {
        *k = renumber[usize::from(*k)];
    }
    // The marks in the code, by order (the first `MARKS`): each its mask (an index into the
    // program's masks here, its output once they are placed) and its word's code.
    let mut in_code: Vec<(u16, u16)> = marks.iter().take(code_marks.min(crate::records::MARKS)).enumerate().map(|(k, m)| (m.mask, lowered.mark_words.iter().find(|&&(x, _)| usize::from(x) == k).map_or(0, |&(_, s)| s + 1))).collect();
    // A token mark of one first word: its mask is every grammar token's start, so where no
    // extra's token can begin with the word's first byte, its bit is the word's alone
    // (`records::WORD_ALONE`): no mask read a token.
    for (k, m) in marks.iter().take(in_code.len()).enumerate() {
        let (Some(w), true) = (&m.word, m.tokens.is_some()) else { continue };
        let extra_first = (0..rows.len()).any(|r| low.extra.contains(&token_rows[r].0) && rows[r].first[usize::from(w[0]) / 64] >> (w[0] % 64) & 1 == 1);
        if in_code[k].1 != 0 && !extra_first {
            in_code[k].0 = crate::records::WORD_ALONE;
        }
    }
    for (k, m) in in_code.iter_mut().enumerate() {
        if alone.get(k).copied().unwrap_or(false) {
            m.0 = crate::records::WORD_ALONE;
        }
    }
    let mut extra_mark = None;
    // The extras' kinds after the first, each with its bit (`Words::extras`).
    let mut extra_kind_bits: Vec<(String, u8)> = Vec::new();
    // The extra's bit, after the marks': its rows' starts (`TOKEN_EXTRAS`, `Words::extras`).
    if token_extras {
        let mut starts: Option<Val> = None;
        let extra_starts: Vec<Val> = token_rows.iter().filter(|(k, _, _)| low.extra.contains(k)).map(|r| r.1).collect();
        for start in &extra_starts {
            starts = Some(match starts {
                Some(s) => low.or(s, *start)?,
                None => *start,
            });
        }
        let starts = starts.ok_or("an extra with no rows")?;
        mark_masks.push(starts);
        in_code.push((mark_masks.len() as u16 - 1, 0));
        let bit = (in_code.len() - 1) as u8;
        extra_mark = Some(bit);
        let mut bits = vec![(bit, id(&low.extra[0])?)];
        // Each kind after the first: its rows' starts, a bit of its own (`Words::extras`).
        let later: Vec<String> = low.extra[1..].to_vec();
        for kind in &later {
            let mut starts: Option<Val> = None;
            let rows_of: Vec<Val> = token_rows.iter().filter(|(k, _, _)| k == kind).map(|r| r.1).collect();
            for r in rows_of {
                starts = Some(match starts {
                    Some(s) => low.or(s, r)?,
                    None => r,
                });
            }
            mark_masks.push(starts.ok_or("an extra with no rows")?);
            in_code.push((mark_masks.len() as u16 - 1, 0));
            let b = (in_code.len() - 1) as u8;
            extra_kind_bits.push((kind.clone(), b));
            bits.push((b, id(kind)?));
        }
        crate::rir_text::extra_rows(&mut lowered.text, &bits);
    }
    // The mark bits an extra's token has: the extra's, and its kind's own after the first.
    let extra_kinds: Vec<String> = low.extra.clone();
    let extra_marks = |kind: &String| -> u32 {
        if !extra_kinds.contains(kind) {
            return 0;
        }
        extra_mark.map_or(0, |e| 1u32 << e) | extra_kind_bits.iter().filter(|(k, _)| k == kind).fold(0, |m, &(_, b)| m | 1u32 << b)
    };
    // The token marks, their kinds the records' (`Words::token_marks`). The marks their
    // `without`s read (`Records::read`).
    let mut token_read = 0u32;
    if !lowered.token_marks.is_empty() {
        // Extras between a mark's tokens are placed in the token step only; a grammar with none
        // has nothing to place.
        if !token_extras && !low.extra.is_empty() {
            return Err("token marks with extras not taken in the token step".into());
        }
        let mut tms = Vec::new();
        for (order, refuses, unless, elems) in &lowered.token_marks {
            let mut out = Vec::new();
            // Kinds' names as the records' kind set, and their hard keywords: a record of the kind
            // with one of these words is the keyword's token (`TokenElem::except`). Not a row of
            // keywords only (`required`, kind 0): its kind names the row, every token of it a keyword.
            let kind_set = |names: &[String]| -> Result<(u64, Vec<u16>), String> {
                let (mut set, mut except) = (0u64, Vec::new());
                for name in names {
                    for r in (0..rows.len()).filter(|&r| &token_rows[r].0 == name && rows[r].kind != 0) {
                        for (w, _) in &rows[r].keywords {
                            let s = lowered.strings.iter().position(|x| x == w).ok_or("a keyword not among the words")?;
                            except.push(s as u16);
                        }
                    }
                    for k in name_kinds(&kinds, name) {
                        if k >= 64 {
                            return Err(format!("a token mark's kind `{name}` is kind {k} of the records, past 64"));
                        }
                        set |= 1 << k;
                    }
                }
                Ok((set, except))
            };
            for (offset, names, words, not, end, run, nest_rows, ending_row, without_names) in elems {
                let (set, except) = kind_set(names)?;
                // `without M`, `with M`: mark `M`'s bit in the record's code, so a mark of a mask
                // the code holds (not a token mark's, which tier 2 tests itself).
                let (mut without, mut with) = (0u32, 0u32);
                for (m, w) in without_names {
                    let k = low.marks.iter().position(|x| &x.0 == m).ok_or_else(|| format!("`with {m}`: no such mark"))?;
                    if !records || k >= code_marks.min(crate::records::MARKS) || low.token_marks.contains_key(&k) {
                        return Err(format!("`with {m}`: a mark of a mask whose bit the records hold"));
                    }
                    if *w {
                        with |= 1 << k;
                    } else {
                        without |= 1 << k;
                    }
                }
                token_read |= without | with;
                let mut nests = Vec::new();
                for (counter, open, ns, ws) in nest_rows {
                    let (kinds, except) = kind_set(ns)?;
                    let test = crate::program::TokenElem { offset: *offset, kinds, words: ws.clone(), except, ..Default::default() };
                    nests.push(crate::program::Nest { counter: *counter, open: *open, test });
                }
                let ending = match ending_row {
                    Some((ns, ws)) => {
                        let (kinds, except) = kind_set(ns)?;
                        Some(Box::new(crate::program::TokenElem { offset: *offset, kinds, words: ws.clone(), except, ..Default::default() }))
                    }
                    None => None,
                };
                out.push(crate::program::TokenElem { offset: *offset, kinds: set, words: words.clone(), except, not: *not, end: *end, run: *run, nests, ending, without, with, first: [0; 4], since: false });
            }
            tms.push(crate::program::TokenMark { order: *order, elems: out, refuses: *refuses, unless: *unless });
        }
        crate::rir_text::token_mark_rows(&mut lowered.text, &tms);
    }
    // The `drops` and `loses` questions (`Words::gap_patterns`, decision 0451): records as the
    // lexer wrote them. An element of an extra's kind is any record with its kind's bits (the
    // first kind's, the extra's alone: none of the other kinds'); its `with M`, the marks of the
    // extra's own gap (`Records::extra`).
    if !lowered.gap_patterns.is_empty() {
        let all_kinds: u32 = extra_kinds.iter().fold(0, |a, k| a | extra_marks(k));
        let mut gps = Vec::new();
        for (order, unless, elems) in &lowered.gap_patterns {
            let mut out = Vec::new();
            for (offset, names, words, written, marks_named, first, since) in elems {
                let not = written;
                let (extras, grammar): (Vec<&String>, Vec<&String>) = names.iter().partition(|n| extra_kinds.contains(n));
                let (mut with, mut without) = (0u32, 0u32);
                let (mut kind_set, mut except, mut not) = (0u64, Vec::new(), *not);
                match (extras.as_slice(), grammar.is_empty()) {
                    ([], _) => {
                        let mut set = 0u64;
                        for name in &grammar {
                            for r in (0..rows.len()).filter(|&r| &&token_rows[r].0 == name && rows[r].kind != 0) {
                                for (w, _) in &rows[r].keywords {
                                    except.push(lowered.strings.iter().position(|x| x == w).ok_or("a keyword not among the words")? as u16);
                                }
                            }
                            for k in name_kinds(&kinds, name) {
                                if k >= 64 {
                                    return Err(format!("a gap pattern's kind `{name}` is kind {k} of the records, past 64"));
                                }
                                set |= 1 << k;
                            }
                        }
                        kind_set = set;
                        // No kinds or words: any record (a `~` of none).
                        if grammar.is_empty() && words.is_empty() {
                            not = !not;
                        }
                    }
                    (chosen, true) if words.is_empty() && !not => {
                        // The bits every kind chosen holds, and none of those the others hold
                        // alone (the first kind's code holds the extra's bit alone): exactly the
                        // kinds chosen, else refused.
                        let common = chosen.iter().fold(all_kinds, |a, k| a & extra_marks(k));
                        let any = chosen.iter().fold(0, |a, k| a | extra_marks(k));
                        with |= common;
                        without |= all_kinds & !any;
                        if extra_kinds.iter().any(|k| (extra_marks(k) & common == common && extra_marks(k) & all_kinds & !any == 0) != chosen.contains(&k)) {
                            return Err(format!("a gap pattern's element of extra kinds {chosen:?} that their bits do not tell apart"));
                        }
                        not = true;
                    }
                    _ => return Err("a gap pattern's element of extras' kinds alone, or of grammar kinds and words".into()),
                }
                for (m, w) in marks_named {
                    let k = low.marks.iter().position(|x| &x.0 == m).ok_or_else(|| format!("`with {m}`: no such mark"))?;
                    if !records || k >= code_marks.min(crate::records::MARKS) || low.token_marks.contains_key(&k) {
                        return Err(format!("`with {m}`: a mark of a mask whose bit the records hold"));
                    }
                    if *w {
                        with |= 1 << k;
                    } else {
                        without |= 1 << k;
                    }
                }
                token_read |= (without | with) & !all_kinds;
                let mut set = [0u64; 4];
                for &b in first {
                    set[usize::from(b / 64)] |= 1 << (b % 64);
                }
                // A `~` the grammar wrote, before the first record, holds there.
                let end = *written && (*offset < 0 || *since && *offset == 0);
                out.push(crate::program::TokenElem { offset: *offset, kinds: kind_set, words: words.clone(), except, not, end, without, with, first: set, since: *since, ..Default::default() });
            }
            gps.push(crate::program::TokenMark { order: *order, elems: out, refuses: false, unless: *unless });
        }
        crate::rir_text::gap_pattern_rows(&mut lowered.text, &gps);
    }
    // The checks' masks, after every other: each its `A`.
    let mut local_checks = Vec::new();
    for (_, _, a, mut c) in checks {
        if !c.token {
            mark_masks.push(a);
            c.mask = mark_masks.len() as u16 - 1;
        }
        local_checks.push(c);
    }
    crate::rir_text::local_check_rows(&mut lowered.text, &local_checks);
    // The end rules (decision 0177): a byte whose kinds share one rule ends its tokens by it; any
    // other, by the first bit at or after its start of its kinds' last-byte masks. Each rule's
    // mask a mask output, as a mark's (its index among them, made an output's below).
    let mut rules: Vec<crate::records::EndRule> = Vec::new();
    // Each rule's token tests (`Records::checks`), by their index in `low.token_escapes`: a rule
    // with tests is its kind's alone.
    let mut rule_tests: Vec<Vec<usize>> = Vec::new();
    let mut readers: Vec<crate::records::Reader> = Vec::new();
    let mut reader_starts: Vec<(usize, Val)> = Vec::new();
    let mut nexts: Vec<crate::records::PairTable> = Vec::new();
    // Each table's pairs, to find it again.
    let mut tabled: Vec<Vec<Vec<u8>>> = Vec::new();
    let mut ends = vec![crate::records::NO_RULE; 256];
    if !low.token_escapes.is_empty() && !(looped && (!low.token_rules.is_empty() || !low.token_reads.is_empty())) {
        return Err("token tests (`escape … at KIND`) need the token loop".into());
    }
    if !low.token_rules.is_empty() || !low.token_reads.is_empty() {
        let mask_of = |low: &mut Lowering, v: Val, mark_masks: &mut Vec<Val>| -> u16 {
            match mark_masks.iter().position(|m: &Val| m.v == v.v && m.d == v.d) {
                Some(k) => k as u16,
                None => {
                    mark_masks.push(v);
                    let _ = low;
                    (mark_masks.len() - 1) as u16
                }
            }
        };
        // A byte whose kinds have pairs (`pair P`): the kind's rule, its pairs' table
        // (`EndRule::Next`), one for all its first bytes, as a lexer's table of tokens.
        let mut next_of = |low: &Lowering, rows: &[usize], _b: usize, nexts: &mut Vec<crate::records::PairTable>| -> Option<crate::records::EndRule> {
            let pairs = low.token_pairs.get(&token_rows[*rows.first()?].0)?;
            if rows.iter().any(|&r| low.token_pairs.get(&token_rows[r].0) != Some(pairs)) {
                return None;
            }
            let at = match tabled.iter().position(|x| x == pairs) {
                Some(at) => at,
                None => {
                    let thirds = pairs.iter().filter(|p| p.len() == 3).map(|p| [p[0], p[1], p[2]]).collect();
                    let mut t = crate::records::PairTable { row: vec![0; 256], sets: vec![[0; 4]], thirds };
                    for first in 0..256usize {
                        let mut set = [0u64; 4];
                        for p in pairs.iter().filter(|p| p.len() == 2 && usize::from(p[0]) == first) {
                            set[usize::from(p[1]) / 64] |= 1 << (p[1] % 64);
                        }
                        if set != [0; 4] {
                            let k = t.sets.iter().position(|x| *x == set).unwrap_or_else(|| {
                                t.sets.push(set);
                                t.sets.len() - 1
                            });
                            t.row[first] = k as u8;
                        }
                    }
                    nexts.push(t);
                    tabled.push(pairs.clone());
                    nexts.len() - 1
                }
            };
            Some(crate::records::EndRule::Next(at as u16))
        };
        for b in 0..256usize {
            // A row read inside a region (`inside`) its reader starts: no byte's rule.
            let these: Vec<usize> = (0..rows.len()).filter(|&r| !low.inside.get(r).copied().unwrap_or(false) && rows[r].first[b / 64] >> (b % 64) & 1 == 1).collect();
            if these.is_empty() {
                continue;
            }
            // A region's token: its reader, the other kinds that start here its other rule.
            let reads: Vec<usize> = these.iter().copied().filter(|&r| low.token_reads.contains_key(&token_rows[r].0)).collect();
            if let Some(&r0) = reads.first() {
                let kind = token_rows[r0].0.clone();
                // The other kinds read here (Kotlin's block comment beside its line comment):
                // their regions are arms of this reader, each with its kind's tag.
                let mut also: Vec<String> = reads.iter().map(|&r| token_rows[r].0.clone()).filter(|k| *k != kind).collect();
                also.sort();
                also.dedup();
                let rest: Vec<usize> = these.iter().copied().filter(|r| !reads.contains(r)).collect();
                let other = match rest.first() {
                    None => crate::records::NO_RULE,
                    Some(&x) => {
                        let ruled: Vec<Option<(bool, Val)>> = rest.iter().map(|&r| low.token_rules.get(&token_rows[r].0).copied()).collect();
                        let Some((pair, v)) = ruled[0].filter(|&(p, v)| ruled.iter().all(|y| y.is_some_and(|(q, w)| q == p && w.v == v.v && w.d == v.d))) else {
                            return Err(format!("`{}` starts at a reader's byte {b:#x} with no rule (`run`, `pair`)", token_rows[x].0));
                        };
                        let rule = match next_of(low, &rest, b, &mut nexts) {
                            Some(rule) => rule,
                            None => {
                                let k = mask_of(low, v, &mut mark_masks);
                                if pair { crate::records::EndRule::Pair(k) } else { crate::records::EndRule::Run(k) }
                            }
                        };
                        let at = match rules.iter().zip(&rule_tests).position(|(&y, t)| y == rule && t.is_empty()) {
                            Some(at) => at,
                            None => {
                                rules.push(rule);
                                rule_tests.push(vec![]);
                                rules.len() - 1
                            }
                        };
                        u8::try_from(at).map_err(|_| "more than 256 end rules")?
                    }
                };
                // Its kind's own, where its rows were moved (above).
                // A token's own rows: a row inside a region of the same name (a closer `"`) is not its.
                let inside_rows = low.inside.clone();
                let retag_of = |k: &String| moved.iter().find(|&&(r, _)| token_rows[r].0 == *k && !inside_rows.get(r).copied().unwrap_or(false)).map(|&(_, f)| (kinds[usize::from(f)], extra_marks(k)));
                let retag = retag_of(&kind);
                // An output of its own: one shared with a mark's may be the token loop's alone
                // (`loop_marks`), not stored.
                let last = low.token_closer_last.get(&kind).copied().map(|l| {
                    mark_masks.push(l);
                    (mark_masks.len() - 1) as u16
                });
                let mut arms: Vec<crate::records::ReadArm> = low.token_reads[&kind].clone().into_iter().map(|(o, c, off, none)| crate::records::ReadArm { opener: mask_of(low, o, &mut mark_masks), closer: mask_of(low, c, &mut mark_masks), off, last, none, retag: None }).collect();
                for k in &also {
                    // A kind's own tag tells its tokens from the reader's: it needs one, as does
                    // the reader's kind, and neither is read in parts.
                    let Some(tag) = retag_of(k).filter(|_| retag.is_some()) else {
                        return Err(format!("two kinds read by readers start at byte {b:#x}, not told apart by their tags"));
                    };
                    if low.token_parts.contains_key(k) || low.token_parts.contains_key(&kind) {
                        return Err(format!("two kinds read by readers start at byte {b:#x}, one in parts"));
                    }
                    for (o, c, off, none) in low.token_reads[k].clone() {
                        arms.push(crate::records::ReadArm { opener: mask_of(low, o, &mut mark_masks), closer: mask_of(low, c, &mut mark_masks), off, last: None, none, retag: Some(tag) });
                    }
                }
                // A region read in parts: each part's start, a mask output.
                let part_names = low.token_part_names.get(&kind).map(|p| p.0.clone()).unwrap_or_default();
                let parts = match low.token_parts.get(&kind).cloned() {
                    Some(ps) => ps
                        .into_iter()
                        .enumerate()
                        .map(|(i, (v, bytes, last))| crate::records::ReadPart { start: mask_of(low, v, &mut mark_masks), bytes, last: last.map(|l| mask_of(low, l, &mut mark_masks)), kind: part_names.get(i).and_then(|n| part_bytes.get(n)).map(|&f| kinds[usize::from(f)]) })
                        .collect(),
                    None => Vec::new(),
                };
                // Its parts are tier 2's (decision 0323's lowering B): tier 1 reads the region as
                // one token, as the lean lexer reads a string, and tier 2 splits it where it reads
                // it, as the lean parser's string arm and simdjson's stage 2 read a string.
                let split = if parts.is_empty() { crate::records::Split::Tier1 } else { crate::records::Split::Tier2 };
                // Its content's own kind, where its rows were moved (above).
                let content = low.token_part_names.get(&kind).and_then(|p| moved.iter().find(|&&(r, _)| token_rows[r].0 == p.1 && low.inside_of.get(r).cloned().flatten().is_some_and(|(k, _)| k == kind))).map(|&(_, f)| kinds[usize::from(f)]);
                let mut closers: Vec<(u8, u8)> = closer_rows.iter().filter(|c| c.0 == kind).flat_map(|&(_, r, f)| { let first = real_first[r]; (0..=255u8).filter(move |&b| first[usize::from(b >> 6)] >> (b & 63) & 1 == 1).map(move |b| (b, f)) }).map(|(b, f)| (b, kinds[usize::from(f)])).collect();
                closers.sort_unstable();
                closers.dedup();
                let reader = crate::records::Reader { arms, other, retag, parts, content, closers, split, quiet: false };
                let at = match readers.iter().position(|x| *x == reader) {
                    Some(at) => at,
                    None => {
                        readers.push(reader);
                        // A reader in parts: its token's start mask, from which its region is
                        // proved quiet below (`Reader::quiet`).
                        if let Some(&fill) = low.token_fills.get(&kind) {
                            reader_starts.push((readers.len() - 1, fill));
                        }
                        readers.len() - 1
                    }
                };
                let rule = crate::records::EndRule::Read(at as u16);
                let tests: Vec<usize> = (0..low.token_escapes.len()).filter(|&i| low.token_escapes[i].0 == kind || also.contains(&low.token_escapes[i].0)).collect();
                let at = match rules.iter().zip(&rule_tests).position(|(&x, t)| x == rule && *t == tests) {
                    Some(at) => at,
                    None => {
                        rules.push(rule);
                        rule_tests.push(tests);
                        rules.len() - 1
                    }
                };
                ends[b] = u8::try_from(at).map_err(|_| "more than 256 end rules")?;
                continue;
            }
            let ruled: Vec<Option<(bool, Val)>> = these.iter().map(|&r| low.token_rules.get(&token_rows[r].0).copied()).collect();
            let rule = match ruled[0] {
                Some((pair, v)) if ruled.iter().all(|x| x.is_some_and(|(p, w)| p == pair && w.v == v.v && w.d == v.d)) => match next_of(low, &these, b, &mut nexts) {
                    Some(rule) => rule,
                    None => {
                        let k = mask_of(low, v, &mut mark_masks);
                        if pair { crate::records::EndRule::Pair(k) } else { crate::records::EndRule::Run(k) }
                    }
                },
                _ => {
                    let mut l: Option<Val> = None;
                    for &r in &these {
                        let x = token_rows[r].2;
                        l = Some(match l {
                            Some(y) => low.or(y, x)?,
                            None => x,
                        });
                    }
                    crate::records::EndRule::Last(mask_of(low, l.expect("a row"), &mut mark_masks))
                }
            };
            // The tests of the kinds starting here: an escape only where no other kind does. A
            // mark's test (`at KIND when`) is a test of every token that starts here: its mask
            // must hold in none of another kind's (decision 0490: a number's `real`, whose bytes
            // a hex number has not), as the ladder's check of the reference finds.
            let tests: Vec<usize> = (0..low.token_escapes.len()).filter(|&i| these.iter().any(|&r| token_rows[r].0 == low.token_escapes[i].0)).collect();
            let escapes = tests.iter().any(|&i| low.token_escapes[i].4 == crate::records::CheckThen::Escape);
            if escapes && these.iter().any(|&r| tests.iter().any(|&i| token_rows[r].0 != low.token_escapes[i].0)) {
                return Err(format!("a token test of `{}`: another kind's tokens start at byte {b:#x} too", low.token_escapes[tests[0]].0));
            }
            let at = match rules.iter().zip(&rule_tests).position(|(&x, t)| x == rule && *t == tests) {
                Some(at) => at,
                None => {
                    rules.push(rule);
                    rule_tests.push(tests);
                    rules.len() - 1
                }
            };
            ends[b] = u8::try_from(at).map_err(|_| "more than 256 end rules")?;
        }
    }
    // The bytes between tokens, the loop's (stage 2a): with end rules, every token's.
    let skip = match low.skip {
        Some(v) if !rules.is_empty() => Some(match mark_masks.iter().position(|m: &Val| m.v == v.v && m.d == v.d) {
            Some(k) => k as u16,
            None => {
                mark_masks.push(v);
                (mark_masks.len() - 1) as u16
            }
        }),
        _ => None,
    };
    // Each `unless after` word's code: its index among the words, plus one (tier 2 names them).
    let mut unless = Vec::new();
    for (k, m) in marks.iter().enumerate().take(crate::records::MARKS) {
        for w in &m.unless {
            let at = lowered.strings.iter().position(|s| s == w).ok_or("an `unless after` word not among the words")?;
            unless.push((k as u8, at as u16 + 1));
        }
    }
    let mask_of_val = |v: Val, mark_masks: &mut Vec<Val>| -> u16 {
        match mark_masks.iter().position(|m: &Val| m.v == v.v && m.d == v.d) {
            Some(at) => at as u16,
            None => {
                mark_masks.push(v);
                (mark_masks.len() - 1) as u16
            }
        }
    };
    // The token tests, each its mask a mask output (made an output's below), as a rule's.
    let mut checks = Vec::new();
    for (k, tests) in rule_tests.iter().enumerate() {
        for &i in tests {
            let (_, count, v, least, then) = low.token_escapes[i].clone();
            let o = match mark_masks.iter().position(|m: &Val| m.v == v.v && m.d == v.d) {
                Some(at) => at as u16,
                None => {
                    mark_masks.push(v);
                    (mark_masks.len() - 1) as u16
                }
            };
            checks.push(crate::records::TokenCheck { rule: k as u8, count, o, least, then });
        }
    }
    // The readers' kinds by their rows' start masks, for the passes that read masks.
    let mut retags = Vec::new();
    for &(r, f) in &moved {
        let o = mask_of_val(token_rows[r].1, &mut mark_masks);
        let marks = extra_marks(&token_rows[r].0);
        let part = low.inside.get(r).copied().unwrap_or(false);
        retags.push(crate::records::Retag { o, kind: kinds[usize::from(f)], marks, part, first: rows[r].first });
    }
    // A region read in parts whose bytes past its opener, through its closer, hold no mask mark
    // and no retag (proved, with the regions' fixed point): its walk's codes need no marks
    // (`Reader::quiet`), as a lexer's string arm marks nothing inside the string.
    for &(k, fill) in &reader_starts {
        let rd = &mut readers[k];
        if rd.parts.is_empty() {
            continue;
        }
        // The region's fill past its opener's byte, and its closer's first byte.
        let opens = low.prev(fill, 1)?;
        let body = low.and(fill, opens)?;
        let ends = low.andnot(opens, fill)?;
        let within = low.or(body, ends)?;
        let mut quiet = true;
        // A part's own kind (`Retag::part`) is the walk's: not a mark it would need.
        let held: Vec<u16> = in_code.iter().map(|&(m, _)| m).filter(|&m| m != crate::records::WORD_ALONE).chain(retags.iter().filter(|t| !t.part).map(|t| t.o)).collect();
        for m in held {
            let both = low.and(mark_masks[usize::from(m)], within)?;
            if !low.settled_never(&mut settled, both) {
                quiet = false;
                break;
            }
        }
        rd.quiet = quiet;
    }
    let gap = match gap_ends(low, token_rows)? {
        Some(v) => Some(mask_of_val(v, &mut mark_masks)),
        None => None,
    };
    // The gap marks the CPU's token loop reads from the blanks (decision 0430), as the lean
    // lexer's `skip` sets `nl` while it passes them: a mark whose mask `M` holds at the bytes of
    // one class `C` alone (its first bytes), at no byte of a token, and at every byte of `C`
    // outside the regions (a region is its token's bytes, its reader walks it). Its bit at a
    // grammar token is then `C` in the blank runs before it and before the extras since the
    // grammar token before. A token's bytes, by its end rule, as the lean lexer's readers: a
    // run's, its first and its run mask's; a pair's, its first and the next; a reader's, its
    // regions' fills; a token inside a region, the region's; else (`last`) from its first to its
    // last, which the prover seldom shows. As `gap_ends`, the claims need hold only up to a
    // refusal.
    let mut blanks: Vec<(u8, u16)> = Vec::new();
    if skip.is_some() && !token_rows.is_empty() {
        let mut regions: Option<Val> = None;
        let mut fills: Vec<(String, Val)> = low.token_fills.iter().chain(&low.read_fills).map(|(k, v)| (k.clone(), *v)).collect();
        fills.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, f) in fills {
            regions = Some(match regions {
                Some(r) => low.or(r, f)?,
                None => f,
            });
        }
        // Each token's bytes, as a few values whose union holds them; each with whether the
        // mark's class must miss it (a lexer's token: no byte of the class) or the mark itself
        // (a region's, where the class's bytes are no mark's).
        let mut bytes_of: Vec<(Val, bool)> = Vec::new();
        for (i, t) in token_rows.iter().enumerate() {
            if let Some(&f) = low.read_fills.get(&t.0).or(low.token_fills.get(&t.0)) {
                bytes_of.push((f, false));
            } else if low.inside.get(i).copied().unwrap_or(false) {
                // Inside a region: its first and last bytes, and the region's between.
                bytes_of.push((regions.ok_or("a token inside a region with no region")?, false));
                bytes_of.push((t.1, true));
                bytes_of.push((t.2, true));
            } else if let Some(&(pair, v)) = low.token_rules.get(&t.0) {
                bytes_of.push((t.1, true));
                if pair {
                    let second = low.and(t.1, v)?;
                    bytes_of.push((low.prev(second, 1)?, true));
                    // A triple's third byte.
                    if let Some(&three) = low.token_threes.get(&t.0) {
                        let third = low.and(t.1, three)?;
                        bytes_of.push((low.prev(third, 2)?, true));
                    }
                } else {
                    bytes_of.push((v, true));
                }
            } else if low.token_pairs.contains_key(&t.0) {
                bytes_of.push((t.1, true));
                bytes_of.push((low.prev(t.1, 1)?, true));
            } else {
                // A last a fixed distance from the first (none: a token of one byte): the bytes
                // between.
                let mut fixed = (t.1.v == t.2.v && t.1.d == t.2.d).then_some(0);
                for k in (1..8u64).filter(|_| fixed.is_none()) {
                    let at = low.prev(t.1, k)?;
                    let differ = low.xor(at, t.2)?;
                    if bitlex_prove::never(&low.proof_ops(), &low.program.tables, differ.v, differ.d) {
                        fixed = Some(k);
                        break;
                    }
                }
                match fixed {
                    Some(k) => {
                        for j in 0..=k {
                            bytes_of.push((low.prev(t.1, j)?, true));
                        }
                    }
                    None => {
                        let past = low.prev(t.2, 1)?;
                        bytes_of.push((low.fill(t.1, past)?, true));
                    }
                }
            }
        }
        let mut refused = low.leaf(BlockOp::Const(0), false);
        for fb in low.fallbacks.clone() {
            refused = low.or(refused, fb)?;
        }
        let refused_next = low.next(refused, 1)?;
        let none = low.leaf(BlockOp::Const(0), false);
        let refused = low.fill(refused_next, none)?;
        let tested: u32 = checks.iter().fold(0, |a, c| match c.then {
            crate::records::CheckThen::At(k) | crate::records::CheckThen::After(k) => a | 1 << k,
            crate::records::CheckThen::Escape => a,
        });
        let unplaced: u32 = low.unplaced.iter().fold(0, |a, &k| a | 1 << k);
        let tested_only: u32 = low.tested_only.iter().fold(0, |a, &k| a | 1 << k);
        for k in 0..in_code.len() {
            let bit = 1u32 << k;
            if gaps & bit == 0 || in_code[k].0 == crate::records::WORD_ALONE || (tested | unplaced | tested_only) & bit != 0 || extra_mark == Some(k as u8) {
                continue;
            }
            let m = mark_masks[usize::from(in_code[k].0)];
            let class_bytes = low.settled_first_bytes(&mut settled, m);
            if class_bytes == [0; 4] || class_bytes == [u64::MAX; 4] {
                continue;
            }
            let class = low.class_set(class_bytes);
            // Each claim: the class misses the bytes, or else the mark does.
            let mut held = true;
            for &(b, lexed) in &bytes_of {
                let by_class = low.and(class, b)?;
                let by_class = low.andnot(by_class, refused)?;
                if lexed && low.either_never(&mut settled, by_class) {
                    continue;
                }
                let by_mark = low.and(m, b)?;
                let by_mark = low.andnot(by_mark, refused)?;
                if !low.either_never(&mut settled, by_mark) {
                    held = false;
                    break;
                }
            }
            let x = low.andnot(class, m)?;
            let x = match regions {
                Some(r) => low.andnot(x, r)?,
                None => x,
            };
            let x = low.andnot(x, refused)?;
            held = held && low.either_never(&mut settled, x);
            if held {
                blanks.push((k as u8, mask_of_val(class, &mut mark_masks)));
            }
        }
    }
    // The marks an extra keeps of its own gap (`Records::own`): the blank marks, which tier 2's
    // gap patterns read, and the settling marks, which `SETTLE` reads at the extras pending.
    let settling = low.settles.iter().filter(|&&k| gaps >> k & 1 == 1 && low.tested_only.iter().all(|&t| t != k)).fold(0u32, |a, &k| a | 1 << k);
    let own = if extra_mark.is_some() { blanks.iter().fold(settling, |a, &(k, _)| a | 1 << k) } else { 0 };
    let token_read = token_read | settling;
    let records = crate::records::Records { own, starts: 0, lasts: 1, kinds, words: lowered.strings.clone(), marks: in_code, ends, rules, skip, gaps, extra: extra_mark, untaken: Vec::new(), unless, unplaced: low.unplaced.iter().fold(0, |a, &k| a | 1 << k), checks, readers, retags, nexts, tested_only: low.tested_only.iter().fold(0, |a, &k| a | 1 << k), alone: token_alone & ((1u32 << crate::records::MARKS) - 1), read: token_read, blanks, check_masks: local_checks.iter().filter(|c| !c.token).map(|c| c.mask).collect(), prefix: low.prefix.as_ref().map(|p| p.0.clone()).unwrap_or_default(), gap };
    crate::records::WordTable::new(&records.words)?;
    // A mark's tests pass over an extra's token by its mark (`records::gap_marks`): the token
    // loop knows it only where the reader gives it (`Reader::retag`).
    if let Some(e) = records.extra {
        let tested = records.checks.iter().any(|c| c.then != crate::records::CheckThen::Escape);
        if tested && !records.readers.iter().any(|rd| rd.tags().any(|(_, m)| m >> e & 1 == 1)) {
            return Err("a mark's token tests need the extra's mark from its reader".into());
        }
    }
    // The names (`Words::names`): each rule's, its level's kind's; each node kind's; each record
    // kind's, the first token row's whose first byte has it.
    let named = named.into_inner();
    let mut names = crate::program::Names::default();
    for (i, &r) in lowered.rule_ids.iter().enumerate() {
        if let Some(n) = states.iter().find(|s| s.rule == r).and_then(|s| named.get(&s.level)) {
            names.rules.push((i as u16, n.clone()));
        }
    }
    names.kinds = named.iter().map(|(&k, n)| (k, n.clone())).collect();
    let mut token_named = [false; 256];
    for b in 0..256usize {
        let k = records.kinds[b];
        if token_named[usize::from(k)] {
            continue;
        }
        if let Some(r) = (0..rows.len()).find(|&r| !low.inside.get(r).copied().unwrap_or(false) && rows[r].first[b / 64] >> (b % 64) & 1 == 1) {
            token_named[usize::from(k)] = true;
            names.tokens.push((k, token_rows[r].0.clone()));
        }
    }
    // A reader's or a part's own kind (`records::Retag`): its row's name.
    let mut closer_kinds = Vec::new();
    for &(r, f) in &moved {
        let k = records.kinds[usize::from(f)];
        if !token_named[usize::from(k)] {
            token_named[usize::from(k)] = true;
            names.tokens.push((k, token_rows[r].0.clone()));
            if closer_rows.iter().any(|c| c.1 == r) {
                closer_kinds.push(k);
            }
        }
    }
    // A closer's whose name is another kind's too: the name and `_close` (a template's `"`,
    // `QUOTE_CLOSE`, apart from a string's opener `QUOTE`).
    let named_tokens = names.tokens.clone();
    for (k, n) in &mut names.tokens {
        if closer_kinds.contains(k) && named_tokens.iter().any(|(j, m)| j != k && m == n) {
            *n = format!("{n}_close");
        }
    }
    names.tokens.sort();
    // Each mark in the code by its bit: a grammar mark's name, or an extra's starts.
    for (j, _) in records.marks.iter().enumerate() {
        let n = if Some(j as u8) == records.extra {
            low.extra.first().map(|k| format!("{k}_start"))
        } else if let Some((k, _)) = extra_kind_bits.iter().find(|x| usize::from(x.1) == j) {
            Some(format!("{k}_start"))
        } else {
            marks.get(j).and_then(|m| low.marks.get(usize::from(m.mask))).map(|m| m.0.clone())
        };
        if let Some(n) = n {
            names.marks.push((j as u8, n));
        }
    }
    // The tables tier 2 as Rust reads (`bitlex_rust`), written once the records' outputs are the program's.
    let mark_bits = (0..low.marks.len())
        .map(|i| {
            let bit = (0..records.marks.len()).find(|&j| marks.get(j).is_some_and(|m: &bitlex_tier2::Mark| usize::from(m.mask) == i && m.tokens.is_none()));
            (bit.map(|j| j as u8), bit.map_or(0, |j| records.marks[j].1))
        })
        .collect();
    let extra_bits = extra_mark.map(|e| std::iter::once((e, low.extra[0].clone())).chain(extra_kind_bits.iter().map(|(k, b)| (*b, k.clone()))).collect()).unwrap_or_default();
    low.rust_tables = Some(RustTables { tokens: names.tokens.clone(), mark_bits, extra_bits, empties });
    let mut text = lowered.text;
    crate::rir_text::name_rows(&mut text, &names);
    Ok((grammar::text(&states, &arms), text, takes_last, mark_masks, records))
}

/// The rows each state of tier 2 can meet: `meets[s][r]`, whether a token of row `r` can be
/// the next one in state `s`.
///
/// A token's row is told by its first byte, and two rows that begin with one byte (`"` opening
/// a string in one and inside a string in another, say) by their start masks (`ROW`, a word per
/// token). Most such rows are apart in another way: they are read in different modes. A mode is
/// a `toggle(x)`'s, the parity of `x`'s bits before a token, and is known in each state:
///   - each row's start fixes the toggle or leaves it free (`never(start & ~t)`: it is set
///     there), and `x` at the start likewise, so its mode, when both are fixed;
///   - when every bit of `x` is some token's start (`never(x & ~starts)`), only a token that
///     starts on `x` changes the mode, and it flips it;
///   - the modes the states are entered in follow, from the root's start in mode 0, by the
///     arms (a mark takes no bytes), the calls and the returns (the mode is the stream's, not
///     the stack's: a return lands in every state the level is called to return to).
/// A row a state can meet in none of its modes is no candidate at a byte it shares.
fn row_modes(
    low: &mut Lowering,
    token_rows: &[(String, Val, Val)],
    rows: &[bitlex_tier2::Row],
    states: &[grammar::StateRow],
    arms: &[grammar::ArmRow],
    marks: &[u16],
) -> Result<Vec<Vec<bool>>, String> {
    // With the fixed points' equations (a region's carry is its fills' bytes after the openers)
    // where the plain search does not prove it, as the marks' proofs.
    let mut settled = low.induction(8, 100_000);
    // A stream a solve found refutes most questions first (`Induction::refutes`): most fail.
    let mut proved = |low: &mut Lowering, v: Val| low.either_never(&mut settled, v);
    // The toggles whose `x` bits are all token starts, at most eight.
    let mut starts: Option<Val> = None;
    for (_, s, _) in token_rows {
        starts = Some(match starts {
            Some(a) => low.or(a, *s)?,
            None => *s,
        });
    }
    let mut modes: Vec<(Val, Val)> = Vec::new();
    for (t, x) in low.toggles.clone() {
        let Some(s) = starts else { break };
        let stray = low.andnot(x, s)?;
        if modes.len() < 8 && proved(low, stray) {
            modes.push((t, x));
        }
    }
    // Each row's mode at its start, and its flips: per toggle, `Some(bit)` when fixed. Both proved, the row's
    // start never holds (`s & ~v` and `s & v`): the row is dead, and meets no state.
    let mut fixed = |low: &mut Lowering, s: Val, v: Val, dead: &mut bool| -> Result<Option<bool>, String> {
        let off = low.andnot(s, v)?;
        let on = low.and(s, v)?;
        Ok(match (proved(low, off), proved(low, on)) {
            (true, false) => Some(true),
            (false, true) => Some(false),
            (true, true) => {
                *dead = true;
                None
            }
            _ => None,
        })
    };
    let mut need: Vec<Vec<Option<bool>>> = Vec::new();
    let mut flip: Vec<Vec<Option<bool>>> = Vec::new();
    let mut dead: Vec<bool> = vec![false; token_rows.len()];
    for (r, (_, s, _)) in token_rows.iter().enumerate() {
        let (mut n, mut f) = (Vec::new(), Vec::new());
        for &(t, x) in &modes {
            // The mode before the token: the toggle at its start, less `x` there (the toggle's
            // asked only where `x`'s is fixed: else the mode is free). A row whose start never
            // holds is dead (`fixed`): `x` asked first finds it.
            let xv = fixed(low, *s, x, &mut dead[r])?;
            n.push(match xv {
                Some(b) => fixed(low, *s, t, &mut dead[r])?.map(|a| a != b),
                None => None,
            });
            f.push(xv);
        }
        need.push(n);
        flip.push(f);
    }
    // A region read in parts is a mode too, the reader's (`TokenRule::Inside`): its opener's
    // row sets it, a row that closes the region clears it, a row inside needs it set, and every
    // other row needs it clear. Its tokens come only between the opener and a closer.
    let mut readers: Vec<String> = low.inside_of.iter().flatten().map(|(k, _)| k.clone()).collect();
    readers.sort();
    readers.dedup();
    for k in readers {
        if need.first().map_or(0, Vec::len) >= 8 {
            break;
        }
        for r in 0..token_rows.len() {
            let (n, f) = match low.inside_of.get(r).cloned().flatten() {
                Some((q, closes)) if q == k => (Some(true), Some(closes)),
                Some(_) => (Some(false), Some(false)),
                None if token_rows[r].0 == k => (Some(false), Some(true)),
                None => (Some(false), Some(false)),
            };
            need[r].push(n);
            flip[r].push(f);
        }
    }
    let fits = |r: usize, m: u8| !dead[r] && need[r].iter().enumerate().all(|(i, n)| n.is_none_or(|n| n == (m >> i & 1 == 1)));
    // The modes after a row's token, from `m`.
    let after = |r: usize, m: u8| -> Vec<u8> {
        let mut out = vec![m];
        for (i, f) in flip[r].iter().enumerate() {
            out = match f {
                Some(false) => out,
                Some(true) => out.into_iter().map(|m| m ^ 1 << i).collect(),
                None => out.into_iter().flat_map(|m| [m, m ^ 1 << i]).collect(),
            };
        }
        out
    };
    let rows_of = |kind: u16| -> Vec<usize> {
        (0..rows.len())
            .filter(|&r| (rows[r].kind == kind && kind != 0) || rows[r].keywords.iter().chain(&rows[r].soft).any(|k| k.1 == kind))
            .collect()
    };
    // The states each level returns to.
    let mut returns: HashMap<u16, Vec<u16>> = HashMap::default();
    for a in arms {
        // The innermost call returns from the level it enters; each outer one from the level of
        // the call inside it, which it returns to.
        let mut from = a.to;
        for &p in a.push.iter().rev() {
            returns.entry(states[usize::from(from)].level).or_default().push(p);
            from = p;
        }
    }
    let root = states.first().ok_or("no states")?.level;
    // The modes each state is entered in: a bit per mode.
    let mut seen = vec![[0u64; 4]; states.len()];
    let mut work = vec![(0u16, 0u8)];
    let enter = |seen: &mut Vec<[u64; 4]>, work: &mut Vec<(u16, u8)>, s: u16, m: u8| {
        let w = &mut seen[usize::from(s)][usize::from(m / 64)];
        if *w >> (m % 64) & 1 == 0 {
            *w |= 1 << (m % 64);
            work.push((s, m));
        }
    };
    seen[0][0] = 1;
    while let Some((s, m)) = work.pop() {
        let st = &states[usize::from(s)];
        if st.end.is_some() && st.level != root {
            for &p in returns.get(&st.level).into_iter().flatten() {
                enter(&mut seen, &mut work, p, m);
            }
        }
        for a in arms.iter().filter(|a| a.from == s) {
            // A mark takes no bytes, nor does a call (it switches on the token in the callee).
            let next: Vec<u8> = if marks.contains(&a.token) || a.call {
                vec![m]
            } else {
                rows_of(a.token).into_iter().filter(|&r| fits(r, m)).flat_map(|r| after(r, m)).collect()
            };
            for n in next {
                enter(&mut seen, &mut work, a.to, n);
            }
        }
    }
    Ok((0..states.len())
        .map(|s| (0..rows.len()).map(|r| (0..=255u8).any(|m| seen[s][usize::from(m / 64)] >> (m % 64) & 1 == 1 && fits(r, m))).collect())
        .collect())
}
