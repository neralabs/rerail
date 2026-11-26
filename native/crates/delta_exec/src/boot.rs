//! The boot's run (decision 0686; B§8's `run`), the whole execution path, bootstrap and run alike:
//!
//! ```text
//! pub fn run(l0: BlobId, globals: Facts, world: &dyn World, last: Option<&Pins>) -> Pins {
//!     let tape = world.tape(l0, Grammar::Rr);                  // the rr parser, as a world read
//!     let mut program = k0_pass(tape.since(last), last);        // L1: the file is its grain
//!     let mut facts = globals;
//!     loop {
//!         let out = delta_phase(&program, &facts, world, last); // L2, L3, L4, main
//!         match out.program() {
//!             Some(next) => { program = next; facts = out.facts(); }
//!             None => return out.pin(),                         // the run's findings
//!         }
//!     }
//! }
//! ```
//!
//! `DocSet` is the document set the world's answerers give (P§11.4: path → tape id, grammar; indexed by path,
//! partitioned by document, each by its key from its path's hash, decision 0052 §3): each read is the load answer
//! (`files::World::load`, a world reader: its leaf recorded, misses too), then for a changed document the parse
//! answer by content (resident, else loaded from the store's objects, else parsed and put there) and the intern
//! answer (B5) into the run's globals; the set moves to its next state, the document's old row retracted where the
//! set keeps it (`drop_parts`) and its new row added; an unchanged document is the identity (no layer).
//!
//! `run`'s first two lines are under demand (P§7.1). The run's productions (l0's K-form) are L1's want. L1 asks of
//! the document set what it needs, and the world answers what is asked, through the three answerers
//! (`files::Load`, `Parse`, `Intern`). Their leaves go into the run's one layer of the world's leaves, and what
//! nothing asks for any more is let go. A run is rounds of L1's step and the world's answers, ending at the first round
//! that moves nothing.
//!
//! A trace has two states, the committed one and the run's open layer on it (0686, "B2's trace: two states"). So
//! `Pins` are where the last run left each trace, and hold nothing back.
//! - A cold run (`last = None`) has every bistate from ∅ (L2), and L1's traces are emptied first.
//! - A later run is given the last run's pins (L3). Pins of any other state are refused.
//!
//! The instance's cache is read as ∅ unless it names the run's before (L5). Slabs are the residency's:
//! - it holds each on a lease, renewed by every use, and ages it out `DROP_AFTER` after its last (P§11.4);
//! - it retires the least recently used slab whole, past its budget;
//! - a document whose slab went is loaded again from the store's objects where its places say (`DocSet::resident`).
//!
//! The loop is `phase::Levels`: each level is Δphase over the program the one before exported (L1's P2 first) and its
//! facts out (the run's `facts` first). A level's documents are the `load` answers its facts ask, read through the
//! document set like l0. Its program's δ is the change of its K-form rows: a changed module's rules removed and its new
//! ones run cold. A level that exports no program ends the loop, and its relations are the run's findings.
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
use crate::files::{Answer, Answerer, Doc, DocIds, Intern, Load, Load1, LeavesW, Parse, Reach, World, key_of, probe};
use crate::intern::InternOps;
use crate::kform::RELATIONS;
use crate::l1::{L1, L1Ports, TAPE_PATH, row_blobs, tape_row, tape_shape};
use crate::op::Tiling;
use crate::phase::{FactIn, Level, LevelPins, Levels, kform_syms};
use crate::sym::Atoms;
use crate::trace::{RollUp, Trace};
use rerail_blocks::bistate::{ColumnIndex, Read};
use rerail_blocks::block::kind;
use rerail_blocks::gens::BODY_AT;
use std::collections::BTreeSet;

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
    pub fn row_of(&self, dir: &Directory, atoms: Atoms, path: &str) -> Result<Option<Vec<i64>>, String> {
        self.row_at(dir, atoms, path)
    }

    /// Document `d`'s path, for messages (`#d` when the set holds none).
    pub fn path_of(&self, dir: &Directory, atoms: Atoms, d: i64) -> String {
        let path = || -> Option<i64> {
            let h = self.tape.head()?;
            let l = self.tape.read(dir, &[Some(h)]).ok()?;
            let table = l.table();
            let s = self.tape.state(&table, Some(h)).ok()?;
            let r = Read::matching(&s, &[ColumnIndex::new(0)], &[d]).next()?;
            Some(Read::get(&s, r, ColumnIndex::new(TAPE_PATH as u16)))
        };
        path().map_or_else(|| format!("#{d}"), |p| atoms.name(dir, p))
    }

    fn row_at(&self, dir: &Directory, atoms: Atoms, path: &str) -> Result<Option<Vec<i64>>, String> {
        let (Some(p), Some(h)) = (atoms.find(dir, path), self.tape.head()) else { return Ok(None) };
        let l = self.tape.read(dir, &[Some(h)])?;
        let table = l.table();
        let s = self.tape.state(&table, Some(h))?;
        // copy: derived (one row's cells, read where they lie)
        Ok(Read::matching(&s, &[ColumnIndex::new(TAPE_PATH as u16)], &[p]).next().map(|r| (0..Read::arity(&s) as u16).map(|c| Read::get(&s, r, ColumnIndex::new(c))).collect()))
    }

    /// The key of the document at `path` (decision 0052 §3, from its path's hash): its row's, when the set has one.
    pub fn key(&self, dir: &Directory, atoms: Atoms, path: &str) -> Result<i64, String> {
        Ok(match self.row_at(dir, atoms, path)? {
            Some(r) => r[0],
            None => key_of(path, |k| probe(dir, &self.tape, &[k]).ok().flatten().is_some()),
        })
    }

    /// The keys of the documents the set holds.
    pub fn keys(&self, dir: &Directory) -> Result<BTreeSet<i64>, String> {
        Ok(crate::contract::rows_at(dir, &self.tape, self.tape.head())?.into_iter().map(|r| r[0]).collect())
    }

    /// Document `d` let go: nothing asks for it any more (P§7, "a file goes as X → ∅ when nothing asks for it"), its
    /// row retracted where the set keeps it and its place forgotten.
    pub fn forget(&mut self, bench: &mut Bench, d: i64) -> Result<(), String> {
        let Some(h) = self.tape.head() else { return Ok(()) };
        let runs = self.tape.open(&mut bench.dir, 4);
        if let Err(e) = drop_parts(&bench.reg, &self.drops, &mut bench.dir, h, &[d], -1, runs) {
            bench.dir.release(runs);
            return Err(e);
        }
        self.tape.commit(&bench.reg, &bench.traces, &mut bench.dir, runs, Tiling::WHOLE)?;
        self.world.place(&bench.reg, &bench.traces, &mut bench.dir, d, None)
    }

    /// Reads document `path` as module `module` (B§8's first line; the engine's load names both) through the three
    /// answerers (P§12 item 7), its leaf into the run's layer `leaves`: the load answer (its source; a miss refused,
    /// naming the path); when it changed, the parse answer (its tape) and the intern answer (its symbols, into
    /// `atoms`), its places, and the document set's next state, its old row retracted where the set keeps it
    /// (`drop_parts`) and its new row added; an unchanged document is the identity (no layer).
    pub fn read(&mut self, bench: &mut Bench, leaves: &mut LeavesW, atoms: Atoms, path: &str, module: &str) -> Result<Doc, String> {
        let kept = self.row_at(&bench.dir, atoms, path)?;
        let d = match &kept {
            Some(r) => r[0],
            None => self.key(&bench.dir, atoms, path)?,
        };
        let before = kept.as_ref().map(|r| row_blobs(r).0);
        let named = [Load1 { d, path: path.to_string(), module: module.to_string(), before }];
        let Bench { reg, traces, dir, blobs, .. } = &mut *bench;
        macro_rules! reach {
            () => {
                Reach { world: &mut self.world, reg, ops: traces, dir, blobs }
            };
        }
        let source = match (Load { at: reach!(), named: &named }).answer(&[d], leaves)? {
            Answer::Blob(s) => s,
            Answer::Miss => return Err(format!("{path}: not a readable file")),
            a => return Err(format!("{path}: the load answer is {a:?}")),
        };
        let blob = |a: Answer, what: &str| match a {
            Answer::Blob(b) => Ok(b),
            a => Err(format!("{path}: the {what} answer is {a:?}")),
        };
        let doc = match before {
            Some(b) if b.source == source => Doc { path: path.to_string(), module: module.to_string(), before, after: b },
            _ => {
                let tape = blob((Parse { at: reach!() }).answer(&source.words(), leaves)?, "parse")?;
                Doc { path: path.to_string(), module: module.to_string(), before, after: DocIds { source, tape } }
            }
        };
        if doc.is_identity() {
            return Ok(doc);
        }
        let key: Vec<i64> = std::iter::once(d).chain(doc.after.source.words()).chain(doc.after.tape.words()).collect(); // copy: derived (the intern answer's key)
        let syms = blob((Intern { at: reach!(), iops: &self.iops, atoms, named: &named }).answer(&key, leaves)?, "intern")?;
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
    /// used now: one whose source and tape the residency no longer holds (retired under the budget, aged out when its
    /// lease lapsed, or a process started since) is loaded from the store where its places say.
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
    /// L1's want (its productions) and its requests of each input port (P§7.1).
    pub want: Option<BlockId>,
    pub asks: Vec<Option<BlockId>>,
    /// Each level's traces (B§8's loop).
    pub levels: Vec<LevelPins>,
}

