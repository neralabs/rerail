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
//! `run` is B§8's first two lines under demand (P§7.1). The run's productions (l0's K-form) are L1's want. L1 asks of
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
//! - it retires the least recently used slab whole, past its budget;
//! - it drops all of them `DROP_AFTER` an execution;
//! - a document whose slab went is loaded again from the store's objects where its places say (`DocSet::resident`).
//!
//! B§8's later lines (the loop over the program's levels, its loads and its δ) are the rest of the rebuild plan's
//! Figure 1, past the milestones of its §4, which build L1 and what it stands on.
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
use crate::drop::{Parts, drop_parts};
use crate::files::{Answer, Answerer, Doc, DocIds, Intern, Load, Load1, LeavesW, Parse, Reach, World, key_of, probe};
use crate::kform::RELATIONS;
use crate::contract::asks_shape;
use crate::k0::named_blocks;
use crate::l1::{DOC_COLS, L1, L1Ports, TAPE_ARITY, TAPE_PATH, row_blobs, tape_row, tape_shape};
use crate::semi::{self, How, Side};
use crate::op::Tiling;
use crate::sym::Atoms;
use crate::trace::{RollUp, Row, Trace, open_runs};
use rerail_blocks::bistate::{ColumnIndex, Read};
use rerail_blocks::block::kind;
use rerail_blocks::gens::BODY_AT;

/// The document set as the world and its answerers give it (B§7's first answerer, `load`, then `parse` and
/// `intern`): L1's tape port, P§11.4's document set.
pub struct DocSet {
    pub world: World,
    /// A row a document (`l1::tape_shape`): no roll-up while a run reads it (its before is a layer in its chain), then
    /// rolled up at the run's end (`Trace::roll_up_run`).
    pub tape: Trace,
    /// The last run's pins, kept (`Pins::keep`).
    kept: Option<BlockId>,
    /// The asks' head the set was last read against (`forget_unasked`; `None`: never): every document read since was
    /// asked there, so while the asks stay at it no document is unasked.
    covered: Option<Option<BlockId>>,
    /// The residency's counts (slabs retired, drops) when every document of the set was last made resident
    /// (`resident`): while they stay, no slab of it went, and a document come since was placed by its read, so the
    /// set is resident still.
    resident_at: Option<(u64, u64)>,
}

impl DocSet {
    /// The document set of `world`, at `head` (a kept run's; `None`: empty), placed where the directory keeps what
    /// lasts; `kept` the pins block that run left (let go when the next run keeps its own).
    pub fn new(world: World, dir: &Directory, head: Option<BlockId>, kept: Option<BlockId>) -> Result<DocSet, String> {
        let tape = Trace::new(&tape_shape())?.with_roll_up(RollUp::Never).resumed(dir.lasting(), head);
        Ok(DocSet { world, tape, kept, covered: None, resident_at: None })
    }

    /// The row of the document at `path`, by its path's symbol (the set's index on its path), when it has one.
    fn row_at(&self, dir: &Directory, atoms: Atoms, path: &str) -> Result<Option<Row>, String> {
        let (Some(p), Some(h)) = (atoms.find(dir, path), self.tape.head()) else { return Ok(None) };
        self.tape.reading(dir, Some(h), |s| Read::matching(s, &[ColumnIndex::new(TAPE_PATH as u16)], &[p]).next().map(|r| Row::of(s, r)))
    }

    /// The key of the document at `path` (decision 0052 §3, from its path's hash): its row's, when the set has one.
    pub fn key(&self, dir: &Directory, atoms: Atoms, path: &str) -> Result<i64, String> {
        Ok(match self.row_at(dir, atoms, path)? {
            Some(r) => r[0],
            None => key_of(path, |k| probe(dir, &self.tape, &[k]).ok().flatten().is_some()),
        })
    }

    /// The documents nothing asks for any more let go (P§7, "a file goes as X → ∅ when nothing asks for it"): one
    /// launch of `semi_change`, the set read cold against the asks of it (`asks`: none, nothing is asked), its rows
    /// whose document no ask holds (`How::UNMATCHED`), their documents into a parts block; then each such document's
    /// rows retracted where the set keeps them, and its places, by `drop_parts` over that block.
    pub fn forget_unasked(&mut self, bench: &mut Bench, asks: Option<&Trace>) -> Result<(), String> {
        // Asks where the set was last read against them: the set holds only what they ask (each read is of an ask).
        let at = asks.and_then(Trace::head);
        if self.covered == Some(at) {
            return Ok(());
        }
        let Some(h) = self.tape.head() else {
            self.covered = Some(at);
            return Ok(());
        };
        let n = self.tape.reading(&bench.dir, Some(h), |s| Read::end(s).get() as usize)?;
        let a = Side { before: None, after: Some(h), arity: TAPE_ARITY, col: 0 };
        let b = asks.map(|t| Side { before: None, after: t.head(), arity: 1, col: 0 });
        let nk = match asks {
            Some(t) => t.reading(&bench.dir, t.head(), |s| Read::end(s).get() as usize)?,
            None => 0,
        };
        let (rows, keys) = (self.tape.open(&mut bench.dir, n), open_runs(&mut bench.dir, &asks_shape().encode()?, nk));
        let release = |dir: &mut Directory| [rows, keys].into_iter().for_each(|x| dir.release(x));
        let parts = match Parts::open(&mut bench.dir, n) {
            Ok(p) => p,
            Err(e) => {
                release(&mut bench.dir);
                return Err(e);
            }
        };
        let chosen = semi::select(&bench.ops, &mut bench.dir, &a, b.as_ref().map(|b| (b, keys)), How::UNMATCHED, rows, parts).and_then(|()| Parts::sealed(&bench.dir, parts));
        release(&mut bench.dir);
        let parts = match chosen {
            Ok(p) => p,
            Err(e) => {
                bench.dir.release(parts);
                return Err(e);
            }
        };
        let dropped = self.drop_from(bench, &parts);
        parts.release(&mut bench.dir);
        if dropped.is_ok() {
            self.covered = Some(at);
        }
        dropped
    }

