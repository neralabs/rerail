//! How likely a chunk's starts are (the design page "Lexing From the Middle", §5).
//!
//! Some chunks give no refusal to end their ambiguity: a JSON chunk of numbers reads as numbers
//! from outside a string and as one long string from inside one. Each start then has a score,
//! from what it implies:
//!
//! ```text
//! score(start) = log P(start's state) + Σ over the regions open at it  log P(its length ≥ L)
//! ```
//!
//! - **The state** is the long-lived part of a start (`Keys`): which region each set has open,
//!   and each stack's depth. Its prior is how often a block edge of the corpus is in it.
//! - **The length** `L` of a region open at the start is how far into the chunk it reaches
//!   from that start: the first place its fill is known closed (the chunk's end if none). Its
//!   prior is how often a region of that kind in the corpus is at least that long.
//!
//! The score decides where work goes, never what the answer is: a chunk keeps the starts of
//! its best `M` states (`solve::Entries::Ranked`), and a start the join needs that was dropped
//! runs again from its now exact state (`join::join_with`).

use crate::chunk::{File, Prepared, Start};
use crate::solve::TStart;
use crate::starts::Component;
use rerail_compiler::block::BlockProgram;
use std::collections::HashMap;
use std::ops::Range;

/// The long-lived part of a start, by component: each set of region fills (at most one open at
/// a place), each toggle (a set of one: inside its parity, as JSON's strings), each stack's
/// levels.
#[derive(Clone, Debug)]
pub struct Keys {
    pub sets: Vec<Vec<usize>>,
    pub stacks: Vec<Vec<usize>>,
}

impl Keys {
    pub fn new(program: &BlockProgram, comps: &[Component]) -> Keys {
        let mut sets = crate::solve::regions(program, comps);
        sets.extend(comps.iter().enumerate().filter(|(_, c)| !c.in_place && matches!(program.ops[c.op], rerail_compiler::block::BlockOp::Toggle(_))).map(|(i, _)| vec![i]));
        Keys { sets, stacks: crate::solve::stacks(program, comps).into_iter().map(|s| s.levels).collect() }
    }

    /// Per set, its open kind (0: none, `k + 1`: kind `k`); per stack, its depth. A bit not
    /// known counts as 0.
    fn key(&self, bit: impl Fn(usize) -> Option<bool>) -> Vec<u8> {
        let mut k = Vec::with_capacity(self.sets.len() + self.stacks.len());
        for set in &self.sets {
            k.push(set.iter().position(|&f| bit(f) == Some(true)).map_or(0, |i| i as u8 + 1));
        }
        for st in &self.stacks {
            k.push(st.iter().take_while(|&&l| bit(l) == Some(true)).count() as u8);
        }
        k
    }

    pub fn of_start(&self, comps: &[Component], s: &Start) -> Vec<u8> {
        self.key(|c| Some(comps[c].at.get(s) & 1 == 1))
    }

    pub fn of(&self, comps: &[Component], s: &TStart) -> Vec<u8> {
        self.key(|c| {
            let t = s.get(comps[c].at);
            (t.k & 1 == 1).then_some(t.v & 1 == 1)
        })
    }
}

/// The priors, measured on files: each state's count over the block edges, each region kind's
/// lengths.
#[derive(Clone, Debug)]
pub struct Priors {
    pub keys: Keys,
    states: HashMap<Vec<u8>, u64>,
    total: u64,
    /// Per set, per kind: the lengths of its regions in bytes, sorted.
    lengths: Vec<Vec<Vec<u32>>>,
}

impl Priors {
    /// The priors of `files` (those the program refuses left out), from the serial run.
    pub fn measure(p: &Prepared<'_>, comps: &[Component], files: &[Vec<u8>]) -> Priors {
        let program = p.program;
        let keys = Keys::new(program, comps);
        // Per set, each kind's lengths and, last, the lengths outside every kind of it.
        let mut lengths: Vec<Vec<Vec<u32>>> = keys.sets.iter().map(|s| vec![Vec::new(); s.len() + 1]).collect();
        let (mut states, mut total) = (HashMap::new(), 0u64);
        for bytes in files {
            if program.run(bytes).totals.iter().any(|&t| t != 0) {
                continue;
            }
            let file = File::new(program, bytes);
            let (want, _) = program.rounds_values(&[bytes]);
            let n = file.count();
            let mut start = p.zero();
            for i in 0..n {
                start.backs = p.backs.iter().map(|&x| want[x].get(i + 1).map_or(0, |w| w & 1)).collect();
                *states.entry(keys.of_start(comps, &start)).or_insert(0) += 1;
                total += 1;
                start = p.run(&file, i..i + 1, &start).1;
            }
            for (si, set) in keys.sets.iter().enumerate() {
                let words = |ki: usize, b: usize| if ki < set.len() { want[comps[set[ki]].op][b] } else { !set.iter().fold(0, |a, &f| a | want[comps[f].op][b]) };
                for ki in 0..=set.len() {
                    let mut run = 0u32;
                    for b in 0..n {
                        let w = words(ki, b);
                        for bit in 0..64 {
                            if w >> bit & 1 == 1 {
                                run += 1;
                            } else if run > 0 {
                                lengths[si][ki].push(run);
                                run = 0;
                            }
                        }
                    }
                    if run > 0 {
                        lengths[si][ki].push(run);
                    }
                }
            }
        }
        for l in lengths.iter_mut().flatten() {
            l.sort_unstable();
        }
        Priors { keys, states, total, lengths }
    }

    /// A start's log likelihood: its state's prior (add-one), and, per set, the run the start
    /// is in at least as long as it reaches into chunk `range`: the open kind's to its first
    /// place known closed, or outside every kind to the first place one is known open.
    pub fn score(&self, p: &Prepared<'_>, comps: &[Component], file: &File, range: Range<usize>, entry: &TStart) -> f64 {
        let key = self.keys.of(comps, entry);
        let seen = *self.states.get(&key).unwrap_or(&0) as f64;
        let mut s = ((seen + 1.0) / (self.total as f64 + self.states.len() as f64 + 1.0)).ln();
        let (values, _) = p.run_t(file, range.clone(), entry);
        for (si, set) in self.keys.sets.iter().enumerate() {
            // The run's end: the open kind known closed, or outside a kind known open.
            let ki = usize::from(key[si]).checked_sub(1).unwrap_or(set.len());
            let ends = |b: usize| -> u64 {
                if ki < set.len() {
                    let w = values[comps[set[ki]].op][b];
                    w.k & !w.v
                } else {
                    set.iter().fold(0, |a, &f| {
                        let w = values[comps[f].op][b];
                        a | (w.k & w.v)
                    })
                }
            };
            let mut len = 64 * range.len();
            for b in 0..range.len() {
                let e = ends(b);
                if e != 0 {
                    len = 64 * b + e.trailing_zeros() as usize;
                    break;
                }
            }
            let ls = &self.lengths[si][ki];
            let at_least = ls.len() - ls.partition_point(|&x| (x as usize) < len);
            s += ((at_least as f64 + 1.0) / (ls.len() as f64 + 2.0)).ln();
        }
        s
    }
}
