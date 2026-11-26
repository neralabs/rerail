//! A document through the engine's file layer (decision 0686, B4; *0686: Operations* §5, §9; P§12 item 1; B§4's
//! tape port, B§7).
//!
//! A document is named by its path relative to the root, and answered as the engine answers a path today:
//!
//! - the observer's digest step (`Observer::digest_step`) reuses the answer it holds while the file's stat holds,
//!   under the racy rule; no byte is read then;
//! - a file it must read is read once, by the observer's placed read (`DigestRead::read_placed`), into a room of a
//!   slab (`place_in`), and its digest recorded (`Observer::digest_read`); the slabs are the residency's, as the
//!   engine places a read (`rerail_slabs::placed`): blocks many documents share, each source and tape a room written
//!   once (`Directory::slab_room`), host memory or a block the device reads where the run is bound to one;
//! - the rr parser writes the tape into a room of the same slab (`rerail_rr_syntax::tape_in`, `InBlock`), where it
//!   lies; the rooms are then sealed: the document's source and tape, which an operation reads as a slot (`intern`,
//!   `k0_pass`) or through a reference to it;
//! - the source and the tape are each a blob (P§11.4: "source text, tapes: blobs, by content"), housed from their
//!   placed bytes (`Blobs::house`, P§12 item 1: `house(bytes: Placed) -> BlobId`, "takes bytes where they lie"), as
//!   the store houses objects: a row a blob, by its name, naming where its bytes lie, in a trace probed by name
//!   (`Blobs::get`); the bytes read where they lie, never
//!   versioned: changed bytes are a new blob. Each is its own blob, so a new source whose tape is the same bytes as
//!   an earlier document's (the tape holds the shape, not the text) has its tape where that one's lies: a document's
//!   source and tape may lie in two blocks, and every reader takes the two places (`reads`).
//!
//! The tape port's bistate (`Doc`) is ∅ → tape on the first answer, the identity while the digest is unchanged, and
//! the old tape → the new one when it changed: the file is the grain. A file that does not parse, is not UTF-8, or
//! cannot be read is refused, naming its path. Nothing here reads a file itself.

use crate::contract::{push_layer, rows_at};
use crate::dir::{BlockId, Directory};
use crate::answer::{AnswerOps, Ask, READ_ANYWAY, answer_load, code, parse_sources};
use crate::op::Registry;
use crate::trace::{Trace, TraceOps};
use rerail_blocks::bistate::{ColumnIndex, Read};
use rerail_blocks::layer::{ColType, Runs, Shape};
use rerail_blocks::block::kind;
use rerail_observe::content_store::observe;
use rerail_store::content_store::ContentStore;
use rerail_store::content_store::DocumentPlaces;
use rerail_store_base::content_store::HashKey;
use rerail_blocks::region::{Frozen, Mutable};
use rerail_rr_syntax::tape_format::home;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use rerail_blocks::blob::{BlobId, Placed};

/// A home of blobs (P§12 item 1's `Blobs`, the trait in the blocks crate), housed as the store houses objects: a row a blob, named by its bytes' sha256,
/// that says where they lie (as an object's row names its segment, offset and length: `object_segments`), and the
/// bytes read in place there, in a frozen block. The rows are a trace, `(name's four words, block, at, len)`, a layer
/// a housing; a blob is found by a probe of its name at the trace's head.
pub struct BlobRows {
    ops: TraceOps,
    rows: Trace,
}

/// The blobs' rows: `(name's four words, block, at, len)`, in that order.
pub fn blobs_shape() -> Shape {
    let c = vec![ColType::I64, ColType::I64, ColType::I64, ColType::I64, ColType::I32, ColType::I64, ColType::I64];
    Shape::run(c, (0..7).collect(), None)
}

impl BlobRows {
    /// No blob, its rows housed by the trace's operations `ops`.
    pub fn new(ops: &TraceOps) -> BlobRows {
        BlobRows { ops: *ops, rows: Trace::new(&blobs_shape()).expect("a storable shape") }
    }

    /// The blobs' rows placed at `place` (`dir::PLACE_KEPT`: they last across processes), at `head`.
    pub fn resumed(self, place: u32, head: Option<BlockId>) -> BlobRows {
        BlobRows { rows: self.rows.resumed(place, head), ..self }
    }

    /// Their rows' trace.
    pub fn rows_trace(&self) -> &Trace {
        &self.rows
    }

    /// Their rows' head, as a run's pins record it.
    pub fn head(&self) -> Option<BlockId> {
        self.rows.head()
    }

    /// Forgets the blobs whose block nothing holds: a slab the residency retired or dropped, or a new process's (a slab
    /// is the run's memory). Their bytes are in the store's objects, loaded again when asked.
    pub fn forget_dead(&mut self, reg: &Registry, dir: &mut Directory) -> Result<(), String> {
        let rows = rows_at(dir, &self.rows, self.rows.head())?;
        let gone: Vec<(Vec<i64>, i32)> = rows.into_iter().filter(|r| !dir.alive(BlockId(r[4] as u32))).map(|r| (r, -1)).collect(); // copy: derived (the dead blobs' rows, killed)
        if !gone.is_empty() {
            push_layer(reg, &self.ops, dir, &mut self.rows, &gone)?;
        }
        Ok(())
    }

    /// Where blob `id`'s bytes lie, while they are resident: its row, probed by its name at the rows' head, its
    /// block still held (a slab the residency retired, or dropped, holds no blob).
    pub fn get(&self, dir: &Directory, id: &BlobId) -> Result<Option<Placed>, String> {
        Ok(self.row(dir, id)?.filter(|p| dir.alive(p.block)))
    }

