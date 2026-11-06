//! The stream program's interpreter (`09-compile-and-lower.md` §4.1): one operation at a time
//! over every block of a batch of files, the operation's kernel (`kernels.rs`) once per block.
//! Each file starts at a block boundary, as in the Metal set steps' layout
//! (`metal_set_steps.rs`), so a file's first block resets every forward carry and its last
//! block every backward one.
//!
//! The loops here are over operations and blocks; a kernel's own loops are over the 64 lanes of
//! a block. Nothing iterates over bytes, set bits, events or segments: the error records, which
//! do, are output (`output.rs`).

use super::kernels::{self, Lanes};
use super::ops::{Id, Op, Program};
use std::collections::HashMap;

#[derive(Clone)]
pub(in crate::code_rule_dsl) enum Val {
    M(Vec<u64>),
    L(Vec<Lanes>),
    Ms(Vec<Vec<u64>>),
}

/// Files laid out from block boundaries.
pub(in crate::code_rule_dsl) struct Batch {
    /// Each file's first block, block count and length.
    pub files: Vec<(usize, usize, usize)>,
    /// The bytes, then 64 zeros (`ucode` reads 3 bytes past a block).
    pub src: Vec<u8>,
    /// Per block: a file's first block, a file's last block, and the bits of real bytes.
    pub start: Vec<bool>,
    pub end: Vec<bool>,
    pub valid: Vec<u64>,
}

impl Batch {
    pub(in crate::code_rule_dsl) fn new(files: &[&[u8]]) -> Batch {
        let (mut layout, mut src, mut start, mut end, mut valid) = (vec![], vec![], vec![], vec![], vec![]);
        for f in files {
            let blocks = f.len().div_ceil(64);
            layout.push((start.len(), blocks, f.len()));
            src.extend_from_slice(f);
            src.resize(src.len().div_ceil(64) * 64, 0);
            for b in 0..blocks {
                start.push(b == 0);
                end.push(b + 1 == blocks);
                let left = f.len() - b * 64;
                valid.push(if left >= 64 { u64::MAX } else { (1u64 << left) - 1 });
            }
        }
        src.resize(src.len() + 64, 0);
        Batch { files: layout, src, start, end, valid }
    }
    pub(in crate::code_rule_dsl) fn blocks(&self) -> usize {
        self.valid.len()
    }
}

/// An error record: the file, and its text.
pub(in crate::code_rule_dsl) type Record = (usize, String);

pub(in crate::code_rule_dsl) struct Run<'a> {
    pub prog: &'a Program,
    pub batch: &'a Batch,
    pub vals: Vec<Option<Val>>,
    /// Per `Errors` operation, its records from its last run.
    pub records: HashMap<Id, Vec<Record>>,
}

type R<T> = Result<T, String>;

impl<'a> Run<'a> {
    pub(in crate::code_rule_dsl) fn new(prog: &'a Program, batch: &'a Batch) -> Self {
        Run { prog, batch, vals: vec![None; prog.ops.len()], records: HashMap::new() }
    }

    pub(in crate::code_rule_dsl) fn m(&self, id: Id) -> &Vec<u64> {
        match &self.vals[id] {
            Some(Val::M(m)) => m,
            Some(_) => panic!("value {id} is not a mask"),
            None => panic!("value {id} ({:?}) is not computed", self.prog.ops[id]),
        }
    }
    fn l(&self, id: Id) -> &Vec<Lanes> {
        match &self.vals[id] {
            Some(Val::L(l)) => l,
            _ => panic!("value {id} ({:?}) is not lanes", self.prog.ops[id]),
        }
    }

