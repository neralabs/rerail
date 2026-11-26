//! The boot's run, its first two lines (decision 0686, B6; B§8's `run`):
//!
//! ```text
//! pub fn run(l0: BlobId, globals: Facts, world: &dyn World, last: Option<&Pins>) -> Pins {
//!     let tape = world.tape(l0, Grammar::Rr);                  // the rr parser, as a world read
//!     let mut program = k0_pass(tape.since(last), last);        // L1: the file is its grain
//! ```
//!
//! `DocSet` is the document set the world's answerers give (P§11.4: path → tape id, grammar; indexed by path,
//! partitioned by document, each by its key from its path's hash, decision 0052 §3): each read is the load answer
//! (`files::World::load`, a world reader: its leaf recorded, misses too), then for a changed document the parse
//! answer by content (resident, else loaded from the store's objects, else parsed and put there) and the intern
//! answer (B5) into the run's globals; the set moves to its next state, the document's old row retracted where the
//! set keeps it (`drop_parts`) and its new row added; an unchanged document is the identity (no layer).
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
//! As agreed in B4, l0 is named by its path, which the file layer reads, not by a blob id the caller already holds; its
//! module is named by the caller, as the engine's load names it.
//!
//! What a run keeps lasts across processes where the directory keeps kept files: every trace's layers, the symbols
//! blocks, and the run's pins as a block that refers to every head (`Pins::keep`), with the store's atoms and objects
//! beside them. A later process opens them (`contract::Bench::open`, `Pins::open`) and its first run starts from those
//! pins: an unchanged root is L4's identity, no file read and nothing parsed.

use crate::contract::{Bench, Global, GlobalIn, Instance, Work};
use crate::dir::{BlockId, Directory};
use crate::drop::{DropOps, drop_parts};
use crate::files::{Doc, World, key_of, probe};
use crate::intern::InternOps;
use crate::l1::{L1, TAPE_PATH, row_blobs, tape_row, tape_shape};
use crate::op::Tiling;
use crate::sym::Atoms;
use crate::trace::{RollUp, Trace};
use rerail_blocks::bistate::{ColumnIndex, Read};
use rerail_blocks::block::kind;
use rerail_blocks::gens::BODY_AT;

/// The document set as the world and its answerers give it (B§7's first answerer, `load`, then `parse` and
/// `intern`): L1's tape port, P§11.4's document set.
pub struct DocSet {
    pub world: World,
    pub iops: InternOps,
    pub drops: DropOps,
    /// A row a document (`l1::tape_shape`): no roll-up while a run reads it (its before is a layer in its chain), then
    /// rolled up at the run's end (`Trace::roll_up_run`).
    pub tape: Trace,
    /// The last run's pins, kept (`Pins::keep`).
    kept: Option<BlockId>,
}

impl DocSet {
    /// The document set of `world`, at `head` (a kept run's; `None`: empty), placed where the directory keeps what
    /// lasts; `kept` the pins block that run left (let go when the next run keeps its own).
    pub fn new(world: World, iops: InternOps, drops: DropOps, dir: &Directory, head: Option<BlockId>, kept: Option<BlockId>) -> Result<DocSet, String> {
        let tape = Trace::new(&tape_shape())?.with_roll_up(RollUp::Never).resumed(dir.lasting(), head);
        Ok(DocSet { world, iops, drops, tape, kept })
    }

    /// The row of the document at `path`, by its path's symbol (the set's index on its path), when it has one.
    fn row_at(&self, dir: &Directory, atoms: Atoms, path: &str) -> Result<Option<Vec<i64>>, String> {
        let (Some(p), Some(h)) = (atoms.find(dir, path), self.tape.head()) else { return Ok(None) };
        let l = self.tape.read(dir, &[Some(h)])?;
        let table = l.table();
        let s = self.tape.state(&table, Some(h))?;
        // copy: derived (one row's cells, read where they lie)
        Ok(Read::matching(&s, &[ColumnIndex::new(TAPE_PATH as u16)], &[p]).next().map(|r| (0..Read::arity(&s) as u16).map(|c| Read::get(&s, r, ColumnIndex::new(c))).collect()))
    }