    /// The documents `parts` lists retracted from the set and from the world's places, where each keeps them.
    fn drop_from(&mut self, bench: &mut Bench, parts: &Parts) -> Result<(), String> {
        if parts.is_empty() {
            return Ok(());
        }
        for t in [&mut self.tape, &mut self.world.places] {
            let Some(h) = t.head() else { continue };
            let runs = t.open(&mut bench.dir, 4);
            if let Err(e) = drop_parts(&bench.ops, &mut bench.dir, h, parts, -1, runs) {
                bench.dir.release(runs);
                return Err(e);
            }
            t.commit(&bench.ops, &mut bench.dir, runs, Tiling::WHOLE)?;
        }
        Ok(())
    }

    /// Reads document `path` as module `module` (B§8's first line; the engine's load names both): `read_all` of it.
    pub fn read(&mut self, bench: &mut Bench, leaves: &mut LeavesW, atoms: Atoms, path: &str, module: &str) -> Result<Doc, String> {
        Ok(self.read_all(bench, leaves, atoms, &[(path, module)])?.pop().expect("one document read"))
    }

    /// The set's documents the journal names as changed since the last run (`World::changed`), read (P1: an edit's
    /// read is of what changed, not of every document); `module` names a path's module, as the caller named it when
    /// it first read it. A path the set holds no document of is not one of its documents: a new one is the caller's
    /// to name (`read_all`). `None` where the journal cannot say: every document is then the caller's to read.
    pub fn read_changed(&mut self, bench: &mut Bench, leaves: &mut LeavesW, atoms: Atoms, module: &dyn Fn(&str) -> String) -> Result<Option<Vec<Doc>>, String> {
        let Some(named) = self.world.changed() else { return Ok(None) };
        let mut held = Vec::new(); // copy: derived (the changed documents' paths and modules)
        for p in named {
            let Some(path) = p.to_str() else { continue };
            if self.row_at(&bench.dir, atoms, path)?.is_some() {
                held.push((path.to_string(), module(path)));
            }
        }
        let docs: Vec<(&str, &str)> = held.iter().map(|(p, m)| (p.as_str(), m.as_str())).collect();
        self.read_all(bench, leaves, atoms, &docs).map(Some)
    }