    /// Blob `id`'s row, its block held or not.
    fn row(&self, dir: &Directory, id: &BlobId) -> Result<Option<Placed>, String> {
        let Some(h) = self.rows.head() else { return Ok(None) };
        let l = self.rows.read(dir, &[Some(h)])?;
        let table = l.table();
        let s = self.rows.state(&table, Some(h))?;
        let (cols, key) = ([0, 1, 2, 3].map(ColumnIndex::new), id.words());
        let c = |r, k| Read::get(&s, r, ColumnIndex::new(k));
        Ok(Read::matching(&s, &cols, &key).next().map(|r| Placed { block: BlockId(c(r, 4) as u32), at: c(r, 5) as usize, len: c(r, 6) as usize }))
    }

    /// Houses the bytes at `p` where they lie, named by their sha256 (`house(bytes: Placed) -> BlobId`): the name,
    /// and whether they are new (a row added). A second of the same bytes is the first; its place is not kept.
    pub fn house(&mut self, reg: &Registry, dir: &mut Directory, p: Placed) -> Result<(BlobId, bool), String> {
        if !dir.is_fixed(p.block, p.at + p.len) {
            return Err(format!("a blob's bytes in {}, not fixed: a block not frozen, or a slab's room not sealed (a blob is never versioned)", p.block));
        }
        let id = {
            let l = dir.read("house", &[p.block])?;
            BlobId::of(&l.block(0)[p.at..p.at + p.len])
        };
        let was = self.row(dir, &id)?;
        if was.is_some_and(|w| dir.alive(w.block)) {
            return Ok((id, false));
        }
        let row = |p: Placed| -> Vec<i64> { id.words().into_iter().chain([i64::from(p.block.0), p.at as i64, p.len as i64]).collect() }; // copy: derived (a blob's row, written once)
        // A blob whose slab went, resident again: its row moves to where it lies now.
        let pairs: Vec<(Vec<i64>, i32)> = was.map(|w| (row(w), -1)).into_iter().chain([(row(p), 1)]).collect(); // copy: derived (the row's change)
        push_layer(reg, &self.ops, dir, &mut self.rows, &pairs)?;
        Ok((id, true))
    }

    /// How many it holds.
    pub fn len(&self, dir: &Directory) -> Result<usize, String> {
        Ok(rows_at(dir, &self.rows, self.rows.head())?.len())
    }
}

/// A document's two blobs: its source and its tape (P§11.4).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DocIds {
    pub source: BlobId,
    pub tape: BlobId,
}

/// A document's reads, its source and its tape where they lie: `src_block` and `tape_block` the bytes of the blocks
/// they lie in (a launch's). The two are blobs of their own, by content, so they may lie in two blocks: a new
/// source whose tape is the same bytes as an earlier document's has its tape where that one's lies.
pub fn reads<'b>(src_block: &'b [u8], source: Placed, tape_block: &'b [u8], tape: Placed) -> crate::tape::Reads<'b> {
    let src = std::str::from_utf8(&src_block[source.at..source.at + source.len]).expect("a source the file layer checked is UTF-8");
    crate::tape::Reads::in_place(&tape_block[tape.at..tape.at + tape.len], src).expect("a tape the parser wrote")
}

/// The nodes of the document whose source and tape lie at `source` and `tape`, read where they lie.
pub fn nodes(dir: &Directory, source: Placed, tape: Placed) -> Result<usize, String> {
    let l = dir.read("a document's nodes", &[source.block, tape.block])?;
    Ok(reads(l.block(0), source, l.block(1), tape).len())
}

/// A document's tape port: its path relative to the root, the module its answer names, and its bistate, from the
/// tape the last answer gave (`None`: a cold run) to this one's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Doc {
    pub path: String,
    pub module: String,
    pub before: Option<DocIds>,
    pub after: DocIds,
}

impl Doc {
    /// Whether the tape is unchanged (L4's check).
    pub fn is_identity(&self) -> bool {
        self.before == Some(self.after)
    }
}

/// The store a run keeps across processes (*Operations* §5, kept as is): one folder holding the observer's store
/// (`objects.db.files`, its answers by path and stat, under the journal), the content store's objects (`objects.db` and its
/// segments: sources and tapes by content, a tape by its grammar), the store's atoms (`atoms.blk`, decision 0401) and
/// the kept traces' blocks (`kept/`, a file a block, named by its id).
pub struct Store {
    pub folder: PathBuf,
    pub objects: ContentStore,
}

impl Store {
    /// The store in `folder`, made when there is none, the user's alone.
    pub fn open(folder: &Path) -> Result<Store, String> {
        // The observer keeps its answers in the content store's facts (`STORE`), which boot registers as the program
        // loads; a process without boot (a test's) registers it here, once.
        static REGISTER: std::sync::Once = std::sync::Once::new();
        REGISTER.call_once(|| {
            if rerail_store_base::content_store::observer_store::base::STORE.try_get().is_none() {
                rerail_store::content_store::observer_store::register();
            }
        });
        // The user's alone (decision 0574): each folder made `0700`, and checked.
        rerail_blocks::mapped::make_folder(&folder.join("kept")).map_err(|e| format!("{}: {e}", folder.display()))?;
        let objects = ContentStore::open(&folder.join("objects.db")).map_err(|e| format!("{}: {e}", folder.display()))?;
        Ok(Store { folder: folder.to_path_buf(), objects })
    }

