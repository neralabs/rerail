//! Δanswer as operations (*0686: Operations* §10): `answer_load` → `parse` → `intern`, each an operation with its
//! slots, run by the executor like the others.
//!
//! - **`answer_load`** (host): read plan (the load δ: the paths asked, each with its key and the source it was last
//!   answered with) · rw tapes (a residency slab), control · write facts (the doc δ). Δanswer's World answerer, which is
//!   the engine's file layer as it is (§5): the observer's digest step under the racy rule and the journal, and the
//!   placed read (`read_placed`) into a room of the slab. A path whose digest did not change gives no δ.
//! - **`parse`** (host now): rw tapes, control; a unit a codec (shape 22), a document an item. The grammar's parser,
//!   for a source the store has no tape for: the tape written into a room of the slab the source lies in. An operation
//!   like the others, so it moves to the GPU by registering a GPU version, checked against this one.
//! - **`intern`** is `crate::intern`.
//!
//! Their versions are host versions (`op::Impl::Host`): they call the observer and the parser, which the twins may not.
//! The host loop around them (`files::World::answer`) is thin: it lays out the plan, gives the slab, and reads the
//! facts.

use crate::dir::{BlockId, Directory};
use crate::op::{Across, Bound, Found, Independence, Inside, Output, Over, ParamsLayout, Phase, Registry, Runner, Signature, Tiling, Unit, Version};
use rerail_blocks::block::{Block, kind};
use rerail_blocks::gens::{BODY_AT, Table};
use rerail_blocks::maps::InBlock;
use rerail_blocks::maps::Placed as _;
use rerail_blocks::region::pin_block;
use rerail_delta_twins::{Ctx, NoParams, Tile, TwinPhase};
use rerail_observe::content_store::observe::{self, DigestRead, DigestStep};
use rerail_rr_syntax::tape_format::home;
use std::path::PathBuf;
use std::sync::Arc;

// ---- answer_load ------------------------------------------------------------------------------------------------

/// `answer_load`'s slots.
const PLAN: usize = 0;
const TAPES: usize = 1;
const CONTROL: usize = 2;
const FACTS: usize = 3;

/// A path an item: its digest step, and its read placed in a room of the slab (host work, the observer's one lock).
static LOAD_PHASE: [Phase; 1] = [Phase { name: "load", found: Found::Fixed, unit: Unit::Partition, inside: Inside::Nothing, across: Across::Nothing, output: Output::ByIndex, bounds: &[] }];

pub static ANSWER_LOAD: Signature = Signature::of(
    &crate::op::interfaces::ANSWER_LOAD,
    ParamsLayout::of::<NoParams>(),
    &LOAD_PHASE,
    Independence::Not("one observer: its answers under one lock"),
    load_facts,
);

static LOAD: [TwinPhase; 1] = [TwinPhase { name: "load", items: plan_items, run: load, again: None }];

/// An item's doc δ fact (`FACT_WORDS` words): `(d, code, found, digest's four words, source's offset, its length,
/// need)`.
pub const FACT_WORDS: usize = 10;

/// What `answer_load` found for a path.
pub mod code {
    /// Its digest is the one it was last answered with: no δ.
    pub const SAME: i64 = 0;
    /// Read, and placed in a room of the slab: its source at `(at, len)`.
    pub const READ: i64 = 1;
    /// Answered by its stat (unread), with a digest it was not last answered with: its content is asked by digest.
    pub const KNOWN: i64 = 2;
    /// No readable file there: a miss (recorded, D3).
    pub const MISS: i64 = 3;
    /// Its source needs `need` bytes of room the slab has not: the host gives a slab with room, and asks again.
    pub const NO_ROOM: i64 = 4;
    /// It could not be read (it grew while read, or went).
    pub const UNREAD: i64 = 5;
}

/// The plan's item flag: read the file, whatever the observer answers (its digest is known, but no home holds the
/// content it names).
pub const READ_ANYWAY: i64 = 1;

/// One path of the load δ.
#[derive(Clone, Debug)]
pub struct Ask {
    pub d: i64,
    pub path: String,
    /// The source's digest it was last answered with.
    pub before: Option<[u8; 32]>,
    pub flags: i64,
}

