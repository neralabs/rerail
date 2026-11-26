//! The boot's run, its first two lines (decision 0686, B6; B§8's `run`): the tape as a world read, then L1 over the
//! tape since the last run.
//!
//! ```text
//! let tape = world.read(path);                 // the file layer's answer, interned when changed
//! let mut program = k0_pass(tape.since(last), last);   // L1: the file is its grain
//! ```
//!
//! `Tape` is L1's tape and facts-in ports as a world read: each read answers the document through the file layer
//! (B4), interns a changed tape (B5), and moves the ports to their next state, a changed document's old rows gone and
//! its new rows added, an unchanged one's the identity (no layer). `run` steps L1's instance over the ports between
//! the last two states. The rest of B§8's run (the loop, loads, the program's δ) comes after the target.

use crate::contract::{Cx, Global, GlobalIn, InPort, Instance, Work, push_layer};
use crate::dir::{BlockId, Directory};
use crate::files::{Blobs, Doc, Files};
use crate::intern::{InternOps, Interned};
use crate::l1::{L1, tape_row, tape_shape};
use crate::sym::Atoms;
use crate::op::Registry;
use crate::trace::{RollUp, Trace, TraceOps};
use std::collections::HashMap;

/// L1's tape port and its facts in (the atoms), as the world reads them.
pub struct Tape {
    pub files: Files,
    pub blobs: Blobs,
    pub interned: Interned,
    pub iops: InternOps,
    pub atoms: Atoms,
    /// The tape port's trace, every state kept.
    pub ins: Vec<Trace>,
    /// Each read's heads, the first every port's ∅.
    pub heads: Vec<Vec<Option<BlockId>>>,
    /// Each read's globals: the atoms at their count then.
    pub globals: Vec<Vec<Global>>,
    /// Each document's row in the tape port now, by path.
    last: HashMap<String, Vec<i64>>,
}

impl Tape {
    pub fn new(files: Files, iops: InternOps, atoms: Atoms) -> Result<Tape, String> {
        let ins = vec![Trace::new(&tape_shape())?.with_roll_up(RollUp::Never)];
        let globals = vec![vec![Global { block: atoms.0, count: 0 }]];
        Ok(Tape { files, blobs: Blobs::default(), interned: Interned::default(), iops, atoms, ins, heads: vec![vec![None]], globals, last: HashMap::new() })
    }

    /// Reads document `path` (B§8's first line): its answer, and the ports' next state.
    pub fn read(&mut self, reg: &Registry, traces: &TraceOps, dir: &mut Directory, path: &str) -> Result<Doc, String> {
        let doc = self.files.answer(path, &mut self.blobs, dir)?;
        let mut now = self.heads.last().cloned().expect("a state");
        if !doc.is_identity() {
            let syms = self.interned.of(reg, &self.iops, dir, self.atoms, &self.blobs, &doc)?;
            let d = self.atoms.find(dir, &doc.module).ok_or("the module, interned")?;
            let blob = self.blobs.get(&doc.after).ok_or("the answer's blob")?;
            let row = tape_row(d, blob, syms);
            let mut pairs = vec![(row.clone(), 1)];
            if let Some(o) = self.last.get(path) {
                pairs.push((o.clone(), -1));
            }
            now[0] = Some(push_layer(reg, traces, dir, &mut self.ins[0], &pairs)?);
            self.last.insert(path.to_string(), row);
        }
        self.heads.push(now);
        self.globals.push(vec![Global { block: self.atoms.0, count: u64::from(self.atoms.count(dir)) }]);
        Ok(doc)
    }

    /// The ports between state `k - 1` and state `k` (`since(last)`: the last read's, `k` the newest).
    pub fn since(&self, k: usize) -> Vec<InPort<'_>> {
        self.ins.iter().enumerate().map(|(p, t)| (t, self.heads[k - 1][p], self.heads[k][p])).collect()
    }

    /// The globals between state `k - 1` and state `k`.
    pub fn globals_since(&self, k: usize) -> Vec<GlobalIn> {
        self.globals[k].iter().zip(&self.globals[k - 1]).map(|(a, b)| GlobalIn { block: a.block, before: Some(b.count), after: a.count }).collect()
    }

    /// The newest state's place.
    pub fn now(&self) -> usize {
        self.heads.len() - 1
    }
}

/// B§8's run, its first two lines: `path` read, then L1 stepped over the tape since the last read.
pub fn run(tape: &mut Tape, l1: &L1, inst: &mut Instance, cx: &mut Cx<'_>, path: &str) -> Result<(Doc, Work), String> {
    let doc = tape.read(cx.reg, cx.traces, cx.dir, path)?;
    let k = tape.now();
    let work = inst.step(l1, cx, &tape.since(k), &tape.globals_since(k))?;
    Ok((doc, work))
}