    /// The observer's store: the P0 store beside the content store (`observe::store_beside`, `<objects.db>.files`),
    /// its answers kept in the content store's facts across processes.
    pub fn observer(&self) -> PathBuf {
        observe::store_beside(&self.folder.join("objects.db"))
    }

    /// The kept traces' folder.
    pub fn kept(&self) -> PathBuf {
        self.folder.join("kept")
    }

    /// The store's atoms' file.
    pub fn atoms(&self) -> PathBuf {
        self.folder.join("atoms.blk")
    }
}

/// The pure answerers' tags in the answers trace (P§12 item 7: `AnswerKind::Pure`, kept by key, D4).
pub const PARSE: i64 = 1;
pub const INTERN: i64 = 2;

/// The answerers (P§12 item 7), the abstraction in the blocks crate; the engine's three below.
pub use rerail_blocks::answer::{Answer, AnswerKind, Answerer};

/// What an answerer reaches: the world, the registry, the trace's operations, the directory and the blobs.
pub struct Reach<'a> {
    pub world: &'a mut World,
    pub reg: &'a Registry,
    pub ops: &'a TraceOps,
    pub dir: &'a mut Directory,
    pub blobs: &'a mut BlobRows,
}

/// The load answerer (B§7's `load`: path → bytes), a world reader: key `[d]`, a document `named` names (its path, its
/// module, the blobs it was last answered with); its answer, its source's blob, read through Δanswer's operations
/// (`World::answer`), or a miss. Its leaf is written, misses too.
pub struct Load<'a> {
    pub at: Reach<'a>,
    pub named: &'a [Load1],
}

impl Answerer for Load<'_> {
    const KIND: AnswerKind = AnswerKind::World;
    type Leaves = LeavesW;
    fn answer(&mut self, key: &[i64], leaves: &mut LeavesW) -> Result<Answer, String> {
        let a = self.named.iter().find(|a| a.d == key[0]).ok_or_else(|| format!("a load of document {}, which no request names", key[0]))?;
        let Reach { world, reg, ops, dir, blobs } = &mut self.at;
        match world.answer(reg, ops, dir, blobs, std::slice::from_ref(a), leaves)?.pop().expect("one answer")? {
            Some(doc) => Ok(Answer::Blob(doc.after.source)),
            None => Ok(Answer::Miss),
        }
    }
}

/// The parse answerer ((content, grammar) → tape), pure: key a source's four words; its answer, the tape's blob, from
/// a home that holds it (the residency, else the store's objects), else the source parsed where it lies (the `parse`
/// operation) and put in the store's objects; a miss when no home holds the source either.
pub struct Parse<'a> {
    pub at: Reach<'a>,
}

impl Answerer for Parse<'_> {
    const KIND: AnswerKind = AnswerKind::Pure;
    type Leaves = LeavesW;
    fn answer(&mut self, key: &[i64], _: &mut LeavesW) -> Result<Answer, String> {
        let Reach { world, reg, ops, dir, blobs } = &mut self.at;
        let g = BlobId::of_words(&key[..4]);
        if let Some(ids) = world.by_content(reg, ops, dir, blobs, g)? {
            return Ok(Answer::Blob(ids.tape));
        }
        let Some(src) = blobs.get(dir, &g)? else { return Ok(Answer::Miss) };
        let tape = parse_sources(reg, &world.ops, dir, src.block, &[(src.at, src.len)])?.pop().expect("one");
        dir.seal(src.block)?;
        world.asked.parses += 1;
        Ok(Answer::Blob(world.parsed(reg, ops, dir, blobs, src.block, g, src, tape)?.tape))
    }
}

/// The intern answerer ((tape, path, module) → symbols), pure over the store's atoms, which never give an id out again:
/// key `[d, source's four words, tape's four words]`, a document `named` names; its answer, the symbols' blob.
pub struct Intern<'a> {
    pub at: Reach<'a>,
    pub iops: &'a crate::intern::InternOps,
    pub atoms: crate::sym::Atoms,
    pub named: &'a [Load1],
}

impl Answerer for Intern<'_> {
    const KIND: AnswerKind = AnswerKind::Pure;
    type Leaves = LeavesW;
    fn answer(&mut self, key: &[i64], _: &mut LeavesW) -> Result<Answer, String> {
        let a = self.named.iter().find(|a| a.d == key[0]).ok_or_else(|| format!("an intern of document {}, which no request names", key[0]))?;
        let Reach { world, reg, ops, dir, blobs } = &mut self.at;
        let doc = Doc { path: a.path.clone(), module: a.module.clone(), before: a.before, after: DocIds { source: BlobId::of_words(&key[1..5]), tape: BlobId::of_words(&key[5..9]) } };
        Ok(Answer::Blob(world.intern_answer(reg, ops, self.iops, dir, blobs, self.atoms, &doc)?))
    }
}

/// The observer's store under `folder` forgotten, as a new process finds it with no stat of its own (tests of a later
/// process: every file is read again by its content).
#[cfg(test)]
pub(crate) fn forget_observer(folder: &std::path::Path) {
    observe::forget(&observe::store_beside(&folder.join("objects.db")));
}

/// A run's layer of the world's leaves (P§12's `leaves: &mut LayerW`): each leaf a world reader read, `(d, digest,
/// found)`, against the row the committed trace holds for its key, pushed as its change into a δ runs block, and
/// committed as the run's layer (`World::commit_leaves`). A key's leaf is written once a run (its answer is).
pub struct LeavesW {
    runs: BlockId,
}