/// The load plan, a control block: `[n, root (at, len), observer (at, len)]`, then an item a path, `[d, before's four
/// words, has before, path (at, len), flags]`, then the strings; offsets from the body's start. Written where it lies.
fn plan_block(dir: &mut Directory, root: &std::path::Path, observer: &std::path::Path, asks: &[Ask]) -> Result<BlockId, String> {
    const HEAD: usize = 5;
    const ITEM: usize = 9;
    let (root, observer) = (root.to_string_lossy(), observer.to_string_lossy());
    let base = 8 * (HEAD + ITEM * asks.len());
    let len = base + root.len() + observer.len() + asks.iter().map(|a| a.path.len()).sum::<usize>();
    control_with(dir, len, |bytes| {
        let mut at = base;
        let mut put = |bytes: &mut [u8], s: &str| {
            bytes[BODY_AT + at..BODY_AT + at + s.len()].copy_from_slice(s.as_bytes()); // copy: derived (a path, written once into the plan)
            at += s.len();
            ((at - s.len()) as i64, s.len() as i64)
        };
        put_word(bytes, 0, asks.len() as i64);
        let (r, o) = (put(bytes, &root), put(bytes, &observer));
        for (k, v) in [r.0, r.1, o.0, o.1].into_iter().enumerate() {
            put_word(bytes, 1 + k, v);
        }
        for (k, a) in asks.iter().enumerate() {
            let item = HEAD + ITEM * k;
            let before = a.before.map_or([0; 4], |g| rerail_blocks::blob::BlobId(g).words());
            let (pat, plen) = put(bytes, &a.path);
            for (i, v) in [a.d, before[0], before[1], before[2], before[3], i64::from(a.before.is_some()), pat, plen, a.flags].into_iter().enumerate() {
                put_word(bytes, item + i, v);
            }
        }
    })
}

fn word(b: &[u8], i: usize) -> i64 {
    i64::from_le_bytes(b[BODY_AT + 8 * i..BODY_AT + 8 * i + 8].try_into().expect("a word"))
}

fn put_word(b: &mut [u8], i: usize, v: i64) {
    b[BODY_AT + 8 * i..BODY_AT + 8 * i + 8].copy_from_slice(&v.to_le_bytes()); // copy: derived (a fact's word)
}

fn text(b: &[u8], at: i64, len: i64) -> &str {
    std::str::from_utf8(&b[BODY_AT + at as usize..BODY_AT + (at + len) as usize]).expect("a plan's path is UTF-8")
}

fn plan_items(c: &Ctx<'_, '_, '_>) -> u32 {
    word(c.read(PLAN), 0) as u32
}

/// A plan item, read where it lies: its key, path (absolute), the digest it was last answered with, and its flags;
/// and the observer's store.
fn ask(plan: &[u8], k: usize) -> (i64, PathBuf, Option<[u8; 32]>, i64, PathBuf) {
    let at = 5 + 9 * k;
    let root = PathBuf::from(text(plan, word(plan, 1), word(plan, 2)));
    let observer = PathBuf::from(text(plan, word(plan, 3), word(plan, 4)));
    let before = (word(plan, at + 5) != 0).then(|| rerail_blocks::blob::BlobId::of_words(&(1..5).map(|i| word(plan, at + i)).collect::<Vec<_>>()).0);
    (word(plan, at), root.join(text(plan, word(plan, at + 6), word(plan, at + 7))), before, word(plan, at + 8), observer)
}

