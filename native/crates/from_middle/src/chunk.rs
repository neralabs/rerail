//! The block program over a range of a file's blocks, from an explicit start.
//!
//! `BlockProgram::all_values` runs every carried operation from zero at a file's first block,
//! and walks a region group over the whole file. A chunk in the middle has neither: it has a
//! start, the carry of every carried operation at its first block. This is the same
//! interpreter (`values_by`, in its rounds form: `rounds_values`) with that start explicit,
//! and the carries after its last block given back as its end.
//!
//! A start holds each carry by what it can still change (`Start`): a `prev` only the `k` bits
//! it shifts in, a toggle its spread bit, a fill or an escape its one bit. So two starts are
//! equal exactly when every run from them is the same.

use rerail_blocks::vector::Vector;
use rerail_compiler::block::{self, carry, BlockOp, BlockProgram, Group, MAX_ROUNDS};
use std::ops::Range;

/// The carried state at a block edge: what a chunk starting there needs and a chunk ending
/// there gives.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Start {
    /// Each carried operation outside every group (`Prepared::carried`, in that order): a
    /// `prev`'s top bits (the `k` it shifts in, low in the word), else its carry.
    pub ops: Vec<u64>,
    /// Each rounds group's members' carries (in `Group::members`' order), held as `ops`'.
    pub groups: Vec<Vec<u64>>,
    /// Each back fill's carry the other way (`Prepared::backs`, in that order): in a start, the
    /// carry into the chunk's last block from the block after it; in an end, the carry out of the
    /// chunk's first block, to the chunk before.
    pub backs: Vec<u64>,
}

/// One step of the interpreter's order: an operation over the chunk's blocks, or a group's
/// rounds block by block.
#[derive(Clone, Copy, Debug)]
enum Step {
    Op(usize),
    Group(usize),
}

/// A block program made ready to run in chunks: its groups and its order, once.
pub struct Prepared<'p> {
    pub program: &'p BlockProgram,
    pub groups: Vec<Group>,
    order: Vec<Step>,
    /// The carried operations outside every group, in op order: a start's `ops`.
    pub carried: Vec<usize>,
    /// The back fills, in op order: a start's `backs`.
    pub backs: Vec<usize>,
    /// Each operation's place in `carried`, or a back fill's in `backs` (`usize::MAX`: none).
    slot: Vec<usize>,
    /// The fills proved within a run, by operation, each with the `prev(R, 1)` that bounds its
    /// carry (`solve::bounds`): proved once a program.
    pub bounds: std::sync::OnceLock<Vec<(usize, usize)>>,
}

/// A file's blocks as the interpreter reads them: padded past its end by the program's delay,
/// with each block's byte before (`block::before_blocks`), read in place by every chunk.
pub struct File {
    pub blocks: Vector<[u8; 64]>,
    before: Vector<[u8; 64]>,
    pub len: usize,
}

impl File {
    pub fn new(program: &BlockProgram, bytes: &[u8]) -> File {
        let blocks = block::padded_by(bytes, usize::from(program.delay));
        let before = block::before_blocks(&blocks);
        File { blocks, before, len: bytes.len() }
    }

    pub fn count(&self) -> usize {
        self.blocks.len()
    }

    /// Block `i`'s bytes before (`block::before_blocks`): the byte before each of its bytes.
    pub fn before_of(&self, i: usize) -> &[u8; 64] {
        &self.before[i]
    }
}

/// A `prev`'s carry as a start holds it: the `k` bits it shifts in.
fn held_prev(previous: u64, k: u8) -> u64 {
    if k == 0 { 0 } else { previous >> (64 - u32::from(k)) }
}

/// A held `prev` carry back as the previous word it stands for.
fn word_prev(held: u64, k: u8) -> u64 {
    if k == 0 { 0 } else { held << (64 - u32::from(k)) }
}