/// The leaves' rows (P§11.4: "leaves read: path, digest, hit or miss", the world's before): `(d, digest's four words,
/// found)`, a document's key and what its path held when last read (found 0: no readable file there, a miss).
pub fn leaves_shape() -> Shape {
    let c = vec![ColType::I32, ColType::I64, ColType::I64, ColType::I64, ColType::I64, ColType::I32];
    Shape::run(c, (0..6).collect(), Some(0))
}

/// The pure answers' rows (P§11.4: "blobs, plus a trace from key to blob id"): `(answerer, six key words, blob's four
/// words)`, partitioned by answerer and ordered by key, so a key's probe is a prefix of the canonical order.
pub fn answers_shape() -> Shape {
    let c = std::iter::once(ColType::I32).chain(std::iter::repeat_n(ColType::I64, 10)).collect();
    Shape::run(c, (0..11).collect(), Some(0))
}

/// The documents' places (decision 0494; *Operations* §5, kept as is): `(d, source's segment, offset and length, the
/// tape's hash (its algorithm and four lanes), its segment, offset and length, its length)`, so a later process reads a
/// document's source and tape where the store's segments hold them, with no SQL, each checked by its hash.
pub fn places_shape() -> Shape {
    let c = std::iter::once(ColType::I32).chain(std::iter::repeat_n(ColType::I64, 12)).collect();
    Shape::run(c, (0..13).collect(), Some(0))
}

/// The first row of `t` at its head whose leading columns are `key`, read where it lies.
pub fn probe(dir: &Directory, t: &Trace, key: &[i64]) -> Result<Option<Vec<i64>>, String> {
    let Some(h) = t.head() else { return Ok(None) };
    let l = t.read(dir, &[Some(h)])?;
    let table = l.table();
    let s = t.state(&table, Some(h))?;
    let cols: Vec<ColumnIndex> = (0..key.len() as u16).map(ColumnIndex::new).collect();
    // copy: derived (one row's cells, read where they lie)
    Ok(Read::matching(&s, &cols, key).next().map(|r| (0..Read::arity(&s) as u16).map(|c| Read::get(&s, r, ColumnIndex::new(c))).collect()))
}

/// `t`'s row keyed by `key` (its leading columns) replaced by `row` (`None`: taken out), in one layer; nothing written
/// when it holds that row already.
pub fn put_row(reg: &Registry, ops: &TraceOps, dir: &mut Directory, t: &mut Trace, key: &[i64], row: Option<Vec<i64>>) -> Result<(), String> {
    let old = probe(dir, t, key)?;
    if old == row {
        return Ok(());
    }
    let pairs: Vec<(Vec<i64>, i32)> = old.map(|r| (r, -1)).into_iter().chain(row.map(|r| (r, 1))).collect(); // copy: derived (the row's change)
    push_layer(reg, ops, dir, t, &pairs)?;
    Ok(())
}

/// A document's key (decision 0052 §3, `rerail_blocks::blob::doc_key`): from its path's hash, the next free one on a
/// collision, `taken` saying which are.
pub fn key_of(path: &str, taken: impl Fn(i64) -> bool) -> i64 {
    let mut k = rerail_blocks::blob::doc_key(path);
    while taken(k) {
        k = rerail_blocks::blob::next_key(k);
    }
    k
}

/// What answering a document asked of the world and its answerers (the observer's counts are the files'): the files
/// read and placed, the documents loaded from the store's objects (a hit: no read, no parse), and the parses.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Asked {
    pub reads: u64,
    pub loads: u64,
    pub parses: u64,
    pub interns: u64,
}

/// The world a run reads (P§7, *Operations* §5): the root's files through the engine's file layer as it is, the
/// observer's digest step under the journal and the racy rule, the placed read into a slab room, and the content
/// store's objects; with what it keeps across runs as kept traces: the leaves it read (the world's before), the pure
/// answers by key, and the documents' places.
pub struct World {
    root: PathBuf,
    pub store: Store,
    /// Δanswer's operations (`answer_load`, `parse`).
    ops: AnswerOps,
    grammar: [i64; 3],
    pub leaves: Trace,
    pub answers: Trace,
    pub places: Trace,
    pub asked: Asked,
}

impl World {
    /// The world of the files under `root`, kept in `store`, read by Δanswer's operations `ops`, its traces at `heads` (`[leaves, answers, places]`, a
    /// kept run's; `None`: empty), each placed where the directory keeps what lasts.
    pub fn new(root: PathBuf, store: Store, ops: AnswerOps, dir: &Directory, heads: [Option<BlockId>; 3]) -> Result<World, String> {
        let place = dir.lasting();
        let t = |shape: Shape, head| Ok::<_, String>(Trace::new(&shape)?.resumed(place, head));
        Ok(World {
            root,
            store,
            ops,
            grammar: rerail_rr_syntax::grammar(),
            leaves: t(leaves_shape(), heads[0])?,
            answers: t(answers_shape(), heads[1])?,
            places: t(places_shape(), heads[2])?,
            asked: Asked::default(),
        })
    }

    /// The traces' heads (`[leaves, answers, places]`), as a run's pins record them.
    pub fn heads(&self) -> [Option<BlockId>; 3] {
        [self.leaves.head(), self.answers.head(), self.places.head()]
    }

    /// The grammar's one word in a key: its version (the tag and the format are the rr grammar's).
    fn grammar_word(&self) -> i64 {
        self.grammar[1]
    }

    /// Starts a run (decision 0105): the observer asks the journal once what changed under the root since the run
    /// before, and trusts the rest with no `stat`.
    pub fn begin(&self) {
        let root = self.root.clone();
        observe::with(Some(&self.store.observer()), |o| {
            o.begin();
            o.watch(&[root]);
        });
    }