    /// Reads the documents `docs` names (each its path and its module; B§8's first line) through the three answerers
    /// (P§12 item 7), their leaves into the run's layer `leaves`: the load answer of all of them at once (one
    /// `answer_load`, and one `parse` of the sources no home holds; a miss refused, naming the path); for each that
    /// changed, the parse answer (its tape) and the intern answer (its symbols, into `atoms`) and its places; and the
    /// document set's next state, one change of it: each changed document's old row retracted where the set keeps it and
    /// its new row added (one `replace_rows`). An unchanged document is the identity; the read's writes are committed
    /// once. So a read of the set costs its launches in what changed, not in how many documents it holds (P1).
    pub fn read_all(&mut self, bench: &mut Bench, leaves: &mut LeavesW, atoms: Atoms, docs: &[(&str, &str)]) -> Result<Vec<Doc>, String> {
        let mut named = Vec::with_capacity(docs.len()); // copy: derived (the load δ's asks)
        for &(path, module) in docs {
            let kept = self.row_at(&bench.dir, atoms, path)?;
            let d = match &kept {
                Some(r) => r[0],
                None => self.key(&bench.dir, atoms, path)?,
            };
            named.push(Load1 { d, path: path.to_string(), module: module.to_string(), before: kept.as_ref().map(|r| row_blobs(r).0) });
        }
        let answers = {
            let Bench { ops, dir, blobs, .. } = &mut *bench;
            (Load { at: Reach { world: &mut self.world, ops, dir, blobs }, named: &named }).answer_all(leaves)?
        };
        let mut out = Vec::with_capacity(docs.len()); // copy: not block data (the documents read)
        let mut rows = Vec::new(); // copy: derived (the set's new rows, the answers)
        for (a, answer) in named.iter().zip(answers) {
            let source = match answer {
                Answer::Blob(s) => s,
                _ => return Err(format!("{}: not a readable file", a.path)),
            };
            let Bench { ops, dir, blobs, .. } = &mut *bench;
            macro_rules! reach {
                () => {
                    Reach { world: &mut self.world, ops, dir, blobs }
                };
            }
            let blob = |x: Answer, what: &str| match x {
                Answer::Blob(b) => Ok(b),
                x => Err(format!("{}: the {what} answer is {x:?}", a.path)),
            };
            let doc = match a.before {
                Some(b) if b.source == source => Doc { path: a.path.clone(), module: a.module.clone(), before: a.before, after: b },
                _ => {
                    let tape = blob((Parse { at: reach!() }).answer(&source.words(), leaves)?, "parse")?;
                    Doc { path: a.path.clone(), module: a.module.clone(), before: a.before, after: DocIds { source, tape } }
                }
            };
            if !doc.is_identity() {
                let (s, t) = (doc.after.source.words(), doc.after.tape.words());
                let key = [a.d, s[0], s[1], s[2], s[3], t[0], t[1], t[2], t[3]];
                let one = std::slice::from_ref(a);
                let syms = blob((Intern { at: reach!(), atoms, named: one }).answer(&key, leaves)?, "intern")?;
                self.world.place(&bench.ops, &mut bench.dir, a.d, Some(doc.after.source))?;
                let p = atoms.find(&bench.dir, &a.path).ok_or("the path, interned")?;
                rows.push(tape_row(a.d, p, rerail_rr_syntax::grammar()[1], &doc.after, &syms));
            }
            out.push(doc);
        }
        if rows.is_empty() {
            self.commit_read(bench, leaves, None)?;
            return Ok(out);
        }
        // The answers, written once where they are made: the documents' rows of the set. Its change of the set (each
        // old row gone where it differs, each new row come) is `replace_rows`'.
        let given = self.tape.open(&mut bench.dir, rows.len());
        let opened = self.tape.open(&mut bench.dir, 2 * rows.len());
        let release = |dir: &mut Directory| [given, opened].into_iter().for_each(|x| dir.release(x));
        let written = (|| {
            {
                let b = bench.dir.block(given).expect("just made");
                let pin = rerail_blocks::region::pin_block(&b, "the document set's answers", true)?;
                let w = rerail_blocks::layer::RunsW::new(&pin);
                for r in &rows {
                    w.push(r, 1).ok_or("the document set's answers, full")?;
                }
            }
            semi::replace(&bench.ops, &mut bench.dir, self.tape.head(), TAPE_ARITY, 1, given, opened)
        })();
        let runs = match written {
            Ok(r) => r,
            Err(e) => {
                release(&mut bench.dir);
                return Err(e);
            }
        };
        // The one not committed let go: the opened runs, when the set was empty and the answers are its change.
        bench.dir.release(if runs == given { opened } else { given });
        self.commit_read(bench, leaves, Some(runs))?;
        Ok(out)
    }

    /// A read's writes committed in one launch (`Trace::commit_wide`): the blobs it housed, the answers and places it
    /// kept, the leaves it read so far (the run's next in runs of their own) and the document set's change `tape`.
    fn commit_read(&mut self, bench: &mut Bench, leaves: &mut LeavesW, tape: Option<BlockId>) -> Result<(), String> {
        let Bench { ops, dir, blobs, .. } = &mut *bench;
        let w = &mut self.world;
        let mut items: Vec<(&mut Trace, BlockId)> = Vec::new(); // copy: not block data (the traces to commit)
        for t in [blobs.rows_mut(), &mut w.answers, &mut w.places] {
            if let Some(r) = t.take_written() {
                if crate::trace::holds_rows(dir, r)? {
                    items.push((t, r));
                } else {
                    dir.release(r);
                }
            }
        }
        if crate::trace::holds_rows(dir, leaves.runs)? {
            let next = w.leaves.open(dir, 64);
            items.push((&mut w.leaves, std::mem::replace(&mut leaves.runs, next)));
        }
        if let Some(r) = tape {
            items.push((&mut self.tape, r));
        }
        if items.is_empty() {
            return Ok(());
        }
        Trace::commit_wide(ops, dir, &mut items, Tiling::WHOLE)
    }