/// A kept pins block's first word.
const PINS_MAGIC: i64 = i64::from_le_bytes(*b"PINS0686");

/// A kept pins block's words before its requests' and outputs' heads: the magic, the atoms' count, the outputs' and
/// requests' counts, then the tape's, the world's three, the blobs' and the want's heads.
const HEAD: usize = 10;

/// The rounds a run may take (each a step of L1, then the world answering what it asked) before it is refused: the
/// boot's demand is l0 alone, answered in the round after it is first asked.
const MAX_ROUNDS: usize = 4;

impl Pins {
    fn heads(&self) -> Vec<Option<BlockId>> {
        [self.tape].into_iter().chain(self.world).chain([self.blobs, self.want]).chain(self.asks.iter().copied()).chain(self.outs.iter().copied()).collect()
    }

    /// The levels' words: their count, then each level's (`LevelPins::words`).
    fn level_words(&self) -> Vec<i64> {
        std::iter::once(self.levels.len() as i64).chain(self.levels.iter().flat_map(LevelPins::words)).collect()
    }

    /// Kept: a frozen block, its words the pins and a reference to every head, where the directory keeps what lasts.
    /// Its id (a later one's is larger: the newest pins are the largest id's).
    pub fn keep(&self, dir: &mut Directory) -> Result<BlockId, String> {
        let heads = self.heads();
        let words: Vec<i64> = [PINS_MAGIC, self.atoms.count as i64, self.outs.len() as i64, self.asks.len() as i64]
            .into_iter()
            .chain(heads.iter().map(|h| h.map_or(-1, |b| i64::from(b.0))))
            .chain(self.level_words())
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
        let levels = self.levels.iter().flat_map(LevelPins::heads).map(Some);
        let all: Vec<BlockId> = heads.into_iter().chain(levels).flatten().map(|b| b.0).collect::<BTreeSet<u32>>().into_iter().map(BlockId).collect();
        dir.refer_all(id, &all.iter().map(|&b| (b, BODY_AT)).collect::<Vec<_>>())?;
        dir.freeze(id)?;
        Ok(id)
    }