    /// Ends a run: the observer's answers written, the journal's cursor kept.
    pub fn end(&self) {
        observe::with(Some(&self.store.observer()), |o| o.flush());
    }

    /// The observer's counts (its stats, reuses, files read and hashed).
    pub fn counts(&self) -> observe::Counts {
        observe::with(Some(&self.store.observer()), |o| o.counts)
    }

    /// Leaf `d`'s row as read now: `digest` (`None`: a miss), written when it changed (D3: every leaf, misses too).
    fn leaf(&mut self, reg: &Registry, ops: &TraceOps, dir: &mut Directory, leaves: &mut LeavesW, d: i64, digest: Option<[u8; 32]>) -> Result<(), String> {
        let w = digest.map_or([0; 4], |g| BlobId(g).words());
        let row = vec![d, w[0], w[1], w[2], w[3], i64::from(digest.is_some())];
        let old = probe(dir, &self.leaves, &[d])?;
        if old.as_ref() == Some(&row) {
            return Ok(());
        }
        let (len, cap, written) = {
            let l = dir.read("the run's leaves", &[leaves.runs])?;
            let r = Runs::new(l.block(0));
            // A key's leaf is written once a run: a run reads the world in one state, so a second read of its path
            // is the first's answer, read where the run's layer holds it.
            let written = (0..r.len()).any(|i| r.diff(i) > 0 && r.get(i, 0) == d);
            (r.len(), r.head.cap as usize, written)
        };
        if written {
            return Ok(());
        }
        // Past its room: the layer so far committed, and the run's next leaves in a runs block twice the size.
        if len + 2 > cap {
            self.commit_leaves(reg, ops, dir, LeavesW { runs: leaves.runs })?;
            leaves.runs = self.leaves.open(dir, 2 * cap);
        }
        let b = dir.block(leaves.runs).ok_or("the run's leaves, held by nothing")?;
        let pin = rerail_blocks::region::pin_block(&b, "the run's leaves", true)?;
        let w = rerail_blocks::layer::RunsW::new(&pin);
        for (r, diff) in old.map(|r| (r, -1)).into_iter().chain([(row, 1)]) {
            w.push(&r, diff).ok_or("the run's leaves, full")?;
        }
        Ok(())
    }

    /// A run's layer of its leaves, open (`LeavesW`).
    pub fn leaves_w(&self, dir: &mut Directory) -> LeavesW {
        LeavesW { runs: self.leaves.open(dir, 64) }
    }

    /// The run's leaves committed as a layer of the leaves trace (none when it read nothing new).
    pub fn commit_leaves(&mut self, reg: &Registry, ops: &TraceOps, dir: &mut Directory, leaves: LeavesW) -> Result<(), String> {
        let n = Runs::new(dir.read("the run's leaves", &[leaves.runs])?.block(0)).len();
        if n == 0 {
            dir.release(leaves.runs);
            return Ok(());
        }
        self.leaves.commit(reg, ops, dir, leaves.runs, crate::op::Tiling::WHOLE).map(drop)
    }

    /// Document `d` at `path` through Δanswer, `before` the blobs it was last answered with (`answer`, for one), its
    /// leaf written in `leaves`: a miss refused, naming the path.
    #[allow(clippy::too_many_arguments)]
    pub fn load(&mut self, reg: &Registry, ops: &TraceOps, dir: &mut Directory, blobs: &mut BlobRows, leaves: &mut LeavesW, d: i64, path: &str, module: &str, before: Option<DocIds>) -> Result<Doc, String> {
        let ask = Load1 { d, path: path.to_string(), module: module.to_string(), before };
        self.answer(reg, ops, dir, blobs, std::slice::from_ref(&ask), leaves)?.pop().expect("one answer")?.ok_or_else(|| format!("{path}: not a readable file"))
    }