    pub(in crate::code_rule_dsl) fn exec(&mut self) -> R<()> {
        let last = self.prog.last_uses();
        let n = self.prog.ops.len();
        let mut frees: Vec<Vec<Id>> = vec![vec![]; n + 1];
        for (id, &l) in last.iter().enumerate() {
            if self.prog.live[id] && l < n {
                frees[l].push(id);
            }
        }
        // Per loop (by its `RoundsBegin`), the round it is in.
        let mut rounds: HashMap<usize, usize> = HashMap::new();
        let mut pc = 0;
        while pc < n {
            if !self.prog.live[pc] {
                pc += 1;
                continue;
            }
            let mut jump = None;
            match &self.prog.ops[pc] {
                Op::RoundsBegin => {
                    rounds.insert(pc, 1);
                }
                Op::RoundsEnd { begin, n: most, settle, carries } => {
                    let nb = self.batch.blocks();
                    let mut changed = vec![0u64; nb];
                    for &(c, x) in carries {
                        let (c, x) = (self.m(c), self.m(x));
                        for b in 0..nb {
                            changed[b] |= c[b] ^ x[b];
                        }
                    }
                    let any = changed.iter().fold(0, |a, &w| a | w) != 0;
                    let r = rounds[begin];
                    if (*settle && !any) || r == *most {
                        rounds.remove(begin);
                        self.vals[pc] = Some(Val::M(changed));
                    } else {
                        for &(c, x) in carries {
                            let v = self.m(x).clone();
                            self.vals[c] = Some(Val::M(v));
                        }
                        rounds.insert(*begin, r + 1);
                        jump = Some(*begin + 1);
                    }
                }
                Op::Errors { parts, all, byte } => {
                    let parts: Vec<(&str, &Vec<u64>)> = parts.iter().map(|(msg, m)| (msg.as_str(), self.m(*m))).collect();
                    let recs = super::output::records(self.batch, &parts, *all, *byte);
                    self.records.insert(pc, recs);
                }
                op => {
                    let v = self.op(op)?;
                    self.vals[pc] = Some(v);
                }
            }
            match jump {
                Some(to) => pc = to,
                None => {
                    for &id in &frees[pc] {
                        self.vals[id] = None;
                    }
                    pc += 1;
                }
            }
        }
        Ok(())
    }

    fn mask_map(&self, f: impl Fn(usize) -> u64) -> Val {
        let valid = &self.batch.valid;
        Val::M((0..valid.len()).map(|b| f(b) & valid[b]).collect())
    }
    fn lane_map(&self, f: impl Fn(usize) -> Lanes) -> Val {
        Val::L((0..self.batch.blocks()).map(f).collect())
    }