    /// The pins a kept run left in `dir`, and their block (the newest, the largest id, should a process have stopped
    /// between keeping one and letting the last go): `None` when none was kept. The atoms' block is the store's own,
    /// opened by its path: its count is the pins'.
    pub fn open(dir: &Directory) -> Result<Option<(BlockId, Pins)>, String> {
        let mut best: Option<(BlockId, Vec<i64>, Vec<LevelPins>)> = None;
        for id in dir.unreferred() {
            let l = dir.read("the run's pins", &[id])?;
            let b = l.block(0);
            let word = |i: usize| b.get(BODY_AT + 8 * i..BODY_AT + 8 * i + 8).map(|w| i64::from_le_bytes(w.try_into().expect("eight bytes")));
            if word(0) != Some(PINS_MAGIC) {
                continue;
            }
            let n = word(2).unwrap_or(0) as usize + word(3).unwrap_or(0) as usize;
            let words: Vec<i64> = (0..HEAD + n).map(word).collect::<Option<_>>().ok_or("a kept pins block, short")?; // copy: derived (the pins' words)
            let mut at = HEAD + n;
            let mut next = || -> Result<i64, String> {
                let x = word(at).ok_or("a kept pins block, short")?;
                at += 1;
                Ok(x)
            };
            let levels = (0..next()?).map(|_| LevelPins::of_words(&mut next)).collect::<Result<Vec<_>, _>>()?;
            best = Some((id, words, levels));
        }
        let Some((id, w, levels)) = best else { return Ok(None) };
        let head = |x: i64| (x >= 0).then_some(BlockId(x as u32));
        let asks = w[3] as usize;
        let n = HEAD + asks + w[2] as usize;
        Ok(Some((id, Pins {
            levels,
            tape: head(w[4]),
            world: [head(w[5]), head(w[6]), head(w[7])],
            blobs: head(w[8]),
            want: head(w[9]),
            asks: w[HEAD..HEAD + asks].iter().map(|&x| head(x)).collect(),
            outs: w[HEAD + asks..n].iter().map(|&x| head(x)).collect(),
            atoms: Global { block: BlockId(u32::MAX), count: w[1] as u64 },
        })))
    }
}