    /// The load answers for `asks` (Δanswer, *Operations* §10: `answer_load`, then `parse` for the contents no home
    /// holds; `intern` follows, `intern_answer`): each ask's leaf recorded, misses too (D3), and its document, the
    /// identity while its digest is unchanged. A file the observer answers by its stat is not read; one whose content
    /// a home holds (the residency, else the store's objects) is not parsed (a hit skips the parse, §6.2); only a new
    /// content is parsed, its source and tape then put in the store's objects. An ask's refusal names its path.
    pub fn answer(&mut self, reg: &Registry, ops: &TraceOps, dir: &mut Directory, blobs: &mut BlobRows, asks: &[Load1], leaves: &mut LeavesW) -> Result<Vec<Result<Option<Doc>, String>>, String> {
        let doc = |a: &Load1, after| Some(Doc { path: a.path.clone(), module: a.module.clone(), before: a.before, after });
        let mut out: Vec<Option<Result<Option<Doc>, String>>> = vec![None; asks.len()];
        // (ask, its flags), and the room the next pass's slab needs.
        let mut pending: Vec<(usize, i64)> = (0..asks.len()).map(|k| (k, 0)).collect(); // copy: derived (the asks left)
        let mut need = room(0);
        let observer = self.store.observer();
        while !pending.is_empty() {
            let (slab, _) = dir.slab_room(kind::TAPE, need)?;
            let plan: Vec<Ask> = pending.iter().map(|&(k, flags)| Ask { d: asks[k].d, path: asks[k].path.clone(), before: asks[k].before.map(|b| b.source.0), flags }).collect(); // copy: derived (the load δ)
            let (facts, reads) = answer_load(reg, &self.ops, dir, slab, &self.root, &observer, &plan)?;
            dir.seal(slab)?;
            self.asked.reads += reads;
            let mut next = Vec::new();
            let mut parse: Vec<(usize, BlobId, usize, usize)> = Vec::new();
            need = room(0);
            for (&(k, _), f) in pending.iter().zip(&facts) {
                let a = &asks[k];
                let g = (f[2] != 0).then(|| BlobId::of_words(&f[3..7]));
                match f[1] {
                    code::NO_ROOM => {
                        need = need.max(f[9] as usize);
                        next.push((k, 0));
                        continue;
                    }
                    // Known by its digest, no home holds it: read it, its leaf written when it is.
                    code::KNOWN if self.by_content(reg, ops, dir, blobs, g.expect("a known digest"))?.is_none() => {
                        next.push((k, READ_ANYWAY));
                        continue;
                    }
                    code::UNREAD => self.leaf(reg, ops, dir, leaves, a.d, None)?,
                    _ => self.leaf(reg, ops, dir, leaves, a.d, g.map(|g| g.0))?,
                }
                out[k] = match (f[1], g) {
                    (code::MISS, _) => Some(Ok(None)),
                    (code::UNREAD, _) => Some(Err(format!("{}: could not be read", a.path))),
                    (code::SAME, _) => Some(Ok(doc(a, a.before.expect("its digest was its last answer's")))),
                    (code::READ, Some(g)) if a.before.is_some_and(|b| b.source == g) => Some(Ok(doc(a, a.before.expect("kept")))),
                    (code::KNOWN | code::READ, Some(g)) => match self.by_content(reg, ops, dir, blobs, g)? {
                        Some(ids) => Some(Ok(doc(a, ids))),
                        None => {
                            parse.push((k, g, f[7] as usize, f[8] as usize));
                            None
                        }
                    },
                    _ => Some(Err(format!("{}: answer_load gave {f:?}", a.path))),
                };
            }
            if !parse.is_empty() {
                let sources: Vec<(usize, usize)> = parse.iter().map(|p| (p.2, p.3)).collect(); // copy: derived (the sources to parse)
                let tapes = parse_sources(reg, &self.ops, dir, slab, &sources)?;
                dir.seal(slab)?;
                for (&(k, g, at, len), t) in parse.iter().zip(tapes) {
                    self.asked.parses += 1;
                    out[k] = Some(self.parsed(reg, ops, dir, blobs, slab, g, Placed { block: slab, at, len }, t).map(|ids| doc(&asks[k], ids)).map_err(|e| format!("{}: {e}", asks[k].path)));
                }
            }
            pending = next;
        }
        Ok(out.into_iter().map(|o| o.expect("every ask answered")).collect())
    }

    /// A source `src` (content `g`) and the tape `parse` gave it in slab `slab`: refused when its parse was, or when a
    /// statement does not parse (the parser writes it as an ERROR leaf); else both housed as blobs, put in the store's
    /// objects, and its parse answer kept.
    #[allow(clippy::too_many_arguments)]
    fn parsed(&mut self, reg: &Registry, ops: &TraceOps, dir: &mut Directory, blobs: &mut BlobRows, slab: BlockId, g: BlobId, src: Placed, tape: crate::answer::Parsed) -> Result<DocIds, String> {
        let t = parsed_tape(dir, slab, src, tape)?;
        let (source, _) = blobs.house(reg, dir, src)?;
        if source != g {
            return Err(format!("its bytes are {source:?}, the observer's digest {g:?}"));
        }
        let (tape, _) = blobs.house(reg, dir, t)?;
        let ids = DocIds { source, tape };
        self.put_object(reg, ops, dir, blobs, ids)?;
        Ok(ids)
    }

    /// The parse answer for content `source` (`Parse`, pure: (content, grammar) → tape), when a home holds it: the
    /// residency (its blobs resident), else the store's objects (its source and tape loaded into a slab room, a load,
    /// not a parse). `None`: a content no home holds, to read and parse.
    pub fn by_content(&mut self, reg: &Registry, ops: &TraceOps, dir: &mut Directory, blobs: &mut BlobRows, source: BlobId) -> Result<Option<DocIds>, String> {
        let key = parse_key(source, self.grammar_word());
        if let Some(row) = probe(dir, &self.answers, &key)? {
            let tape = BlobId::of_words(&row[7..11]);
            if blobs.get(dir, &source)?.is_some() && blobs.get(dir, &tape)?.is_some() {
                return Ok(Some(DocIds { source, tape }));
            }
        }
        let Some(places) = self.store.objects.document_places(source.0, self.grammar).map_err(|e| e.to_string())? else { return Ok(None) };
        let ids = self.load_places(reg, dir, blobs, &places)?;
        put_row(reg, ops, dir, &mut self.answers, &key, Some(answer_row(&key, ids.tape)))?;
        Ok(Some(ids))
    }