/// Each path of the tile through the file layer as it is: the observer's digest step; a changed file read once,
/// placed in a room of the slab (`read_placed`), its digest recorded by the observer; the fact written.
fn load(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    for k in t.items() {
        let (d, abs, before, flags, observer) = ask(c.read(PLAN), k);
        let slab = c.pin_mut(TAPES).block().clone();
        let promised = word(c.read(CONTROL), 1) as usize;
        let f = load_one(&slab, promised, &abs, before, flags, &observer);
        let out = c.pin_mut(FACTS).bytes_mut().expect("a write pin");
        let fact = [d, f.code, i64::from(f.digest.is_some()), 0, 0, 0, 0, f.at, f.len, f.need];
        for (i, &w) in fact.iter().enumerate() {
            put_word(out, FACT_WORDS * k + i, w);
        }
        if let Some(g) = f.digest {
            for (i, &w) in rerail_blocks::blob::BlobId(g).words().iter().enumerate() {
                put_word(out, FACT_WORDS * k + 3 + i, w);
            }
        }
        if f.code == code::READ {
            // Read once; and the room its tape needs promised, so a later source placed leaves it free.
            let ctl = c.pin_mut(CONTROL).bytes_mut().expect("a write pin");
            put_word(ctl, 0, word(ctl, 0) + 1);
            put_word(ctl, 1, word(ctl, 1) + (room(f.len as usize) - f.len as usize) as i64);
        }
    }
}

struct Found1 {
    code: i64,
    digest: Option<[u8; 32]>,
    at: i64,
    len: i64,
    need: i64,
}

fn load_one(slab: &Arc<Block>, promised: usize, abs: &std::path::Path, before: Option<[u8; 32]>, flags: i64, observer: &std::path::Path) -> Found1 {
    let found = |code, digest| Found1 { code, digest, at: 0, len: 0, need: 0 };
    let step = observe::with(Some(observer), |o| o.digest_step(abs));
    let read = match step {
        DigestStep::Answered(None) => return found(code::MISS, None),
        DigestStep::Answered(Some(g)) if before == Some(g) => return found(code::SAME, Some(g)),
        DigestStep::Answered(Some(g)) if flags & READ_ANYWAY == 0 => return found(code::KNOWN, Some(g)),
        DigestStep::Answered(Some(_)) => None,
        DigestStep::Read(r) => Some(r),
    };
    // The room a document needs (its source, and its tape at the parser's bound), checked when its length is known.
    let need = std::sync::atomic::AtomicUsize::new(0);
    let place = |len: usize, fill: &mut dyn FnMut(&mut [u8]) -> std::io::Result<()>| {
        if slab.reserved().saturating_sub(slab.used() + promised) < room(len) {
            need.store(room(len), std::sync::atomic::Ordering::Relaxed);
            return Err(std::io::Error::other("no room in the slab"));
        }
        crate::files::place_in(slab, len, fill)
    };
    let placed = match read {
        Some(r) => match r.read_placed(&place) {
            Some(home) => {
                let hashed = DigestRead::hashed(&home);
                observe::with(Some(observer), |o| o.digest_read(r, Some(hashed))).map(|g| (g, home))
            }
            None => None,
        },
        None => rerail_observe::git_workspace::snapshot::read_placed(abs, &place).ok().map(|home| (DigestRead::hashed(&home).1, home)),
    };
    let need = need.into_inner();
    if need > 0 {
        return Found1 { need: need as i64, ..found(code::NO_ROOM, None) };
    }
    let Some((g, source)) = placed else { return found(code::UNREAD, None) };
    let Some((at, len)) = crate::files::room_at(&source) else { return found(code::UNREAD, None) };
    let len = len as i64;
    // The room stays where it lies, written once: the slab is sealed after the launch.
    Found1 { code: code::READ, digest: Some(g), at: at as i64, len, need: 0 }
}

/// The room a document of a source of `len` bytes needs in its slab (`files::room`).
pub fn room(len: usize) -> usize {
    crate::files::room(len)
}

/// The doc δ facts by meaning: a row an item.
fn load_facts(_: &Table<'_>, slot: usize, bytes: &[u8]) -> Vec<Vec<i64>> {
    match slot {
        FACTS => {
            let n = (bytes.len().saturating_sub(BODY_AT)) / (8 * FACT_WORDS);
            (0..n).map(|k| (0..FACT_WORDS).map(|i| word(bytes, FACT_WORDS * k + i)).collect()).collect()
        }
        _ => vec![vec![word(bytes, 0)]],
    }
}

// ---- parse ------------------------------------------------------------------------------------------------------

/// `parse`'s slots.
const P_TAPES: usize = 0;
const P_CONTROL: usize = 1;