    /// Every document of the set resident (B§8's `world.tape`: a tape read where the residency holds it), its slabs
    /// used now: one whose source and tape the residency no longer holds (retired under the budget, dropped after an
    /// execution, or a process started since) is loaded from the store where its places say. Which documents those are,
    /// and which blocks the set names, is `named_blocks`' (the set joined with the blobs' rows, kept to the blocks the
    /// directory holds by the blob store's `forget_dead`); the loads are the file layer's and the touches the residency's.
    pub fn resident(&mut self, bench: &mut Bench) -> Result<(), String> {
        let Some(h) = self.tape.head() else { return Ok(()) };
        // Resident still: no slab retired or dropped since, and each document come since placed by its read (L4: an
        // identity run's are its checks; P1: an edit's, its change's). An eviction's look names every document's
        // blocks, the residency's event, not an edit's.
        let at = bench.dir.residency();
        if self.resident_at == Some(at) {
            return Ok(());
        }
        bench.blobs.forget_dead(&bench.ops, &mut bench.dir)?;
        let n = self.tape.reading(&bench.dir, Some(h), |s| Read::end(s).get() as usize)?;
        let (blocks, missing) = match bench.blobs.head() {
            Some(b) => named_blocks(&bench.ops, &mut bench.dir, h, TAPE_ARITY, n, b, DOC_COLS)?,
            // No blob at all: every document is to load (`change_rows` from ∅, the keys of the rows it adds).
            None => {
                let none = Parts::open(&mut bench.dir, 0)?;
                (Parts::sealed(&bench.dir, none)?, self.every_document(bench, h, n)?)
            }
        };
        let loaded = (|| {
            for i in 0..missing.len() {
                let d = nth(&bench.dir, &missing, i)?;
                let row = probe(&bench.dir, &self.tape, &[d])?.ok_or("a document the set names, its row")?;
                self.world.resident(&bench.ops, &mut bench.dir, &mut bench.blobs, d, row_blobs(&row).0)?;
            }
            blocks.each(&bench.dir, &mut |b| bench.dir.touch(BlockId(b as u32)))?;
            // The blobs loaded, housed, committed before a launch reads their rows.
            bench.blobs.commit(&bench.ops, &mut bench.dir)
        })();
        blocks.release(&mut bench.dir);
        missing.release(&mut bench.dir);
        if loaded.is_ok() {
            self.resident_at = Some(at);
        }
        loaded
    }

    /// Every document of the set at `h` (of `n` ids at most), a sealed parts block.
    fn every_document(&self, bench: &mut Bench, h: BlockId, n: usize) -> Result<Parts, String> {
        let rows = self.tape.open(&mut bench.dir, n);
        let parts = match Parts::open(&mut bench.dir, n) {
            Ok(p) => p,
            Err(e) => {
                bench.dir.release(rows);
                return Err(e);
            }
        };
        let a = Side { before: None, after: Some(h), arity: TAPE_ARITY, col: 0 };
        let ran = semi::select(&bench.ops, &mut bench.dir, &a, None, How { neg: false, parts_sign: 1 }, rows, parts).and_then(|()| Parts::sealed(&bench.dir, parts));
        bench.dir.release(rows);
        if ran.is_err() {
            bench.dir.release(parts);
        }
        ran
    }
}

/// The `i`th partition a sealed parts block lists, read where it lies.
fn nth(dir: &Directory, parts: &Parts, i: usize) -> Result<i64, String> {
    let mut v = None;
    let mut k = 0;
    parts.each(dir, &mut |p| {
        if k == i {
            v = Some(p);
        }
        k += 1;
    })?;
    v.ok_or_else(|| format!("a parts block's place {i}"))
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
    fn heads(&self) -> impl Iterator<Item = Option<BlockId>> + '_ {
        [self.tape].into_iter().chain(self.world).chain([self.blobs, self.want]).chain(self.asks.iter().copied()).chain(self.outs.iter().copied())
    }

    /// Kept: a frozen block, its words the pins and a reference to every head, where the directory keeps what lasts.
    /// Its id (a later one's is larger: the newest pins are the largest id's).
    pub fn keep(&self, dir: &mut Directory) -> Result<BlockId, String> {
        // Its words written where they lie: the magic, the atoms' count, the outputs' and requests' counts, each head.
        let words = [PINS_MAGIC, self.atoms.count as i64, self.outs.len() as i64, self.asks.len() as i64].into_iter().chain(self.heads().map(|h| h.map_or(-1, |b| i64::from(b.0))));
        let id = dir.make_placed(dir.lasting(), kind::FACTS, 8 * (HEAD + self.asks.len() + self.outs.len()), 0)?;
        {
            let b = dir.block(id).expect("just made");
            let mut w = rerail_blocks::region::pin_block(&b, "the run's pins", true)?;
            let bytes = w.bytes_mut().expect("a write pin");
            for (i, x) in words.enumerate() {
                rerail_blocks::gens::set_word::<i64>(bytes, BODY_AT + 8 * i, x);
            }
        }
        for h in self.heads().flatten() {
            dir.refer(id, h, BODY_AT)?;
        }
        dir.freeze(id)?;
        Ok(id)
    }

    /// The pins a kept run left in `dir`, and their block (the newest, the largest id, should a process have stopped
    /// between keeping one and letting the last go): `None` when none was kept. The atoms' block is the store's own,
    /// opened by its path: its count is the pins'.
    pub fn open(dir: &Directory) -> Result<Option<(BlockId, Pins)>, String> {
        let word = |b: &[u8], i: usize| b.get(BODY_AT + 8 * i..BODY_AT + 8 * i + 8).map(|_| rerail_blocks::gens::word::<i64>(b, BODY_AT + 8 * i));
        let mut best = None;
        for id in dir.unreferred() {
            let l = dir.read("the run's pins", &[id])?;
            let b = l.block(0);
            if word(b, 0) != Some(PINS_MAGIC) {
                continue;
            }
            let n = word(b, 2).unwrap_or(0) as usize + word(b, 3).unwrap_or(0) as usize;
            if word(b, HEAD + n - 1).is_none() {
                return Err("a kept pins block, short".to_string());
            }
            best = Some(id);
        }
        let Some(id) = best else { return Ok(None) };
        // Its words read where the block holds them.
        let l = dir.read("the run's pins", &[id])?;
        let b = l.block(0);
        let w = |i: usize| word(b, i).expect("a kept pins block's word");
        let head = |x: i64| (x >= 0).then_some(BlockId(x as u32));
        let (outs, asks) = (w(2) as usize, w(3) as usize);
        Ok(Some((id, Pins {
            tape: head(w(4)),
            world: [head(w(5)), head(w(6)), head(w(7))],
            blobs: head(w(8)),
            want: head(w(9)),
            asks: (HEAD..HEAD + asks).map(|i| head(w(i))).collect(),
            outs: (HEAD + asks..HEAD + asks + outs).map(|i| head(w(i))).collect(),
            atoms: Global { block: BlockId(u32::MAX), count: w(1) as u64 },
        })))
    }
}