    /// The source and tape `places` names, read where the store's segments hold them (no SQL) and loaded into a room
    /// of a slab, the residency's home (a load to their exact bytes: 0353), then housed. Refused when the store no
    /// longer holds them there.
    fn load_places(&mut self, reg: &Registry, dir: &mut Directory, blobs: &mut BlobRows, places: &DocumentPlaces) -> Result<DocIds, String> {
        let src = self.store.objects.placed_source(places).ok_or("a document's source, gone from the store's segments")?;
        let tape = self.store.objects.placed_tape(places).ok_or("a document's tape, gone from the store's segments")?;
        let tape = tape.bytes().map_err(|e| e.to_string())?;
        let (src, tape): (&[u8], &[u8]) = (&src, &tape);
        let (block, b) = dir.slab_room(kind::TAPE, src.len() + tape.len() + 64)?;
        let put = |bytes: &[u8]| {
            let mut m = Mutable::<u8>::alloc_aligned(&b, bytes.len().max(1) + 16, 64, "a document loaded from the store").ok_or("a slab's room")?;
            m.write()[..bytes.len()].copy_from_slice(bytes); // copy: load (the store's object to its exact bytes in the residency's slab)
            let at = m.region().at();
            drop(m.freeze());
            Ok::<_, String>(Placed { block, at, len: bytes.len() })
        };
        let (s, t) = (put(src)?, put(tape)?);
        drop(b);
        dir.seal(block)?;
        let (source, _) = blobs.house(reg, dir, s)?;
        let (tape, _) = blobs.house(reg, dir, t)?;
        self.asked.loads += 1;
        Ok(DocIds { source, tape })
    }

    /// A parsed document's source and tape put in the store's objects (the bytes read where they lie, written once to
    /// a segment of their own), and its parse answer kept.
    fn put_object(&mut self, reg: &Registry, ops: &TraceOps, dir: &mut Directory, blobs: &BlobRows, ids: DocIds) -> Result<(), String> {
        let (s, t) = (blobs.get(dir, &ids.source)?.ok_or("a source just housed")?, blobs.get(dir, &ids.tape)?.ok_or("a tape just housed")?);
        {
            let l = dir.read("a document put in the store", &[s.block, t.block])?;
            let (src, tape) = (&l.block(0)[s.at..s.at + s.len], &l.block(1)[t.at..t.at + t.len]);
            let [g, v, f] = self.grammar;
            self.store.objects.put_source_and_tape(src, HashKey::from_sha256_digest(ids.source.0), g, v, f, tape).map_err(|e| e.to_string())?;
        }
        let key = parse_key(ids.source, self.grammar_word());
        put_row(reg, ops, dir, &mut self.answers, &key, Some(answer_row(&key, ids.tape)))
    }

    /// Document `d`'s places, as the store's objects hold its source `source` and its tape: kept, so a later process
    /// reads them where they lie.
    pub fn place(&mut self, reg: &Registry, ops: &TraceOps, dir: &mut Directory, d: i64, source: Option<BlobId>) -> Result<(), String> {
        let row = match source {
            None => None,
            Some(s) => {
                let p = self.store.objects.document_places(s.0, self.grammar).map_err(|e| e.to_string())?.ok_or("a document's source and tape, not in the store")?;
                Some(std::iter::once(d).chain(p.source).chain(p.tape_hash).chain(p.tape).chain([p.tape_length]).collect())
            }
        };
        put_row(reg, ops, dir, &mut self.places, &[d], row)
    }

    /// Document `d`'s blobs resident: loaded from the store where its places say, when the residency no longer
    /// holds them (a slab retired, or a process started since). The sha256 its source has is the place's check.
    pub fn resident(&mut self, reg: &Registry, dir: &mut Directory, blobs: &mut BlobRows, d: i64, ids: DocIds) -> Result<(), String> {
        if blobs.get(dir, &ids.source)?.is_some() && blobs.get(dir, &ids.tape)?.is_some() {
            return Ok(());
        }
        let row = probe(dir, &self.places, &[d])?.ok_or("a document's places, not kept")?;
        let w = |a: usize, b: usize| -> Vec<i64> { row[a..b].to_vec() }; // copy: derived (a place's words)
        let places = DocumentPlaces {
            sha256: ids.source.0,
            source: w(1, 4).try_into().expect("three"),
            tape_hash: w(4, 9).try_into().expect("five"),
            tape: w(9, 12).try_into().expect("three"),
            tape_length: row[12],
        };
        let got = self.load_places(reg, dir, blobs, &places)?;
        if got != ids {
            return Err(format!("document {d}'s places hold {got:?}, not {ids:?}"));
        }
        Ok(())
    }

    /// The intern answer (`Intern`, pure) for `doc`'s tape at its path and module: its symbols blob, kept by key
    /// while the atoms keep their ids; `intern` runs only for a key not answered.
    #[allow(clippy::too_many_arguments)]
    pub fn intern_answer(&mut self, reg: &Registry, ops: &TraceOps, iops: &crate::intern::InternOps, dir: &mut Directory, blobs: &mut BlobRows, atoms: crate::sym::Atoms, doc: &Doc) -> Result<BlobId, String> {
        let key = match (atoms.find(dir, &doc.path), atoms.find(dir, &doc.module)) {
            (Some(p), Some(m)) => Some(intern_key(doc.after.tape, p, m)),
            _ => None,
        };
        if let Some(k) = &key
            && let Some(row) = probe(dir, &self.answers, k)?
        {
            let s = BlobId::of_words(&row[7..11]);
            if blobs.get(dir, &s)?.is_some() {
                return Ok(s);
            }
        }
        let get = |id: &BlobId| blobs.get(dir, id)?.ok_or_else(|| format!("{}: a blob nothing holds", doc.path));
        let (src, tape) = (get(&doc.after.source)?, get(&doc.after.tape)?);
        let s = crate::intern::intern(reg, iops, dir, blobs, atoms, src, tape, &doc.path, &doc.module)?;
        self.asked.interns += 1;
        let (p, m) = (atoms.find(dir, &doc.path).ok_or("the path, interned")?, atoms.find(dir, &doc.module).ok_or("the module, interned")?);
        let k = intern_key(doc.after.tape, p, m);
        put_row(reg, ops, dir, &mut self.answers, &k, Some(answer_row(&k, s)))?;
        Ok(s)
    }
}