    /// Reads document `path` as module `module` (B§8's first line; the engine's load names both): its load answer;
    /// when it changed, its symbols (the intern answer, into `atoms`), its places, and the document set's next
    /// state, its old row retracted where the set keeps it (`drop_parts`) and its new row added; an unchanged document
    /// is the identity (no layer).
    pub fn read(&mut self, bench: &mut Bench, atoms: Atoms, path: &str, module: &str) -> Result<Doc, String> {
        let kept = self.row_at(&bench.dir, atoms, path)?;
        let d = match &kept {
            Some(r) => r[0],
            None => {
                let (dir, tape) = (&bench.dir, &self.tape);
                key_of(path, |k| probe(dir, tape, &[k]).ok().flatten().is_some())
            }
        };
        let before = kept.as_ref().map(|r| row_blobs(r).0);
        let doc = self.world.load(&bench.reg, &bench.traces, &mut bench.dir, &mut bench.blobs, d, path, module, before)?;
        if doc.is_identity() {
            return Ok(doc);
        }
        let syms = self.world.intern_answer(&bench.reg, &bench.traces, &self.iops, &mut bench.dir, &mut bench.blobs, atoms, &doc)?;
        self.world.place(&bench.reg, &bench.traces, &mut bench.dir, d, Some(doc.after.source))?;
        let p = atoms.find(&bench.dir, path).ok_or("the path, interned")?;
        let runs = self.tape.open(&mut bench.dir, 4);
        let written = (|| {
            {
                let b = bench.dir.block(runs).expect("just made");
                let pin = rerail_blocks::region::pin_block(&b, "the document set", true)?;
                rerail_blocks::layer::RunsW::new(&pin).push(&tape_row(d, p, rerail_rr_syntax::grammar()[1], &doc.after, &syms), 1).ok_or("the document set's runs, full")?;
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

    /// Every document of the set resident (B§8's `world.tape`: a tape read where the residency holds it), its slabs
    /// used now: one whose source and tape the residency no longer holds (retired under the budget, dropped after an
    /// execution, or a process started since) is loaded from the store where its places say.
    pub fn resident(&mut self, bench: &mut Bench) -> Result<(), String> {
        let rows = crate::contract::rows_at(&bench.dir, &self.tape, self.tape.head())?;
        for r in &rows {
            let (ids, syms) = row_blobs(r);
            self.world.resident(&bench.reg, &mut bench.dir, &mut bench.blobs, r[0], ids)?;
            for b in [ids.source, ids.tape, syms] {
                if let Some(p) = bench.blobs.get(&bench.dir, &b)? {
                    bench.dir.touch(p.block);
                }
            }
        }
        Ok(())
    }
}

/// Where a run left each trace it read or wrote (B§8's `Pins`): the document set's head, L1's outputs' heads, its
/// globals' counts, and the world's and the blobs' heads. A record: it holds no block (a trace has two states, and a
/// run reads from the committed one). Kept (`keep`) as a block that refers to every head, so a later process opens
/// each trace where the last run left it (`open`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pins {
    pub tape: Option<BlockId>,
    pub outs: Vec<Option<BlockId>>,
    pub atoms: Global,
    pub world: [Option<BlockId>; 3],
    pub blobs: Option<BlockId>,
}

/// A kept pins block's first word.
const PINS_MAGIC: i64 = i64::from_le_bytes(*b"PINS0686");

impl Pins {
    fn heads(&self) -> Vec<Option<BlockId>> {
        [self.tape].into_iter().chain(self.world).chain([self.blobs]).chain(self.outs.iter().copied()).collect()
    }

    /// Kept: a frozen block, its words the pins and a reference to every head, where the directory keeps what lasts.
    /// Its id (a later one's is larger: the newest pins are the largest id's).
    pub fn keep(&self, dir: &mut Directory) -> Result<BlockId, String> {
        let heads = self.heads();
        let words: Vec<i64> = [PINS_MAGIC, self.atoms.count as i64, self.outs.len() as i64]
            .into_iter()
            .chain(heads.iter().map(|h| h.map_or(-1, |b| i64::from(b.0))))
            .collect(); // copy: derived (the pins' words, written once)
        let id = dir.make_placed(dir.lasting(), kind::FACTS, 8 * words.len(), 0)?;
        {
            let b = dir.block(id).expect("just made");
            let mut w = rerail_blocks::region::pin_block(&b, "the run's pins", true)?;
            let bytes = w.bytes_mut().expect("a write pin");
            for (i, x) in words.iter().enumerate() {
                bytes[BODY_AT + 8 * i..BODY_AT + 8 * i + 8].copy_from_slice(&x.to_le_bytes()); // copy: derived (a word of the pins)
            }
        }
        for h in heads.into_iter().flatten() {
            dir.refer(id, h, BODY_AT)?;
        }
        dir.freeze(id)?;
        Ok(id)
    }

    /// The pins a kept run left in `dir`, and their block (the newest, the largest id, should a process have stopped
    /// between keeping one and letting the last go): `None` when none was kept. The atoms' block is the store's own,
    /// opened by its path: its count is the pins'.
    pub fn open(dir: &Directory) -> Result<Option<(BlockId, Pins)>, String> {
        let mut best: Option<(BlockId, Vec<i64>)> = None;
        for id in dir.unreferred() {
            let l = dir.read("the run's pins", &[id])?;
            let b = l.block(0);
            let word = |i: usize| b.get(BODY_AT + 8 * i..BODY_AT + 8 * i + 8).map(|w| i64::from_le_bytes(w.try_into().expect("eight bytes")));
            if word(0) != Some(PINS_MAGIC) {
                continue;
            }
            let n = word(2).unwrap_or(0) as usize;
            let words: Option<Vec<i64>> = (0..3 + 5 + n).map(word).collect(); // copy: derived (the pins' words)
            best = Some((id, words.ok_or("a kept pins block, short")?));
        }
        let Some((id, w)) = best else { return Ok(None) };
        let head = |x: i64| (x >= 0).then_some(BlockId(x as u32));
        Ok(Some((id, Pins {
            tape: head(w[3]),
            world: [head(w[4]), head(w[5]), head(w[6])],
            blobs: head(w[7]),
            outs: w[8..].iter().map(|&x| head(x)).collect(),
            atoms: Global { block: BlockId(u32::MAX), count: w[1] as u64 },
        })))
    }
}

/// B§8's run, its first two lines: `l0` read by the world (its load, parse and intern answers), its changed names
/// interned into `globals`, then L1 over the document set since `last` (`None`: cold, every bistate from ∅, L1's
/// outputs emptied first). Refused when `last` is not where the last run left the traces. The run's pins, kept where
/// the directory keeps what lasts; l0's answer; L1's work.
#[allow(clippy::too_many_arguments)]
pub fn run(l1: &L1, inst: &mut Instance, l0: &str, module: &str, globals: Atoms, docs: &mut DocSet, bench: &mut Bench, last: Option<&Pins>) -> Result<(Pins, Doc, Work), String> {
    match last {
        Some(p) if p.tape != docs.tape.head() || p.outs != inst.heads() || p.atoms.count != u64::from(globals.count(&bench.dir)) => {
            return Err(format!("pins of another state: {p:?} (a run starts where the last run left its traces)"));
        }
        Some(_) => {}
        None => inst.outs.iter_mut().for_each(|t| t.clear(&mut bench.dir)),
    }
    docs.world.begin();
    let ran = (|| {
        // The blobs of slabs the residency retired or dropped since, forgotten (their documents load again).
        bench.blobs.forget_dead(&bench.reg, &mut bench.dir)?;
        let doc = docs.read(bench, globals, l0, module)?;
        docs.resident(bench)?;
        let count = u64::from(globals.count(&bench.dir));
        let ins = [(&docs.tape, last.and_then(|p| p.tape), docs.tape.head())];
        let atoms = [GlobalIn { block: globals.0, before: last.map(|p| p.atoms.count), after: count }];
        let work = inst.step(l1, &mut bench.cx(), &ins, &atoms)?;
        // Its delta read, the run's before is no longer a layer the set needs: its layers rolled up (P§11.4's roll-up,
        // the document set's), so the next run's before is one head.
        docs.tape.roll_up_run(&bench.reg, &bench.traces, &mut bench.dir, last.and_then(|p| p.tape), Tiling::WHOLE)?;
        Ok::<_, String>((doc, work, count))
    })();
    docs.world.end();
    // An execution ended: the residency drops everything `DROP_AFTER` from now, unless another run comes first.
    bench.dir.executed();
    let (doc, work, count) = ran?;
    let pins = Pins { tape: docs.tape.head(), outs: inst.heads(), atoms: Global { block: globals.0, count }, world: docs.world.heads(), blobs: bench.blobs.head() };
    if bench.dir.lasting() == crate::dir::PLACE_KEPT {
        let id = pins.keep(&mut bench.dir)?;
        if let Some(old) = docs.kept.replace(id) {
            bench.dir.release(old);
        }
    }
    Ok((pins, doc, work))
}