/// B§8's run, its first two lines, under demand (P§7.1): the run's productions, l0's K-form (every relation of it, by
/// l0's key), are the want on L1; L1 asks of its tape port the documents it needs, and the world answers what is asked
/// (its load, parse and intern answers; the names it interns into `globals`), and lets go what is no longer asked. A
/// run is rounds: L1 stepped over the document set since the last round, then every key asked answered (the first
/// round's answers each kept request again, the world's cover; a later round's, those asked since), until a round
/// moves nothing. `last`: `None` is cold (every bistate from ∅, L1's traces emptied first); refused when not where the
/// last run left the traces. The run's pins, kept where the directory keeps what lasts; l0's answer; L1's work.
#[allow(clippy::too_many_arguments)]
pub fn run(l1: &L1, inst: &mut Instance, l0: &str, module: &str, globals: Atoms, docs: &mut DocSet, bench: &mut Bench, last: Option<&Pins>) -> Result<(Pins, Doc, Work), String> {
    match last {
        Some(p) if p.tape != docs.tape.head() || p.outs != inst.heads() || p.want != inst.want.head() || p.asks != inst.ask_heads() || p.atoms.count != u64::from(globals.count(&bench.dir)) => {
            return Err(format!("pins of another state: {p:?} (a run starts where the last run left its traces)"));
        }
        Some(_) => {}
        None => inst.clear(&mut bench.dir),
    }
    docs.world.begin();
    let mut leaves = docs.world.leaves_w(&mut bench.dir);
    let ran = (|| {
        // The blobs of slabs the residency retired or dropped since, forgotten (their documents load again).
        bench.blobs.forget_dead(&bench.ops, &mut bench.dir)?;
        // The run's productions: the want on L1's outputs. A request is answered by the spelling the run names it by.
        let d = docs.key(&bench.dir, globals, l0)?;
        let productions = crate::contract::AtKey { ports: RELATIONS.len(), key: d };
        let (wb, wa) = inst.set_want(&mut bench.cx(), &productions)?;
        let mut want = (wb, wa);
        let mut from = (last.and_then(|p| p.tape), last.map(|p| p.atoms.count));
        // Whether l0, the one document the run names, was answered in an earlier round.
        let (mut work, mut doc, mut answered) = (Work::default(), None, false);
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
            // Demand reaches the world as reads: what is asked answered, what is no longer asked let go. The asks and
            // the document set are read where their states lie.
            let asks = inst.asks_of(L1Ports::TAPE.index());
            let la = asks.map(|t| t.read(&bench.dir, &[t.head()])).transpose()?;
            let ta = la.as_ref().map(|l| l.table());
            let sa = match (asks, &ta) {
                (Some(t), Some(table)) => Some(t.state(table, t.head())?),
                _ => None,
            };
            let key = |s: &rerail_blocks::relation::View<'_>, id| Read::get(s, id, ColumnIndex::new(0));
            docs.forget_unasked(bench, asks)?;
            for k in sa.iter().flat_map(|s| s.live_ids().map(move |id| key(s, id))) {
                if k != d {
                    return Err(format!("a request for document {k}, which the run names no path for"));
                }
                if round > 0 && answered {
                    continue;
                }
                doc = Some(docs.read(bench, &mut leaves, globals, l0, module)?);
                answered = true;
            }
            if docs.tape.head() == from.0 && u64::from(globals.count(&bench.dir)) == count {
                break;
            }
        }
        // Its delta read, the run's before is no longer a layer the set needs: its layers rolled up (P§11.4's roll-up,
        // the document set's), so the next run's before is one head.
        docs.tape.roll_up_run(&bench.ops, &mut bench.dir, last.and_then(|p| p.tape), Tiling::WHOLE)?;
        inst.roll_up_want(&mut bench.cx(), last.and_then(|p| p.want))?;
        let doc = doc.ok_or("l0 was not asked")?;
        Ok::<_, String>((doc, work, from.1.unwrap_or(0)))
    })();
    // The leaves the run read, its layer of the world's leaves (D3: misses too).
    let committed = docs.world.commit_leaves(&bench.ops, &mut bench.dir, leaves);
    docs.world.end();
    let ran = committed.and(ran);
    // An execution ended: the residency drops everything `DROP_AFTER` from now, unless another run comes first.
    bench.dir.executed();
    let (doc, work, count) = ran?;
    let pins = Pins { tape: docs.tape.head(), outs: inst.heads(), atoms: Global { block: globals.0, count }, world: docs.world.heads(), blobs: bench.blobs.head(), want: inst.want.head(), asks: inst.ask_heads() };
    if bench.dir.lasting() == crate::dir::PLACE_KEPT {
        let id = pins.keep(&mut bench.dir)?;
        if let Some(old) = docs.kept.replace(id) {
            bench.dir.release(old);
        }
    }
    Ok((pins, doc, work))
}