/// A unit a codec (shape 22: a writer's calls replayed into its region), a document an item: its source parsed, its
/// tape written into a room it claims in the slab, with the parser's stacks.
static PARSE_PHASE: [Phase; 1] = [Phase {
    name: "parse",
    found: Found::Fixed,
    unit: Unit::Partition,
    inside: Inside::Stacks,
    across: Across::Nothing,
    output: Output::Claim,
    // The room a tape needs was promised when its source was placed (`answer_load`): past it, refused.
    bounds: &[Bound::StackDepth(MAX_DEPTH), Bound::Capacity(Over::Refuse)],
}];

/// The deepest the parser's stack of open nodes may go: a source nested deeper is refused (a rule's terms, each a node,
/// nest far less).
pub const MAX_DEPTH: u32 = 1 << 12;

pub static PARSE: Signature = Signature::of(
    &crate::op::interfaces::PARSE,
    ParamsLayout::of::<NoParams>(),
    &PARSE_PHASE,
    Independence::Along("documents: each claims its own room in the slab"),
    parse_facts,
);

static PARSER: [TwinPhase; 1] = [TwinPhase { name: "parse", items: parse_items, run: parse, again: None }];

/// A parse item's words in control, after its count: `(source's offset, its length, tape's offset, its length, code)`.
pub const PARSE_WORDS: usize = 5;

/// Why a source was not parsed: its bytes are not UTF-8, or the parser refused it.
pub const NOT_UTF8: i64 = 1;
pub const REFUSED: i64 = 2;

/// The control block for parsing the sources at `sources` (each `(offset, length)` in the slab), written where it
/// lies: their count, then each one's place, its tape's place and its code, none yet.
pub fn parse_control(dir: &mut Directory, sources: &[(usize, usize)]) -> Result<BlockId, String> {
    control_with(dir, 8 * (1 + PARSE_WORDS * sources.len()), |b| {
        put_word(b, 0, sources.len() as i64);
        for (k, &(at, len)) in sources.iter().enumerate() {
            for (i, v) in [at as i64, len as i64, 0, 0, 0].into_iter().enumerate() {
                put_word(b, 1 + PARSE_WORDS * k + i, v);
            }
        }
    })
}

fn parse_items(c: &Ctx<'_, '_, '_>) -> u32 {
    word(c.read(P_CONTROL), 0) as u32
}

/// Each source of the tile parsed where it lies, its tape into a room the parser claims in the same slab (the room its
/// source's load promised), its stack's depth metered.
fn parse(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    for k in t.items() {
        let (at, len) = {
            let ctl = c.read(P_CONTROL);
            (word(ctl, 1 + PARSE_WORDS * k) as usize, word(ctl, 2 + PARSE_WORDS * k) as usize)
        };
        let (tape_at, tape_len, code) = if c.room(P_TAPES, room(len) - len) { parse_one(c, at, len) } else { (0, 0, REFUSED) };
        let ctl = c.pin_mut(P_CONTROL).bytes_mut().expect("a write pin");
        put_word(ctl, 3 + PARSE_WORDS * k, tape_at);
        put_word(ctl, 4 + PARSE_WORDS * k, tape_len);
        put_word(ctl, 5 + PARSE_WORDS * k, code);
    }
}

/// The source at `(at, len)` of the slab parsed, its tape claimed in the slab: `(tape's offset, its length, code)`.
fn parse_one(c: &mut Ctx<'_, '_, '_>, at: usize, len: usize) -> (i64, i64, i64) {
    let pin = c.pin_mut(P_TAPES);
    let slab = pin.block().clone();
    let Ok(src) = std::str::from_utf8(&pin.bytes()[at..at + len]) else { return (0, 0, NOT_UTF8) };
    let Ok((tape, deepest)) = rerail_rr_syntax::tape_in_deep(src, Arc::new(InBlock(slab.clone()))) else { return (0, 0, REFUSED) };
    if !c.depth(deepest) {
        return (0, 0, REFUSED);
    }
    match tape.home() {
        home::Home::Own(v) => match v.place() {
            Some((b, tape_at)) if Arc::ptr_eq(b, &slab) => (tape_at as i64, tape.as_bytes().len() as i64, 0),
            _ => (0, 0, REFUSED),
        },
        home::Home::Placed(_) => (0, 0, REFUSED),
    }
}

