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
use crate::drop::{DropOps, Parts, drop_parts};
use crate::files::{Answer, Answerer, Doc, DocIds, Intern, Load, Load1, LeavesW, Parse, Reach, World, key_of, probe};
use crate::intern::InternOps;
use crate::kform::RELATIONS;
use crate::contract::asks_shape;
use crate::l1::{L1, L1Ports, TAPE_ARITY, TAPE_PATH, row_blobs, tape_row, tape_shape};
use crate::semi::{self, How, SemiOps, Side};
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
    pub fn forget_unasked(&mut self, bench: &mut Bench, semis: &SemiOps, asks: Option<&Trace>) -> Result<(), String> {
        let Some(h) = self.tape.head() else { return Ok(()) };
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
        let chosen = semi::select(&bench.reg, semis, &mut bench.dir, &a, b.as_ref().map(|b| (b, keys)), How::UNMATCHED, rows, parts).and_then(|()| Parts::sealed(&bench.dir, parts));
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
            if let Err(e) = drop_parts(&bench.reg, &self.drops, &mut bench.dir, h, parts, -1, runs) {
                bench.dir.release(runs);
                return Err(e);
            }
            t.commit(&bench.reg, &bench.traces, &mut bench.dir, runs, Tiling::WHOLE)?;
        }
        Ok(())
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
        let (s, t) = (doc.after.source.words(), doc.after.tape.words());
        let key = [d, s[0], s[1], s[2], s[3], t[0], t[1], t[2], t[3]];
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
                let parts = Parts::write(&mut bench.dir, 1, |put| put(d))?;
                let n = drop_parts(&bench.reg, &self.drops, &mut bench.dir, h, &parts, -1, runs);
                parts.release(&mut bench.dir);
                n?;
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
        let head = self.tape.head();
        let l = self.tape.read(&bench.dir, &[head])?;
        let table = l.table();
        let s = self.tape.state(&table, head)?;
        for id in s.live_ids() {
            let r = Row::of(&s, id);
            let (ids, syms) = row_blobs(&r);
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
        bench.blobs.forget_dead(&bench.reg, &mut bench.dir)?;
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
            docs.forget_unasked(bench, &l1.semis, asks)?;
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
        docs.tape.roll_up_run(&bench.reg, &bench.traces, &mut bench.dir, last.and_then(|p| p.tape), Tiling::WHOLE)?;
        inst.roll_up_want(&mut bench.cx(), last.and_then(|p| p.want))?;
        let doc = doc.ok_or("l0 was not asked")?;
        Ok::<_, String>((doc, work, from.1.unwrap_or(0)))
    })();
    // The leaves the run read, its layer of the world's leaves (D3: misses too).
    let committed = docs.world.commit_leaves(&bench.reg, &bench.traces, &mut bench.dir, leaves);
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