/// l0's K-form by meaning after a cold run of it (B§8's first two lines, no pins) on `bench`, from a root of its own
/// under `base` holding l0: each relation's rows at its trace's head, in `RELATIONS`' order. What a device's run is
/// checked against the CPU's by.
#[cfg(any(test, feature = "test-hooks"))]
pub fn cold_k_form(bench: &mut Bench, base: &std::path::Path) -> Result<Vec<std::collections::BTreeSet<Vec<i64>>>, String> {
    const PATH: &str = "bootstrap-l0.rerail";
    let root = base.join("root");
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    // Written a minute ago: past the window in which a file's stat does not tell an edit.
    std::fs::write(root.join(PATH), crate::L0).map_err(|e| e.to_string())?;
    let f = std::fs::File::options().write(true).open(root.join(PATH)).map_err(|e| e.to_string())?;
    f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(60)).map_err(|e| e.to_string())?;
    let store = crate::files::Store::open(&base.join("store"))?;
    let atoms = Atoms::new(&mut bench.dir);
    let world = World::new(root, store, &bench.dir, [None; 3])?;
    let mut inst = Instance::new(&L1, &mut bench.dir)?;
    let mut docs = DocSet::new(world, &bench.dir, None, None)?;
    run(&L1, &mut inst, PATH, "bootstrap-l0", atoms, &mut docs, bench, None)?;
    inst.outs.iter().map(|t| crate::contract::oracle_rows(&bench.dir, t, t.head())).collect()
}

/// One run of a boot chain (`boot_chain`): the pins it started from and where it left every trace, its document, its
/// work, its launches (`Runner::launches`), the digest of L1's cache block, L1's K-form at its outputs' heads and the
/// plain function's K-form on the document set as the run left it (L1's reference, by the same atoms).
#[cfg(any(test, feature = "test-hooks"))]
pub struct Ran {
    pub before: Option<Pins>,
    pub after: Pins,
    pub doc: Doc,
    pub work: Work,
    /// The world's reads, loads, parses and interns in the run.
    pub asked: crate::files::Asked,
    pub launches: u64,
    pub cache: u64,
    pub kform: Vec<std::collections::BTreeSet<Vec<i64>>>,
    pub plain: Vec<std::collections::BTreeSet<Vec<i64>>>,
}

/// l0's boot run once per version of `versions`, on `bench`, each from the last run's pins (B§8): the file at a root of
/// its own under `base` is written again only when a version's text differs from the one before (then with another
/// modification time, older than the racy window), so an unchanged version is the same file with the same stat.
#[cfg(any(test, feature = "test-hooks"))]
pub fn boot_chain(bench: &mut Bench, base: &std::path::Path, versions: &[&str]) -> Result<Vec<Ran>, String> {
    boot_chain_touched(bench, base, versions, &[])
}