impl<'p> Prepared<'p> {
    /// The program's groups and order, as `values_by` makes them with every group run as rounds.
    /// Refused: a row program.
    pub fn new(program: &'p BlockProgram) -> Result<Self, String> {
        if program.rows.is_some() {
            return Err("a row program".into());
        }
        let ops = &program.ops;
        let n = ops.len();
        let groups = block::groups(ops, &program.rounds)?;
        let mut owner: Vec<Option<usize>> = vec![None; n];
        for (g, group) in groups.iter().enumerate() {
            for &m in group.members.iter() {
                owner[m] = Some(g);
            }
        }
        // The groups each value waits on: its own, or one a value it reads waits on.
        let mut waits: Vec<Vec<usize>> = vec![Vec::new(); n];
        for x in 0..n {
            let mut w: Vec<usize> = owner[x].into_iter().collect();
            for o in ops[x].operands().iter() {
                w.extend(waits[usize::from(*o)].iter().copied());
            }
            w.sort_unstable();
            w.dedup();
            waits[x] = w;
        }
        let last: Vec<usize> = groups.iter().map(|g| *g.members.last().expect("a member")).collect();
        let (mut ran, mut deferred) = (vec![false; groups.len()], vec![Vec::new(); groups.len()]);
        let mut order = Vec::with_capacity(n);
        for x in 0..n {
            if let Some(g) = owner[x] {
                if x == last[g] {
                    order.push(Step::Group(g));
                    order.extend(std::mem::take(&mut deferred[g]).into_iter().map(Step::Op));
                    ran[g] = true;
                }
                continue;
            }
            match waits[x].iter().copied().filter(|&g| !ran[g]).max_by_key(|&g| last[g]) {
                Some(g) => deferred[g].push(x),
                None => order.push(Step::Op(x)),
            }
        }
        let carried: Vec<usize> = (0..n).filter(|&x| owner[x].is_none() && matches!(ops[x], BlockOp::Prev(..) | BlockOp::Toggle(_) | BlockOp::Escaped(_) | BlockOp::Fill(..))).collect();
        let mut slot = vec![usize::MAX; n];
        for (s, &x) in carried.iter().enumerate() {
            slot[x] = s;
        }
        let backs: Vec<usize> = (0..n).filter(|&x| matches!(ops[x], BlockOp::BackFill(..))).collect();
        for (s, &x) in backs.iter().enumerate() {
            slot[x] = s;
        }
        Ok(Prepared { program, groups, order, carried, backs, slot, bounds: std::sync::OnceLock::new() })
    }