fn parse_facts(_: &Table<'_>, slot: usize, bytes: &[u8]) -> Vec<Vec<i64>> {
    match slot {
        P_CONTROL => {
            let n = word(bytes, 0) as usize;
            (0..n).map(|k| (0..PARSE_WORDS).map(|i| word(bytes, 1 + PARSE_WORDS * k + i)).collect()).collect()
        }
        _ => Vec::new(),
    }
}

// ---- the operations, declared; and their launches -----------------------------------------------------------------

/// The module's operations declared, each from its UKL interface, and their twins registered as references: the
/// runner's registry (`Runner::new`), the one place they are registered.
pub(crate) fn register(reg: &mut Registry) -> Result<(), String> {
    let load = reg.declare(&ANSWER_LOAD)?;
    reg.register_reference(Version::host(load, "answer_load on the host (the file layer)", &LOAD, &LOAD_PHASE))?;
    let parse = reg.declare(&PARSE)?;
    reg.register_reference(Version::host(parse, "parse on the host (the rr parser)", &PARSER, &PARSE_PHASE))?;
    Ok(())
}

/// A control block of `bytes` body bytes, its words written by `fill` where they lie.
fn control_with(dir: &mut Directory, bytes: usize, fill: impl FnOnce(&mut [u8])) -> Result<BlockId, String> {
    let id = dir.make(kind::CONTROL, bytes.max(8));
    let b = dir.block(id).expect("just made");
    match pin_block(&b, "a launch's control", true) {
        Ok(mut w) => {
            fill(w.bytes_mut().expect("a write pin"));
            Ok(id)
        }
        Err(e) => {
            drop(b);
            dir.release(id);
            Err(e)
        }
    }
}

/// `answer_load` over `asks`, its rooms in slab `slab`: each ask's fact, and how many files it read.
pub fn answer_load(ops: &Runner, dir: &mut Directory, slab: BlockId, root: &std::path::Path, observer: &std::path::Path, asks: &[Ask]) -> Result<(Vec<[i64; FACT_WORDS]>, u64), String> {
    let plan = plan_block(dir, root, observer, asks)?;
    let control = control_with(dir, 16, |b| (0..2).for_each(|i| put_word(b, i, 0)))?;
    let facts = dir.make(kind::FACTS, 8 * FACT_WORDS * asks.len().max(1));
    let ran = ops.run(dir, &ANSWER_LOAD, &[plan, slab, control, facts], &[], Tiling::WHOLE).and_then(|()| {
        let l = dir.read("answer_load's facts", &[facts, control])?;
        let (f, c) = (l.block(0), l.block(1));
        let rows = (0..asks.len()).map(|k| std::array::from_fn(|i| word(f, FACT_WORDS * k + i))).collect();
        Ok((rows, word(c, 0) as u64))
    });
    for b in [plan, control, facts] {
        dir.release(b);
    }
    ran
}

/// A source's tape `(offset, length)` in its slab, or why it was refused (`NOT_UTF8`, `REFUSED`).
pub type Parsed = Result<(usize, usize), i64>;

/// `parse` over the sources at `sources` in slab `slab`: each one's tape, or its refusal.
pub fn parse_sources(ops: &Runner, dir: &mut Directory, slab: BlockId, sources: &[(usize, usize)]) -> Result<Vec<Parsed>, String> {
    let control = parse_control(dir, sources)?;
    let ran = ops.run(dir, &PARSE, &[slab, control], &[], Tiling::WHOLE).and_then(|()| {
        let l = dir.read("parse's places", &[control])?;
        let c = l.block(0);
        Ok((0..sources.len())
            .map(|k| {
                let (at, len, code) = (word(c, 3 + PARSE_WORDS * k), word(c, 4 + PARSE_WORDS * k), word(c, 5 + PARSE_WORDS * k));
                if code == 0 { Ok((at as usize, len as usize)) } else { Err(code) }
            })
            .collect())
    });
    dir.release(control);
    ran
}

#[cfg(test)]
#[path = "answer_tests.rs"]
mod tests;