/// `boot_chain`, the file also written again before each run of `touched` (by its place in `versions`): the same bytes
/// under a new modification time.
#[cfg(any(test, feature = "test-hooks"))]
pub fn boot_chain_touched(bench: &mut Bench, base: &std::path::Path, versions: &[&str], touched: &[usize]) -> Result<Vec<Ran>, String> {
    const PATH: &str = "bootstrap-l0.rerail";
    let root = base.join("root");
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let store = crate::files::Store::open(&base.join("store"))?;
    let atoms = Atoms::new(&mut bench.dir);
    let world = World::new(root.clone(), store, &bench.dir, [None; 3])?;
    let mut inst = Instance::new(&L1, &mut bench.dir)?;
    let mut docs = DocSet::new(world, &bench.dir, None, None)?;
    let (mut last, mut text): (Option<Pins>, Option<&str>) = (None, None);
    let mut out = Vec::with_capacity(versions.len()); // copy: not block data (the chain's runs)
    for (k, &v) in versions.iter().enumerate() {
        if text != Some(v) || touched.contains(&k) {
            std::fs::write(root.join(PATH), v).map_err(|e| e.to_string())?;
            let f = std::fs::File::options().write(true).open(root.join(PATH)).map_err(|e| e.to_string())?;
            f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(600 - 10 * k as u64)).map_err(|e| e.to_string())?;
            text = Some(v);
        }
        let (launched, a0) = (bench.ops.launches(), docs.world.asked);
        let (pins, doc, work) = run(&L1, &mut inst, PATH, "bootstrap-l0", atoms, &mut docs, bench, last.as_ref())?;
        let launches = bench.ops.launches() - launched;
        let a = docs.world.asked;
        let asked = crate::files::Asked { reads: a.reads - a0.reads, loads: a.loads - a0.loads, parses: a.parses - a0.parses, interns: a.interns - a0.interns };
        let cache = rerail_blocks::maps::hash_bytes(bench.dir.read("L1's cache", &[inst.cache.0])?.block(0));
        let kform = inst.outs.iter().map(|t| crate::contract::oracle_rows(&bench.dir, t, t.head())).collect::<Result<_, _>>()?;
        let globals = [Global { block: atoms.0, count: u64::from(atoms.count(&bench.dir)) }];
        let plain = crate::contract::apply(&L1, &mut bench.cx(), &[(&docs.tape, docs.tape.head())], &globals)?;
        out.push(Ran { before: last.replace(pins.clone()), after: pins, doc, work, asked, launches, cache, kform, plain });
    }
    Ok(out)
}

/// l0's versions for the boot chain: as shipped, the same again, comments only (above the first rule, between two
/// rules, inside a body, after the last), and the comments' version with one rule added after the last.
#[cfg(any(test, feature = "test-hooks"))]
pub fn chain_versions() -> [String; 4] {
    let first = "kind(d, c, \"clause\"), ";
    let second = "k_arg(d, c, -1, -1, i, a) :-";
    assert!(crate::L0.contains(first) && crate::L0.contains(second));
    let comments = format!("// more comments: above the rules,\n{}// and after the last.\n", crate::L0.replacen(first, "kind(d, c, \"clause\"), // inside a body\n    ", 1).replacen(second, &format!("// between two rules\n{second}"), 1));
    let added = format!("{comments}b6_added(c, h) :- k_rule(_, c, h), !k_wild(_, c).\n");
    [crate::L0.to_string(), crate::L0.to_string(), comments, added]
}

/// What a run of the boot chain must show (0686: Shape and Invariants, §3), whatever runs its launches: cold, ∅ → l0's
/// 14 rules (L2); unchanged, every trace where it was, the cache unchanged, no read and no launch (L4); comments only,
/// a new source and tape and the same program by value (what leaves a module is judged by value, L9); a rule added,
/// by value exactly that rule's rows added and none gone; and every run's K-form the plain function's on the document
/// set as the run left it (L1, L3).
#[cfg(any(test, feature = "test-hooks"))]
pub fn check_boot_chain(runs: &[Ran], atoms: impl Fn(&str) -> Option<i64>) {
    use crate::kform::by_value;
    let [cold, same, comments, added] = runs else { panic!("four runs") };
    for (k, r) in runs.iter().enumerate() {
        assert_eq!(r.kform, r.plain, "run {k}: L1's K-form is the plain function's on the document set");
    }
    // Cold.
    assert!(cold.before.is_none() && !cold.doc.is_identity());
    assert_eq!(cold.kform[0].len(), 14, "l0's 14 rules");
    // Unchanged: the identity everywhere.
    assert!(same.doc.is_identity());
    assert_eq!(same.before.as_ref(), Some(&same.after), "an unchanged run leaves every trace where it was");
    assert!(same.work.is_checks_only(), "{:?}", same.work);
    assert_eq!(same.launches, 1, "an identity run's launches are its checks: l0's stat (answer_load, D2)");
    assert_eq!((same.cache, &same.kform), (cold.cache, &cold.kform), "the cache and the K-form unchanged");
    // Comments only.
    let (b, a) = (comments.doc.before.expect("a document before"), comments.doc.after);
    assert!(b.source != a.source && b.tape != a.tape, "a new source and a new tape");
    assert_eq!(by_value(&comments.kform), by_value(&same.kform), "comments only: the same program by value");
    // A rule added: by value, its rows alone.
    let (was, now) = (by_value(&comments.kform), by_value(&added.kform));
    let gone: usize = was.iter().zip(&now).map(|(w, n)| w.difference(n).count()).sum();
    assert_eq!(gone, 0, "a rule added takes no row away");
    let new: Vec<Vec<Vec<i64>>> = was.iter().zip(&now).map(|(w, n)| n.difference(w).cloned().collect()).collect();
    let head = atoms("b6_added").expect("the new rule's head interned");
    assert_eq!(new[0].len(), 1, "one rule added: {:?}", new[0]);
    let (m, rule) = (new[0][0][0], new[0][0][1]);
    assert_eq!(new[0][0][2], head, "the new rule writes b6_added");
    let terms: std::collections::BTreeSet<i64> = new[2].iter().map(|r| r[5]).collect();
    assert!(new[1].iter().chain(&new[2]).all(|r| r[0] == m && r[1] == rule), "every added step and argument is the new rule's");
    assert!(new[3..7].iter().flatten().all(|r| r[0] == m && terms.contains(&r[1])), "every added term is one of its arguments");
    assert_eq!((new[1].len(), new[2].len(), new[3].len(), new[6].len()), (2, 7, 5, 2), "two steps, seven arguments (two in the head), five variables, two wildcards");
    assert!(new[7].is_empty(), "the module's stratum unchanged");
}