    fn op(&self, op: &Op) -> R<Val> {
        let bt = self.batch;
        let nb = bt.blocks();
        Ok(match op {
            Op::Empty => Val::M(vec![0; nb]),
            Op::All => Val::M(bt.valid.clone()),
            Op::FileFirst => self.mask_map(|b| bt.start[b] as u64),
            Op::FileLast => self.mask_map(|b| if bt.end[b] { 1u64 << (63 - bt.valid[b].leading_zeros()) } else { 0 }),
            Op::Or(a, c) => {
                let (a, c) = (self.m(*a), self.m(*c));
                self.mask_map(|b| a[b] | c[b])
            }
            Op::And(a, c) => {
                let (a, c) = (self.m(*a), self.m(*c));
                self.mask_map(|b| a[b] & c[b])
            }
            Op::AndNot(a, c) => {
                let (a, c) = (self.m(*a), self.m(*c));
                self.mask_map(|b| a[b] & !c[b])
            }
            Op::Xor(a, c) => {
                let (a, c) = (self.m(*a), self.m(*c));
                self.mask_map(|b| a[b] ^ c[b])
            }
            Op::Not(a) => {
                let a = self.m(*a);
                self.mask_map(|b| !a[b])
            }
            Op::Pattern { k, c } => {
                let words = kernels::pattern_words(*k, *c);
                self.mask_map(|b| words[(b * 64) % *k as usize])
            }
            Op::Prev { a, k } => {
                let a = self.m(*a);
                self.mask_map(|b| kernels::prev(a[b], if bt.start[b] { 0 } else { a[b - 1] }, *k))
            }
            Op::Next { a, k } => {
                let a = self.m(*a);
                self.mask_map(|b| kernels::next(a[b], if bt.end[b] { 0 } else { a[b + 1] }, *k))
            }
            Op::Toggle(a) => {
                let a = self.m(*a);
                let mut out = vec![0; nb];
                let mut carry = 0;
                for b in 0..nb {
                    carry *= !bt.start[b] as u64;
                    out[b] = kernels::toggle(a[b], &mut carry) & bt.valid[b];
                }
                Val::M(out)
            }
            Op::Fill { s, e } => {
                let (s, e) = (self.m(*s), self.m(*e));
                let mut out = vec![0; nb];
                let mut carry = 0;
                for b in 0..nb {
                    carry *= !bt.start[b] as u64;
                    out[b] = kernels::fill(s[b], e[b], &mut carry) & bt.valid[b];
                }
                Val::M(out)
            }
            Op::BackFill { s, e } => {
                let (s, e) = (self.m(*s), self.m(*e));
                let mut out = vec![0; nb];
                let mut carry = 0;
                for b in (0..nb).rev() {
                    carry *= !bt.end[b] as u64;
                    out[b] = kernels::back_fill(s[b], e[b], &mut carry) & bt.valid[b];
                }
                Val::M(out)
            }
            Op::Classify(g) => {
                let table = &self.prog.groups[*g];
                self.lane_map(|b| kernels::classify(&bt.src[b * 64..b * 64 + 64], table))
            }
            Op::LaneTest { a, bit } => {
                let a = self.l(*a);
                self.mask_map(|b| kernels::bits(|l| a[b][l] & bit != 0))
            }
            Op::LaneConst(v) => self.lane_map(|_| [*v; 64]),
            Op::LaneSelect { m, a, b: c } => {
                let (m, a, c) = (self.m(*m), self.l(*a), self.l(*c));
                self.lane_map(|b| kernels::lane_select(m[b], &a[b], &c[b]))
            }
            Op::LaneMap { a, table } => {
                let a = self.l(*a);
                self.lane_map(|b| kernels::lane_map(&a[b], table))
            }
            Op::LaneEq { a, k } => {
                let a = self.l(*a);
                self.mask_map(|b| kernels::bits(|l| a[b][l] == *k))
            }
            Op::LaneGe { a, k } => {
                let a = self.l(*a);
                self.mask_map(|b| kernels::bits(|l| a[b][l] >= *k))
            }
            Op::LaneEqLanes(a, c) => {
                let (a, c) = (self.l(*a), self.l(*c));
                self.mask_map(|b| kernels::bits(|l| a[b][l] == c[b][l]))
            }
            Op::LanePrev { a, init } => {
                let a = self.l(*a);
                self.lane_map(|b| kernels::lane_prev(&a[b], if bt.start[b] { *init } else { a[b - 1][63] }))
            }
            Op::MapScan { entries, init } => {
                let entries: Vec<&Vec<Lanes>> = entries.iter().map(|&e| self.l(e)).collect();
                let mut out = Vec::with_capacity(nb);
                let mut carry = *init;
                for b in 0..nb {
                    if bt.start[b] {
                        carry = *init;
                    }
                    let block: Vec<&Lanes> = entries.iter().map(|e| &e[b]).collect();
                    out.push(kernels::map_scan(&block, &mut carry));
                }
                Val::L(out)
            }
            Op::Count { up, down, reset, first, max } => {
                let (u, d) = (self.m(*up), self.m(*down));
                let r = reset.map(|r| self.m(r));
                let f = first.map(|f| self.m(f));
                let mut out = Vec::with_capacity(nb);
                let mut carry = 0u8;
                for b in 0..nb {
                    carry *= !bt.start[b] as u8;
                    out.push(kernels::count(u[b], d[b], r.map_or(0, |r| r[b]), f.map_or(0, |f| f[b]), *max, &mut carry));
                }
                Val::L(out)
            }
            Op::Ucode { over, classes } => {
                let over = self.m(*over);
                let mut blocks = Vec::with_capacity(nb);
                let mut covers = [0u64; 3];
                for b in 0..nb {
                    let keep = !bt.start[b] as u64;
                    let cover_in = keep * ((covers[0] >> 63) | (covers[1] >> 62) | (covers[2] >> 61));
                    let (blk, _) = kernels::ucode_block(&bt.src[b * 64..b * 64 + 67], over[b], cover_in, classes);
                    covers = blk.covers;
                    blocks.push(blk);
                }
                let outs = (0..classes.len())
                    .map(|k| {
                        (0..nb)
                            .map(|b| {
                                let h = &blocks[b].hits[k];
                                let before = if bt.start[b] { [0; 4] } else { blocks[b - 1].hits[k] };
                                (h[0] | kernels::prev(h[1], before[1], 1) | kernels::prev(h[2], before[2], 2) | kernels::prev(h[3], before[3], 3)) & bt.valid[b]
                            })
                            .collect()
                    })
                    .collect();
                Val::Ms(outs)
            }
            Op::Pick(id, k) => match &self.vals[*id] {
                Some(Val::Ms(v)) => Val::M(v[*k].clone()),
                _ => return Err(format!("pick {k} of {id}: not a multi-mask")),
            },
            Op::Carry(init) => Val::M(self.m(*init).clone()),
            Op::RoundsBegin | Op::RoundsEnd { .. } | Op::Errors { .. } => unreachable!("run by exec"),
        })
    }
}
