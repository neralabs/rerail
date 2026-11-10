//! The Bitlex compiler: a Bitlex file (`docs/parser-generators/03-bitlex.md`) to a block program
//! (`block`), printed as `rir` rows so it goes through the transducer's own assembler and
//! verifier. It holds no language: everything comes from the file.
//!
//! The parser is the reference evaluator's (`tools/bitlex`), so Bitlex is parsed in one place, and
//! the evaluator is the compiler's test oracle (`bitlex_compile_tests`).
//!
//! Built one construct at a time (`docs/parser-generators/13-tape-on-the-gpu.md` §7, step 2).
//!
//! - **Classes.** Each class is a union of products of a set of high nibbles and a set of low
//!   nibbles. Products are shared between classes and packed eight to a nibble table. A one-byte
//!   class is an `eq`.
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
use ::bitlex::syntax::{Arg, Expr, NExpr, Stmt};
use std::collections::HashMap;

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

pub struct Lowering {
    pub program: BlockProgram,
    /// Each operation's value, so an operation is emitted once.
    pub seen: HashMap<BlockOp, u16>,
    /// Each product's table and bit.
    pub products: HashMap<Product, (u16, u8)>,
    /// Classes, lowered when first used.
    pub classes: HashMap<String, Vec<(u32, u32)>>,
    pub names: HashMap<String, Val>,
    /// The masks any of whose bits sends the file to tree-sitter: errors and escapes.
    pub fallbacks: Vec<Val>,
    pub counts: HashMap<String, Count>,
    /// Each number's planes, bit 0 first (`NUMBER_BITS`).
    pub numbers: HashMap<String, Vec<Val>>,
    pub tokens: Vec<(String, Val, Option<TokenEnd>)>,
    pub keywords: Vec<(String, Vec<(Vec<u8>, Option<String>)>, bool)>,
    /// Each row's soft keywords: a word's kind where the grammar takes it, the row's elsewhere.
    pub soft: Vec<(String, Vec<Vec<u8>>)>,
    pub skip: Option<Val>,
    /// The tree section's rules (`node`, `choice`), in order, and its extra kinds.
    pub tree: Vec<Stmt>,
    pub extra: Vec<String>,
    /// Each `mark`, in order: its kind, its bits and whether the file's end has it; and the
    /// hidden ones' kinds.
    pub marks: Vec<(String, Val, bool)>,
    pub hidden: Vec<String>,
    /// Each `check`: a token kind and its grammar over the token's bytes.
    pub checks: Vec<(String, ::bitlex::syntax::Gram)>,
    /// Each `toggle(x)`: its value and `x`'s, the modes tier 2's states are read in
    /// (`row_modes`).
    pub toggles: Vec<(Val, Val)>,
    /// Each cost's source and its operations' range (`Compiled::costs`).
    pub costs: Vec<(String, usize, usize)>,
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
pub type Tier2 = (String, String, Vec<bool>, Vec<Val>);

/// A tree section's lowering (the grammar's state machine and tier 2's words), the engine's: it
/// adds its checks to `low` and gives tier 2's text. A program without one is a lexer's.
pub trait TreeSection {
    fn lower(&self, low: &mut Lowering, token_rows: &[(String, Val, Val)]) -> Result<Tier2, String>;
}

/// Compile `stmts`, with `outputs` (mask names, or `FALLBACK`) as the block program's first
/// outputs, then the token rows' masks. `symbols` names the grammar's ids; a program
/// with a tree section needs it.
pub fn compile(stmts: &[Stmt], outputs: &[&str], tree: Option<&dyn TreeSection>) -> Result<Compiled, String> {
    let mut low = Lowering {
        program: BlockProgram::default(),
        seen: HashMap::new(),
        products: HashMap::new(),
        classes: HashMap::new(),
        names: HashMap::new(),
        fallbacks: Vec::new(),
        counts: HashMap::new(),
        numbers: HashMap::new(),
        tokens: Vec::new(),
        keywords: Vec::new(),
        skip: None,
        tree: Vec::new(),
        extra: Vec::new(),
        marks: Vec::new(),
        hidden: Vec::new(),
        checks: Vec::new(),
        toggles: Vec::new(),
        soft: Vec::new(),
        costs: Vec::new(),
    };
    // Every class's products first: the tables come before the operations.
    for stmt in stmts {
        if let Stmt::Class(name, ranges) = stmt {
            for product in products(ranges) {
                low.product(product);
            }
            low.classes.insert(name.clone(), ranges.clone());
        }
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
        let tree = tree.ok_or("a tree section needs the grammar's ids")?;
        tier2 = Some(tree.lower(&mut low, &token_rows)?);
    }
    let mut vals: Vec<(String, Val)> = Vec::new();
    // The words read the fallback's totals (`EMPTY`), so a tree's program always has them.
    if has_tree && !outputs.contains(&FALLBACK) {
        for x in low.fallback()? {
            vals.push((FALLBACK.to_owned(), x));
        }
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
    // The stream tier 2 reads: every token's first byte, an extract, as simdjson's structural
    // indexes; and the last byte of each token whose end is not its row's to know (`SPAN`), a
    // mask tier 2 reads a word at a time.
    let mut stream = None;
    if let Some((_, _, takes_last, mark_masks)) = &tier2 {
        let mut starts = token_rows[0].1;
        for (_, s, _) in &token_rows[1..] {
            starts = low.or(starts, *s)?;
        }
        let mut lasts: Option<Val> = None;
        for ((_, _, l), _) in token_rows.iter().zip(takes_last).filter(|(_, t)| **t) {
            lasts = Some(match lasts {
                Some(x) => low.or(x, *l)?,
                None => *l,
            });
        }
        let lasts = match lasts {
            Some(x) => x,
            None => low.leaf(BlockOp::Const(0), false),
        };
        vals.push((STREAM_START.to_owned(), starts));
        vals.push((STREAM_LAST.to_owned(), lasts));
        stream = Some((vals.len() - 2, vals.len() - 1));
        for &m in mark_masks {
            vals.push((STREAM_MARK.to_owned(), m));
        }
    }
    let delay = vals.iter().map(|x| x.1.d).max().unwrap_or(0);
    // Each output once.
    let mut index: HashMap<u16, u16> = HashMap::new();
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
    if let Some((text, lowered, _, mark_masks)) = tier2 {
        let (s, l) = stream.expect("a tree section's stream");
        low.program.extracts = vec![at[s]];
        // The last bytes, then each mark's: `MARK`'s operand.
        low.program.masks = vec![at[l]];
        low.program.masks.extend((0..mark_masks.len()).map(|k| at[l + 1 + k]));
        // The fallback's totals, in `fallback()`'s order: `EMPTY t` reads total `t`.
        low.program.totals = vals.iter().zip(&at).filter(|((label, _), _)| label == FALLBACK).map(|(_, &v)| v).collect();
        words = lowered;
        machine = text;
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
    Ok(Compiled { rows: rows(&program), program, words, machine, labels, fallbacks, costs, origin, values })
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
                }
                // Both send the file to tree-sitter.
                Stmt::Error(_, expr) | Stmt::Escape(_, expr) => {
                    let value = self.expr(expr)?;
                    self.fallbacks.push(value);
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
                    let count = self.count(up, down, *max)?;
                    self.counts.insert(name.clone(), count);
                }
                Stmt::Token { kind, start, through, last } => {
                    let start = self.expr(start)?;
                    let through = match through {
                        Some(t) => Some(TokenEnd::Through(self.expr(t)?)),
                        None => match last {
                            Some(l) => Some(TokenEnd::Last(self.expr(l)?)),
                            None => None,
                        },
                    };
                    self.tokens.push((kind.clone(), start, through));
                }
                Stmt::Keywords { kind, words, soft, required } => {
                    self.keywords.push((kind.clone(), words.clone(), *required));
                    if !soft.is_empty() {
                        self.soft.push((kind.clone(), soft.clone()));
                    }
                }
                Stmt::Skip(expr) => {
                    let x = self.expr(expr)?;
                    self.skip = Some(match self.skip {
                        Some(s) => self.or(s, x)?,
                        None => x,
                    });
                }
                Stmt::Tree { body, .. } => self.stmts(body)?,
                // The grammar's rules: the state machine is built once the token rows are known.
                Stmt::Node { .. } | Stmt::Choice { .. } | Stmt::Refuse { .. } | Stmt::Reading { .. } | Stmt::Provisional(_) => self.tree.push(stmt.clone()),
                Stmt::Check { kind, body } => self.checks.push((kind.clone(), body.clone())),
                Stmt::Extra(kinds) => self.extra.extend(kinds.iter().cloned()),
                Stmt::Hidden(kinds) => self.hidden.extend(kinds.iter().cloned()),
                Stmt::Mark { kind, at, end } => {
                    let x = self.expr(at)?;
                    self.marks.push((kind.clone(), x, *end));
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

    pub fn product(&mut self, product: Product) -> (u16, u8) {
        if let Some(&at) = self.products.get(&product) {
            return at;
        }
        let n = self.products.len();
        let (table, bit) = ((n / 8) as u16, 1u8 << (n % 8));
        if usize::from(table) == self.program.tables.len() {
            self.program.tables.push(ClassTable::default());
        }
        let t = &mut self.program.tables[usize::from(table)];
        for h in 0..16 {
            if product.0 >> h & 1 == 1 {
                t.hi[h] |= bit;
            }
            if product.1 >> h & 1 == 1 {
                t.lo[h] |= bit;
            }
        }
        self.products.insert(product, (table, bit));
        (table, bit)
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

    /// `within`, the stream's bytes, at delay `d`: a leaf, never a carry.
    pub fn within(&mut self, d: u8) -> Result<u16, String> {
        if d > MAX_DELAY {
            return Err(format!("a delay of {d} bytes: more than {MAX_DELAY}"));
        }
        Ok(self.op(BlockOp::Within(d)))
    }

    pub fn two(&mut self, a: Val, b: Val, f: fn(u16, u16) -> BlockOp, dirty: bool) -> Result<Val, String> {
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

    /// A class: an `eq` for one byte, otherwise one `class` per table its products use, ORed.
    /// Dirty when the class holds the padding's byte, 0x00.
    pub fn class_set(&mut self, set: ByteSet) -> Val {
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
        let mut per_table: Vec<(u16, u8)> = Vec::new();
        for product in products(&ranges) {
            let (table, bit) = self.product(product);
            match per_table.iter_mut().find(|(t, _)| *t == table) {
                Some((_, bits)) => *bits |= bit,
                None => per_table.push((table, bit)),
            }
        }
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
                let mut operand = |me: &mut Self, i: usize| -> Result<Vec<Val>, String> {
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
    out.rounds = program.rounds.iter().filter(|g| keep[usize::from(g[0].0)]).map(|g| g.iter().map(|&(c, x)| (n(c), n(x))).collect()).collect();
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
        Stmt::Mask(n, _) | Stmt::Number(n, _) | Stmt::Error(n, _) | Stmt::Escape(n, _) => n.clone(),
        Stmt::Count { name, .. } => name.clone(),
        Stmt::Token { kind, .. } | Stmt::Mark { kind, .. } => kind.clone(),
        _ => String::new(),
    };
    format!("{} {name}", statement(stmt).to_lowercase())
}

pub fn statement(stmt: &Stmt) -> String {
    let text = format!("{stmt:?}");
    text.split(['(', ' ', '{']).next().unwrap_or("?").to_owned()
}