/// l0 with a rule added, then that rule's neighbour changed, then a rule removed: the edits the harness's chains
/// thread through L1.
#[cfg(any(test, feature = "test-hooks"))]
pub fn chain_edits() -> [String; 3] {
    let l0 = crate::L0;
    let added = format!("{l0}\nb6_added(c, h) :- k_rule(_, c, h), !k_wild(_, c).\n");
    let changed = added.replace("k_wild(d, t) :- kind(d, t, \"wildcard\").", "k_wild(d, t) :- kind(d, t, \"wildcard\"), node(d, t, _, _, _).");
    let line = changed.lines().find(|l| l.starts_with("k_num(")).expect("a k_num rule").to_string();
    let removed = changed.replace(&format!("{line}\n"), "");
    assert!(added != l0 && changed != added && removed != changed);
    [added, changed, removed]
}

/// The harness's laws (L1 to L5, and D1 under `wants`) for L1 over a chain of edits of l0 on disk, on `bench`: the file
/// layer reads each version into the document set (an unchanged one twice, the identity), and `contract::harness_on`
/// threads the set's states through L1, its cache dropped at a step each seed of `seeds` picks. Under `wants`, the want
/// cycles through all of l0's relations, two of them and none.
#[cfg(any(test, feature = "test-hooks"))]
pub fn harness_over_edits(bench: &mut Bench, base: &std::path::Path, wants: bool, seeds: std::ops::RangeInclusive<u64>) -> Result<(), String> {
    use std::collections::BTreeSet;
    const PATH: &str = "bootstrap-l0.rerail";
    let root = base.join("root");
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let store = crate::files::Store::open(&base.join("store"))?;
    let atoms = Atoms::new(&mut bench.dir);
    let world = World::new(root.clone(), store, &bench.dir, [None; 3])?;
    let mut docs = DocSet::new(world, &bench.dir, None, None)?;
    let [added, changed, removed] = chain_edits();
    let l0 = crate::L0.to_string();
    // Each edit's stat differs from the one before (its size, its modification time); `None` reads the file again.
    let chain: Vec<Option<&String>> = if wants {
        vec![Some(&l0), None, Some(&added), None, Some(&changed), None, Some(&removed), Some(&l0)]
    } else {
        vec![Some(&l0), None, Some(&added), Some(&changed), None, Some(&removed), Some(&l0)]
    };
    let count = |dir: &Directory| Global { block: atoms.0, count: u64::from(atoms.count(dir)) };
    let (mut heads, mut globals) = (vec![vec![None]], vec![vec![count(&bench.dir)]]); // copy: not block data (the chain's states)
    for (k, e) in chain.into_iter().enumerate() {
        if let Some(text) = e {
            std::fs::write(root.join(PATH), text).map_err(|e| e.to_string())?;
            let f = std::fs::File::options().write(true).open(root.join(PATH)).map_err(|e| e.to_string())?;
            f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(600 - 10 * k as u64)).map_err(|e| e.to_string())?;
        }
        let mut leaves = docs.world.leaves_w(&mut bench.dir);
        docs.read(bench, &mut leaves, atoms, PATH, "bootstrap-l0")?;
        docs.world.commit_leaves(&bench.ops, &mut bench.dir, leaves)?;
        heads.push(vec![docs.tape.head()]);
        globals.push(vec![count(&bench.dir)]);
    }
    if heads[1] != heads[2] {
        return Err("an unchanged file is the identity: the set's head moved".into());
    }
    let wanted = if wants {
        let d = docs.key(&bench.dir, atoms, PATH)?;
        let all: BTreeSet<(usize, i64)> = (0..RELATIONS.len()).map(|k| (k, d)).collect();
        let cycle = [all.clone(), BTreeSet::from([(1, d), (3, d)]), BTreeSet::new(), BTreeSet::new(), all];
        Some((0..heads.len()).map(|k| cycle[k % cycle.len()].clone()).collect::<Vec<_>>())
    } else {
        None
    };
    for seed in seeds {
        let drop_at = 1 + (rerail_blocks::layer::mix(seed) % (heads.len() as u64 - 1)) as usize;
        crate::contract::harness_on(&L1, bench, std::slice::from_mut(&mut docs.tape), &heads, &globals, drop_at, wanted.as_deref()).map_err(|b| format!("seed {seed}: {b:?}"))?;
    }
    Ok(())
}
