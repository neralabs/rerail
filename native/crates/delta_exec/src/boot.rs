//! The boot's run, its first two lines (decision 0686, B6; B§8's `run`):
//!
//! ```text
//! pub fn run(l0: BlobId, globals: Facts, world: &dyn World, last: Option<&Pins>) -> Pins {
//!     let tape = world.tape(l0, Grammar::Rr);                  // the rr parser, as a world read
//!     let mut program = k0_pass(tape.since(last), last);        // L1: the file is its grain
//! ```
//!
//! `DocSet` is B§7's first answerer (path → bytes → parse → document) and the document set it keeps (P§11.4: path →
//! tape id, partitioned by document): each read answers the document through the file layer (B4), interns a changed
//! tape (B5) into the run's globals, and moves the document set to its next state, the document's old row retracted
//! where the set keeps it (`drop_parts`) and its new row added; an unchanged document is the identity (no layer).
//!
//! `run` reads l0, then steps L1 over the document set since `last`. A trace has two states, the committed one and
//! the run's open layer on it (0686, "B2's trace: two states"), so `Pins` are where the last run left each trace,
//! and hold nothing back: a cold run (`last = None`) has every bistate from ∅ (L2), its outputs emptied first; a
//! later run is given the last run's pins (L3), and pins of any other state are refused. The instance's cache is
//! read as ∅ unless it names the run's before (L5). A run starts by retiring what the document set no longer names
//! (`DocSet::retire`), once the set moved since it last did: the slabs and symbols blocks that hold no blob of its
//! committed state, the run's before, as the residency retires its slabs whole after an execution. The last run's two
//! states stay readable until the next run starts. The rest of B§8's run (the loop, loads, the program's δ) comes
//! after the target.
//!
//! As agreed in B4, l0 is named by its path, which the file layer reads, not by a blob id the caller already holds.

use crate::contract::{Bench, Global, GlobalIn, Instance, Work};
use crate::dir::BlockId;
use crate::drop::{DropOps, drop_parts};
use crate::files::{BlobId, Doc, Files};
use crate::intern::{InternOps, Interned};
use crate::l1::{L1, tape_row, tape_shape};
use crate::op::Tiling;
use crate::sym::Atoms;
use crate::trace::{RollUp, Trace};

/// The document set as the file layer and Intern answer it: L1's tape port.
pub struct DocSet {
    pub files: Files,
    pub interned: Interned,
    pub iops: InternOps,
    pub drops: DropOps,
    /// A row a document (`l1::tape_shape`), its layers kept (no roll-up): a run's before is a layer in its chain.
    pub tape: Trace,
    /// The set's state and the current slab when it last retired (`retire`).
    retired: Option<(Option<BlockId>, Option<BlockId>)>,
}

impl DocSet {
    pub fn new(files: Files, iops: InternOps, drops: DropOps) -> Result<DocSet, String> {
        Ok(DocSet { files, interned: Interned::default(), iops, drops, tape: Trace::new(&tape_shape())?.with_roll_up(RollUp::Never), retired: None })
    }

    /// Retires what the set's committed state does not name, once the set or the current slab moved since it last
    /// did (as the residency retires slabs whole): the blocks that hold none of its live rows' blobs
    /// (`Blobs::retire`), and the answers that name what they held. The blobs retired.
    pub fn retire(&mut self, bench: &mut Bench) -> Result<usize, String> {
        let at = (self.tape.head(), bench.dir.current_slab());
        if self.retired == Some(at) {
            return Ok(0);
        }
        self.retired = Some(at);
        let rows = crate::contract::rows_at(&bench.dir, &self.tape, self.tape.head())?;
        let live: Vec<BlobId> = rows.iter().flat_map(|r| [1, 5, 9].map(|c| BlobId::of_words(&r[c..c + 4]))).collect(); // copy: derived (the live documents' blobs)
        let gone = bench.blobs.retire(&bench.reg, &mut bench.dir, &live)?;
        self.files.forget(&gone);
        self.interned.forget(&gone);
        Ok(gone.len())
    }

    /// Reads document `path` (B§8's first line): its answer; when it changed, its tape interned into `atoms`, and the
    /// document set's next state, its old row retracted where the set keeps it and its new row added.
    pub fn read(&mut self, bench: &mut Bench, atoms: Atoms, path: &str) -> Result<Doc, String> {
        let doc = self.files.answer(path, &bench.reg, &mut bench.blobs, &mut bench.dir)?;
        if doc.is_identity() {
            return Ok(doc);
        }
        let syms = self.interned.of(&bench.reg, &self.iops, &mut bench.dir, &mut bench.blobs, atoms, &doc)?;
        let d = atoms.find(&bench.dir, &doc.module).ok_or("the module, interned")?;
        let runs = self.tape.open(&mut bench.dir, 4);
        let written = (|| {
            {
                let b = bench.dir.block(runs).expect("just made");
                let pin = rerail_blocks::region::pin_block(&b, "the document set", true)?;
                rerail_blocks::layer::RunsW::new(&pin).push(&tape_row(d, &doc.after, &syms), 1).ok_or("the document set's runs, full")?;
            }
            if let Some(h) = self.tape.head() {
                drop_parts(&bench.reg, &self.drops, &mut bench.dir, h, &[d], -1, runs)?;
            }
            Ok::<(), String>(())
        })();
        if let Err(e) = written {
            bench.dir.release(runs);
            return Err(e);
        }
        self.tape.commit(&bench.reg, &bench.traces, &mut bench.dir, runs, Tiling::WHOLE)?;
        Ok(doc)
    }
}

/// Where a run left each trace it read or wrote (B§8's `Pins`): the document set's head, L1's outputs' heads, and
/// its globals' counts. A record: it holds no block (a trace has two states, and a run reads from the committed one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pins {
    pub tape: Option<BlockId>,
    pub outs: Vec<Option<BlockId>>,
    pub atoms: Global,
}

/// B§8's run, its first two lines: `l0` read by the file layer, its changed names interned into `globals`, then L1
/// over the document set since `last` (`None`: cold, every bistate from ∅, L1's outputs emptied first). Refused when
/// `last` is not where the last run left the traces. The run's pins, l0's answer and L1's work.
#[allow(clippy::too_many_arguments)]
pub fn run(l1: &L1, inst: &mut Instance, l0: &str, globals: Atoms, docs: &mut DocSet, bench: &mut Bench, last: Option<&Pins>) -> Result<(Pins, Doc, Work), String> {
    match last {
        Some(p) if p.tape != docs.tape.head() || p.outs != inst.heads() || p.atoms.block != globals.0 => {
            return Err(format!("pins of another state: {p:?} (a run starts where the last run left its traces)"));
        }
        Some(_) => {}
        None => inst.outs.iter_mut().for_each(|t| t.clear(&mut bench.dir)),
    }
    docs.retire(bench)?;
    let doc = docs.read(bench, globals, l0)?;
    let count = u64::from(globals.count(&bench.dir));
    let ins = [(&docs.tape, last.and_then(|p| p.tape), docs.tape.head())];
    let atoms = [GlobalIn { block: globals.0, before: last.map(|p| p.atoms.count), after: count }];
    let work = inst.step(l1, &mut bench.cx(), &ins, &atoms)?;
    Ok((Pins { tape: docs.tape.head(), outs: inst.heads(), atoms: Global { block: globals.0, count } }, doc, work))
}