/// A parse answer's key: the answerer, the source's four words, the grammar's word, and a word unused.
fn parse_key(source: BlobId, grammar: i64) -> Vec<i64> {
    std::iter::once(PARSE).chain(source.words()).chain([grammar, 0]).collect()
}

/// An intern answer's key: the answerer, the tape's four words, the path's and the module's symbols.
fn intern_key(tape: BlobId, path: i64, module: i64) -> Vec<i64> {
    std::iter::once(INTERN).chain(tape.words()).chain([path, module]).collect()
}

/// An answer's row: its key, then the blob.
fn answer_row(key: &[i64], blob: BlobId) -> Vec<i64> {
    key.iter().copied().chain(blob.words()).collect()
}

/// One ask of the load δ: document `d` at `path`, as `module`, last answered with `before`.
#[derive(Clone, Debug)]
pub struct Load1 {
    pub d: i64,
    pub path: String,
    pub module: String,
    pub before: Option<DocIds>,
}

/// A document of a source in hand (a test's, the bootstrap's own), in a slab's room as a read places one, parsed by
/// the `parse` operation into a room of the same slab, and the two housed as blobs.
pub fn doc_of(reg: &Registry, ops: &AnswerOps, dir: &mut Directory, blobs: &mut BlobRows, src: &str) -> Result<DocIds, String> {
    let (block, b) = dir.slab_room(kind::TAPE, room(src.len()))?;
    let home = place_in(&b, src.len(), &mut |out| {
        out.copy_from_slice(src.as_bytes()); // copy: load (a source in hand, written into its room)
        Ok(())
    })
    .map_err(|e| e.to_string())?;
    let (at, _) = room_at(&home).ok_or("a source placed in its slab")?;
    drop((home, b));
    let s = Placed { block, at, len: src.len() };
    let tape = parse_sources(reg, ops, dir, block, &[(at, src.len())])?.pop().expect("one");
    dir.seal(block)?;
    let t = parsed_tape(dir, block, s, tape)?;
    let (source, _) = blobs.house(reg, dir, s)?;
    let (tape, _) = blobs.house(reg, dir, t)?;
    Ok(DocIds { source, tape })
}

/// The tape `parse` gave the source at `src` in slab `slab`, where it lies: refused when the parse was, or when a
/// statement does not parse (the parser writes it as an ERROR leaf, and a file the engine refuses to run still has a
/// tape).
fn parsed_tape(dir: &Directory, slab: BlockId, src: Placed, tape: crate::answer::Parsed) -> Result<Placed, String> {
    let (at, len) = tape.map_err(|c| if c == crate::answer::NOT_UTF8 { "not UTF-8".to_string() } else { "the parser refused it".to_string() })?;
    let t = Placed { block: slab, at, len };
    let l = dir.read("a parsed tape", &[slab])?;
    let r = reads(l.block(0), src, l.block(0), t);
    if let Some(i) = (0..r.len()).find(|&i| r.node(i).0 == rerail_rr_syntax::tape::ERROR) {
        return Err(format!("a statement that does not parse: {:?}", r.text(i)));
    }
    Ok(t)
}

/// Where a placed source's room lies in its slab, and the source's length.
pub(crate) fn room_at(h: &home::Home) -> Option<(usize, usize)> {
    match h {
        home::Home::Placed(r) => r.as_any().downcast_ref::<SourceRoom>().map(|r| (r.region.region().at(), r.len)),
        home::Home::Own(_) => None,
    }
}

/// A source's room in its tape block: a region of it, frozen once the read has written it.
struct SourceRoom {
    region: Frozen<u8>,
    len: usize,
}

impl std::fmt::Debug for SourceRoom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SourceRoom {{ at: {}, len: {} }}", self.region.region().at(), self.len)
    }
}

impl home::Room for SourceRoom {
    fn bytes(&self) -> &[u8] {
        &self.region[..self.len]
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// `len` bytes written by `fill` into a room of block `b` (the placer a read is given): with a word's room past them,
/// as the slabs' placer leaves.
pub(crate) fn place_in(b: &Arc<rerail_blocks::block::Block>, len: usize, fill: &mut dyn FnMut(&mut [u8]) -> std::io::Result<()>) -> std::io::Result<home::Home> {
    let mut m = Mutable::<u8>::alloc_aligned(b, len + 16, 64, "a document's source").ok_or_else(|| std::io::Error::other("a tape block's room"))?;
    fill(&mut m.write()[..len])?;
    Ok(home::Home::Placed(Arc::new(SourceRoom { region: m.freeze(), len })))
}

/// The room a document of a source of `len` bytes needs in its slab: the source and a word past it, and the parser's
/// tape at its bound (80 times the source). It is address space; what is resident is what the two write.
pub(crate) fn room(len: usize) -> usize {
    81 * len + (64 << 10)
}

impl rerail_blocks::blob::Blobs for BlobRows {
    type Dir = Directory;
    type Reg = Registry;

    fn get(&self, dir: &Directory, id: &BlobId) -> Result<Option<Placed>, String> {
        BlobRows::get(self, dir, id)
    }

    fn house(&mut self, reg: &Registry, dir: &mut Directory, bytes: Placed) -> Result<(BlobId, bool), String> {
        BlobRows::house(self, reg, dir, bytes)
    }
}

#[cfg(test)]
#[path = "files_tests.rs"]
pub(crate) mod tests;