    /// A file's start: every carry zero.
    /// The interpreter's order: `Ok(op)`, or `Err(group)` for a group's rounds.
    pub(crate) fn order_ops(&self) -> impl Iterator<Item = Result<usize, usize>> + '_ {
        self.order.iter().map(|s| match *s {
            Step::Op(x) => Ok(x),
            Step::Group(g) => Err(g),
        })
    }

    /// Op `x`'s place in a start's `ops` or `backs`.
    pub(crate) fn slot_of(&self, x: usize) -> usize {
        self.slot[x]
    }

    pub fn zero(&self) -> Start {
        Start { ops: vec![0; self.carried.len()], groups: self.groups.iter().map(|g| vec![0; g.members.len()]).collect(), backs: vec![0; self.backs.len()] }
    }

    /// The program's values over `file`'s blocks `range`, from `start`, and the carries after
    /// the range's last block. `values[x][i]` is value `x`'s word at block `range.start + i`.
    pub fn run(&self, file: &File, range: Range<usize>, start: &Start) -> (Vec<Vec<u64>>, Start) {
        let program = self.program;
        let m = range.len();
        let mut values: Vec<Vec<u64>> = vec![Vec::new(); program.ops.len()];
        let mut end = start.clone();
        let at = |i: usize| range.start + i;
        for &step in &self.order {
            let x = match step {
                Step::Group(g) => {
                    self.rounds(g, range.clone(), &start.groups[g], &mut values, &mut end.groups[g]);
                    continue;
                }
                Step::Op(x) => x,
            };
            let v = |a: u16| &values[usize::from(a)];
            let two = |a: u16, b: u16, f: fn(u64, u64) -> u64| -> Vec<u64> { v(a).iter().zip(v(b)).map(|(&p, &q)| f(p, q)).collect() };
            let s = self.slot[x];
            let words: Vec<u64> = match program.ops[x] {
                BlockOp::Within(d) => (0..m).map(|i| block::within_at(at(i), file.len, d)).collect(),
                BlockOp::Prefix(n, w) => (0..m).map(|i| block::prefix_at(&file.blocks[at(i)], at(i), file.len, n, w)).collect(),
                BlockOp::Phase(k, r) => (0..m).map(|i| block::phase(64 * at(i), k, r)).collect(),
                BlockOp::Position(j) => (0..m).map(|i| block::position(at(i), j)).collect(),
                BlockOp::Eq(byte) => (0..m).map(|i| block::eq(&file.blocks[at(i)], byte)).collect(),
                BlockOp::Class { table, bits } => {
                    let table = &program.tables[usize::from(table)];
                    (0..m).map(|i| block::class(&file.blocks[at(i)], table, bits)).collect()
                }
                BlockOp::Pair { before, table, bits } => {
                    let (a, b) = (&program.tables[usize::from(before)], &program.tables[usize::from(table)]);
                    (0..m).map(|i| block::pair(&file.before[at(i)], &file.blocks[at(i)], a, b, bits)).collect()
                }
                BlockOp::Const(c) => vec![c; m],
                BlockOp::Not(a) => v(a).iter().map(|&p| !p).collect(),
                BlockOp::And(a, b) => two(a, b, |p, q| p & q),
                BlockOp::Or(a, b) => two(a, b, |p, q| p | q),
                BlockOp::Xor(a, b) => two(a, b, |p, q| p ^ q),
                BlockOp::AndNot(a, b) => two(a, b, |p, q| p & !q),
                BlockOp::Early(a, k) => v(a).iter().map(|&p| p >> k).collect(),
                // The carried operations: one pass over the chunk's blocks, from the start's carry.
                BlockOp::Prev(a, k) => {
                    let mut c = word_prev(start.ops[s], k);
                    let out = v(a).iter().map(|&p| { let o = carry::prev(p, c, k); c = p; o }).collect();
                    end.ops[s] = held_prev(c, k);
                    out
                }
                BlockOp::Toggle(a) => self.pass(v(a).iter().map(|&p| (p, 0)), start.ops[s], &mut end.ops[s], |p, _, c| carry::toggle(p, c)),
                BlockOp::Escaped(a) => self.pass(v(a).iter().map(|&p| (p, 0)), start.ops[s], &mut end.ops[s], |p, _, c| carry::escaped(p, c)),
                BlockOp::Fill(a, b) => self.pass(v(a).iter().copied().zip(v(b).iter().copied()), start.ops[s], &mut end.ops[s], carry::fill),
                // Last block to first, from the carry into the chunk's last block.
                BlockOp::BackFill(a, b) => {
                    let (p, q) = (v(a), v(b));
                    let mut out = vec![0u64; m];
                    let mut c = start.backs[s];
                    for i in (0..m).rev() {
                        (out[i], c) = carry::back_fill(p[i], q[i], c);
                    }
                    end.backs[s] = c;
                    out
                }
                BlockOp::Carry(_) => unreachable!("a carry is its group's"),
                BlockOp::Cmp { .. } | BlockOp::Bit { .. } => unreachable!("refused by `new`"),
            };
            values[x] = words;
        }
        (values, end)
    }

    /// A 1-bit carried pass over the chunk's words, from carry `cin`; its carry out into `cout`.
    fn pass(&self, words: impl Iterator<Item = (u64, u64)>, cin: u64, cout: &mut u64, f: impl Fn(u64, u64, u64) -> (u64, u64)) -> Vec<u64> {
        let mut c = cin;
        let out = words.map(|(p, q)| { let (o, next) = f(p, q, c); c = next; o }).collect();
        *cout = c;
        out
    }

    /// Group `g`'s rounds over the chunk, block by block (`rounds_group`), its members' carries
    /// from `start` and after the last block into `end`.
    fn rounds(&self, g: usize, range: Range<usize>, start: &[u64], values: &mut [Vec<u64>], end: &mut [u64]) {
        let program = self.program;
        let group = &self.groups[g];
        let m = range.len();
        let mut local = vec![0u64; program.ops.len()];
        // Each member's carry in, as the word its operation takes (a `prev`'s previous word).
        let mut state: Vec<u64> = group.members.iter().zip(start).map(|(&x, &h)| match program.ops[x] {
            BlockOp::Prev(_, k) => word_prev(h, k),
            _ => h,
        }).collect();
        let mut outs = vec![0u64; group.members.len()];
        for &x in group.members.iter() {
            values[x] = vec![0; m];
        }
        let carry_of = |c: usize| group.carries.iter().position(|p| p.0 == c).expect("a carry of the group");
        for i in 0..m {
            let mut cur: Vec<u64> = group.carries.iter().map(|&(c, _)| match program.ops[c] {
                BlockOp::Carry(init) => values[usize::from(init)][i],
                _ => unreachable!("verified: a carry"),
            }).collect();
            let mut settled = false;
            for _ in 0..MAX_ROUNDS {
                for (k, &x) in group.members.iter().enumerate() {
                    let get = |o: u16| if group.members.binary_search(&usize::from(o)).is_ok() { local[usize::from(o)] } else { values[usize::from(o)][i] };
                    let (w, c) = match program.ops[x] {
                        BlockOp::Carry(_) => (cur[carry_of(x)], 0),
                        BlockOp::Not(a) => (!get(a), 0),
                        BlockOp::And(a, b) => (get(a) & get(b), 0),
                        BlockOp::Or(a, b) => (get(a) | get(b), 0),
                        BlockOp::Xor(a, b) => (get(a) ^ get(b), 0),
                        BlockOp::AndNot(a, b) => (get(a) & !get(b), 0),
                        BlockOp::Early(a, k) => (get(a) >> k, 0),
                        BlockOp::Prev(a, d) => (carry::prev(get(a), state[k], d), get(a)),
                        BlockOp::Toggle(a) => carry::toggle(get(a), state[k]),
                        BlockOp::Escaped(a) => carry::escaped(get(a), state[k]),
                        BlockOp::Fill(a, b) => carry::fill(get(a), get(b), state[k]),
                        op => unreachable!("a value that reads nothing is no member: {op:?}"),
                    };
                    local[x] = w;
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
            for &x in group.members.iter() {
                values[x][i] = local[x];
            }
            state.copy_from_slice(&outs);
        }
        for ((e, &x), &w) in end.iter_mut().zip(group.members.iter()).zip(&state) {
            *e = match program.ops[x] {
                BlockOp::Prev(_, k) => held_prev(w, k),
                _ => w,
            };
        }
    }
}