/// The levels a run may take before it is refused: each exported a program, and a chain that never stops exporting
/// is a program that reads itself again at every level.
const MAX_LEVELS: usize = 64;

/// B§8's run under demand (P§7.1). Its first two lines: the run's productions, l0's K-form (every relation of it, by
/// l0's key), are the want on L1; L1 asks of its tape port the documents it needs, and the world answers what is asked
/// (its load, parse and intern answers; the names it interns into `globals`). A run's first part is rounds: L1 stepped
/// over the document set since the last round, then every key asked answered (the first round's answers each kept
/// request again, the world's cover; a later round's, those asked since), until a round moves nothing. Then B§8's
/// loop: each level (`phase::Level`, Δphase) runs the program the one before it exported (L1's P2 first) over the
/// facts it exported (the run's `facts` first), and the loop ends at the first level that exports no program; what
/// no reader asks any more is let go. `last`: `None` is cold (every bistate from ∅, L1's and the levels' traces
/// emptied first); refused when not where the last run left the traces. The run's pins, kept where the directory
/// keeps what lasts; l0's answer; the run's work.
#[allow(clippy::too_many_arguments)]
pub fn run(l1: &L1, inst: &mut Instance, levels: &mut Levels, l0: &str, module: &str, globals: Atoms, facts: &[FactIn<'_>], docs: &mut DocSet, bench: &mut Bench, last: Option<&Pins>) -> Result<(Pins, Doc, Work), String> {
    match last {
        Some(p) => {
            let now = levels.pins();
            let differ: Vec<String> = [
                (p.tape != docs.tape.head(), "the document set".to_string()),
                (p.outs != inst.heads(), "L1's outputs".to_string()),
                (p.want != inst.want.head(), "L1's want".to_string()),
                (p.asks != inst.ask_heads(), "L1's asks".to_string()),
                (p.atoms.count != u64::from(globals.count(&bench.dir)), format!("the atoms ({} now)", globals.count(&bench.dir))),
                (p.levels != now, format!("the levels ({now:?} now)")),
            ]
            .into_iter()
            .filter_map(|(d, what)| d.then_some(what))
            .collect();
            if !differ.is_empty() {
                return Err(format!("pins of another state: {} (a run starts where the last run left its traces): {p:?}", differ.join(", ")));
            }
        }
        None => {
            inst.clear(&mut bench.dir);
            levels.clear(&mut bench.dir);
        }
    }
    docs.world.begin();
    let mut leaves = docs.world.leaves_w(&mut bench.dir);
    let ran = (|| {
        // The blobs of slabs the residency retired or dropped since, forgotten (their documents load again).
        bench.blobs.forget_dead(&bench.reg, &mut bench.dir)?;
        // The run's productions: the want on L1's outputs. A request is answered by the spelling the run names it by.
        let d = docs.key(&bench.dir, globals, l0)?;
        let productions: BTreeSet<(usize, i64)> = (0..RELATIONS.len()).map(|k| (k, d)).collect();
        let (wb, wa) = inst.set_want(&mut bench.cx(), &productions)?;
        let mut want = (wb, wa);
        let mut from = (last.and_then(|p| p.tape), last.map(|p| p.atoms.count));
        let (mut work, mut doc, mut answered) = (Work::default(), None, BTreeSet::new());
        let read_by_levels = levels.docs(&bench.dir)?;
        for round in 0.. {
            if round == MAX_ROUNDS {
                return Err(format!("a run past {MAX_ROUNDS} rounds: each asked what the last answered"));
            }
            docs.resident(bench)?;
            let count = u64::from(globals.count(&bench.dir));
            let ins = [(&docs.tape, from.0, docs.tape.head())];
            let atoms = [GlobalIn { block: globals.0, before: from.1, after: count }];
            work += inst.step(l1, &mut bench.cx(), &ins, &atoms, Some(want))?;
            from = (docs.tape.head(), Some(count));
            want = (wa, wa);
            // Demand reaches the world as reads: what is asked answered, what no reader asks any more let go.
            let asked = inst.asked(&bench.dir, L1Ports::TAPE.index())?;
            for gone in docs.keys(&bench.dir)?.difference(&asked) {
                if !read_by_levels.contains(gone) {
                    docs.forget(bench, *gone)?;
                }
            }
            for &k in &asked {
                if round > 0 && answered.contains(&k) {
                    continue;
                }
                if k != d {
                    return Err(format!("a request for document {k}, which the run names no path for"));
                }
                doc = Some(docs.read(bench, &mut leaves, globals, l0, module)?);
                answered.insert(k);
            }
            if docs.tape.head() == from.0 && u64::from(globals.count(&bench.dir)) == count {
                break;
            }
        }
        // B§8's loop: each level over the export of the one before; the first that exports no program ends it.
        let mut k = 0;
        loop {
            // K-form's symbols as each level starts: a level's documents may name a K-form relation first (a level
            // that lowers `.decl` writes `k_decl`), interned as it loaded them.
            let ksyms = kform_syms(&bench.dir, globals);
            if k == MAX_LEVELS {
                return Err(format!("a run past {MAX_LEVELS} levels: each exported a program"));
            }
            if levels.levels.len() == k {
                levels.levels.push(Level::new(bench.dir.lasting())?);
            }
            let ops = levels.ops;
            let (done, rest) = levels.levels.split_at_mut(k);
            let program: Vec<_> = if k == 0 { inst.outs.iter().map(|t| Some((t, None, t.head()))).collect() } else { done[k - 1].program(&ksyms) };
            let owned;
            let ins: &[FactIn<'_>] = if k == 0 {
                facts
            } else {
                owned = done[k - 1].facts(&ksyms);
                &owned
            };
            let r = rest[0].step(ops, bench, docs, &mut leaves, globals, &program, ins).map_err(|e| format!("level {}: {e}", k + 2))?;
            work += r.work;
            k += 1;
            // A level run from ∅: every level after it reads its export from ∅ too.
            if r.reset {
                levels.clear_from(&mut bench.dir, k);
            }
            if !r.program {
                break;
            }
        }
        // The documents no reader asks for any more let go.
        let keep: BTreeSet<i64> = inst.asked(&bench.dir, L1Ports::TAPE.index())?.into_iter().chain(levels.docs(&bench.dir)?).collect();
        for gone in docs.keys(&bench.dir)?.difference(&keep) {
            docs.forget(bench, *gone)?;
        }
        // Their deltas read, the run's befores are no longer layers anyone needs: rolled up (P§11.4's roll-up, the
        // document set's and each level's), so the next run's before is one head; each level then sees its facts in
        // where they are now.
        for (j, l) in levels.levels[..k].iter_mut().enumerate() {
            l.finish(bench).map_err(|e| format!("level {}'s roll-up: {e}", j + 2))?;
        }
        let ksyms = kform_syms(&bench.dir, globals);
        for j in 1..k {
            let (done, rest) = levels.levels.split_at_mut(j);
            rest[0].saw(&done[j - 1].facts(&ksyms));
        }
        docs.tape.roll_up_run(&bench.reg, &bench.traces, &mut bench.dir, last.and_then(|p| p.tape), Tiling::WHOLE)?;
        inst.roll_up_want(&mut bench.cx(), last.and_then(|p| p.want))?;
        let doc = doc.ok_or("l0 was not asked")?;
        // The atoms where the run left them: the levels' documents intern too. They are append-only (P§13: a trace
        // with no kills), a symbol's id and name fixed once interned, so what an operator read of them holds at the
        // later count, and the next run's before is this one's end.
        Ok::<_, String>((doc, work, u64::from(globals.count(&bench.dir))))
    })();
    // The leaves the run read, its layer of the world's leaves (D3: misses too).
    let committed = docs.world.commit_leaves(&bench.reg, &bench.traces, &mut bench.dir, leaves);
    docs.world.end();
    let ran = committed.and(ran);
    let (doc, work, count) = ran?;
    let pins = Pins { tape: docs.tape.head(), outs: inst.heads(), atoms: Global { block: globals.0, count }, world: docs.world.heads(), blobs: bench.blobs.head(), want: inst.want.head(), asks: inst.ask_heads(), levels: levels.pins() };
    if bench.dir.lasting() == crate::dir::PLACE_KEPT {
        let id = pins.keep(&mut bench.dir)?;
        if let Some(old) = docs.kept.replace(id) {
            bench.dir.release(old);
        }
    }
    Ok((pins, doc, work))
}
