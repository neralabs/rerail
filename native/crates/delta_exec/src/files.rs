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
use crate::op::Registry;
use crate::trace::{Trace, TraceOps};
use rerail_blocks::bistate::{ColumnIndex, Read};
use rerail_blocks::layer::{ColType, Shape};
use crate::tape::Text;
use rerail_blocks::block::kind;
use rerail_blocks::maps::InBlock;
use rerail_observe::content_store::observe::{self, DigestRead, DigestStep};
use rerail_blocks::maps::Placed as _;
use rerail_blocks::region::{Frozen, Mutable};
use rerail_rr_syntax::tape_format::home;
use std::collections::HashMap;
use std::path::PathBuf;
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

    /// Where blob `id`'s bytes lie: its row, probed by its name at the rows' head.
    pub fn get(&self, dir: &Directory, id: &BlobId) -> Result<Option<Placed>, String> {
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
        if self.get(dir, &id)?.is_some() {
            return Ok((id, false));
        }
        let row: Vec<i64> = id.words().into_iter().chain([i64::from(p.block.0), p.at as i64, p.len as i64]).collect(); // copy: derived (a blob's row, written once)
        push_layer(reg, &self.ops, dir, &mut self.rows, &[(row, 1)])?;
        Ok((id, true))
    }

    /// Retires the blocks that hold no blob of `live` (the residency's retirement, whole slabs): the blocks its rows
    /// name and the directory's slabs, a slab whose rooms were all housed already (a re-read of the same bytes) among
    /// them. Their blobs' rows go (−1), and the directory's own handle is let go, so each goes once no reference or
    /// reader holds it. The slab rooms go to now is kept. The blobs forgotten.
    pub fn retire(&mut self, reg: &Registry, dir: &mut Directory, live: &[BlobId]) -> Result<Vec<BlobId>, String> {
        let rows = rows_at(dir, &self.rows, self.rows.head())?;
        let mut blocks: Vec<(u32, bool)> = rows.iter().map(|r| (r[4] as u32, live.contains(&BlobId::of_words(&r[..4])))).collect(); // copy: derived (each blob's block, and whether it is live)
        blocks.sort_unstable();
        let held: Vec<u32> = blocks.iter().filter(|b| b.1).map(|b| b.0).collect(); // copy: derived (the blocks a live blob lies in)
        let retired = |b: u32| !held.contains(&b) && !dir.is_current_slab(BlockId(b));
        let gone: Vec<(Vec<i64>, i32)> = rows.iter().filter(|r| retired(r[4] as u32)).map(|r| (r.clone(), -1)).collect(); // copy: derived (the retired blobs' rows, killed)
        let mut freed: Vec<u32> = gone.iter().map(|(r, _)| r[4] as u32).chain(dir.slabs().into_iter().map(|s| s.0).filter(|&s| retired(s))).collect(); // copy: derived (the blocks retired)
        freed.sort_unstable();
        freed.dedup();
        if !gone.is_empty() {
            push_layer(reg, &self.ops, dir, &mut self.rows, &gone)?;
        }
        for b in freed {
            dir.release(BlockId(b));
        }
        Ok(gone.iter().map(|(r, _)| BlobId::of_words(&r[..4])).collect())
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

/// The module a document's path names: the path relative to the root, without its `.rerail`.
fn module_of(path: &str) -> String {
    path.strip_suffix(".rerail").unwrap_or(path).to_string()
}

/// The root's documents as the file layer answers them, with the digest and the blob each was last answered with.
pub struct Files {
    root: PathBuf,
    /// The observer's store (`observe::with`): its answers kept there.
    store: PathBuf,
    kept: HashMap<String, ([u8; 32], DocIds)>,
    /// Files read (each read placed once, by `read_placed`).
    pub reads: u64,
}

impl Files {
    pub fn new(root: PathBuf, store: PathBuf) -> Files {
        Files { root, store, kept: HashMap::new(), reads: 0 }
    }

    /// Forgets its answers that name a blob of `gone` (retired): their paths are read again when next asked.
    pub fn forget(&mut self, gone: &[BlobId]) {
        self.kept.retain(|_, (_, d)| !gone.contains(&d.source) && !gone.contains(&d.tape));
    }

    /// The observer's counts (its stats, reuses, files read and hashed).
    pub fn counts(&self) -> observe::Counts {
        observe::with(Some(&self.store), |o| o.counts.clone())
    }

    /// Document `path` (relative to the root): its tape port, the tape housed in `blobs`, a tape block made in `dir`
    /// for a tape the parser writes.
    pub fn answer(&mut self, path: &str, reg: &Registry, blobs: &mut BlobRows, dir: &mut Directory) -> Result<Doc, String> {
        let abs = self.root.join(path);
        let kept = self.kept.get(path).copied();
        let doc = |before: Option<DocIds>, after| Doc { path: path.to_string(), module: module_of(path), before, after };
        let step = observe::with(Some(&self.store), |o| o.digest_step(&abs));
        let read = match step {
            DigestStep::Answered(None) => return Err(format!("{path}: not a readable file")),
            DigestStep::Answered(Some(d)) => match kept {
                Some((k, b)) if k == d => return Ok(doc(Some(b), b)),
                // Known to the observer, not to these documents: read.
                _ => None,
            },
            DigestStep::Read(r) => Some(r),
        };
        let size = read.as_ref().and_then(|r| r.stat()).map_or(1 << 20, |s| s.size as usize);
        let (block, b) = dir.slab_room(kind::TAPE, room(size)).map_err(|e| format!("{path}: {e}"))?;
        let place = |len: usize, fill: &mut dyn FnMut(&mut [u8]) -> std::io::Result<()>| place_in(&b, len, fill);
        let placed = match read {
            Some(r) => match r.read_placed(&place) {
                Some(home) => {
                    let hashed = DigestRead::hashed(&home);
                    observe::with(Some(&self.store), |o| o.digest_read(r, Some(hashed))).map(|d| (d, home))
                }
                None => None,
            },
            None => rerail_observe::git_workspace::snapshot::read_placed(&abs, &place).ok().map(|home| (DigestRead::hashed(&home).1, home)),
        };
        let Some((digest, source)) = placed else {
            dir.seal(block)?;
            return Err(format!("{path}: could not be read"));
        };
        self.reads += 1;
        if let Some((k, b)) = kept
            && k == digest
        {
            // Read again (its stat changed, or a check), the same bytes: the identity. Its room stays in its slab, as
            // the residency's rooms do, until the slab goes.
            drop(source);
            dir.seal(block)?;
            return Ok(doc(Some(b), b));
        }
        let id = house(reg, dir, blobs, block, b, source).map_err(|e| format!("{path}: {e}"))?;
        self.kept.insert(path.to_string(), (digest, id));
        Ok(doc(kept.map(|k| k.1), id))
    }
}

/// A document of a source in hand (a test's, the bootstrap's own), in a slab's rooms, as a read places one.
pub fn doc_of(reg: &Registry, dir: &mut Directory, blobs: &mut BlobRows, src: &str) -> Result<DocIds, String> {
    let (block, b) = dir.slab_room(kind::TAPE, room(src.len()))?;
    let home = place_in(&b, src.len(), &mut |out| {
        out.copy_from_slice(src.as_bytes()); // copy: load (a source in hand, written into its room)
        Ok(())
    })
    .map_err(|e| e.to_string())?;
    house(reg, dir, blobs, block, b, home)
}

/// A document's source in its slab's room: the tape parsed from it into a room of the same slab, the rooms sealed, and
/// the two housed as blobs. Rooms whose bytes were housed already, or a parse refused, stay in the slab until it goes.
fn house(reg: &Registry, dir: &mut Directory, blobs: &mut BlobRows, block: BlockId, b: Arc<rerail_blocks::block::Block>, source: home::Home) -> Result<DocIds, String> {
    let parsed = parse(block, b, source);
    dir.seal(block)?;
    let (s, t) = parsed?;
    let (source, _) = blobs.house(reg, dir, s)?;
    let (tape, _) = blobs.house(reg, dir, t)?;
    Ok(DocIds { source, tape })
}

/// A document's source in its slab's room, and its tape parsed into a room of the same slab: where each lies. Every
/// handle on the slab is let go here, and the rooms are written: they are sealed next.
fn parse(block: BlockId, b: Arc<rerail_blocks::block::Block>, source: home::Home) -> Result<(Placed, Placed), String> {
    let src_at = source.room().and_then(|r| r.as_any().downcast_ref::<SourceRoom>()).map(|r| r.region.region().at()).ok_or("a source placed in its slab")?;
    let src = home::Text::new(source).map_err(|e| format!("not UTF-8: {e}"))?;
    let text = Text::parse_in(src, Arc::new(InBlock(b.clone())))?;
    let tape_at = match text.tape.home() {
        home::Home::Own(v) => {
            let (tb, at) = v.place().ok_or("a tape written in its block")?;
            assert!(Arc::ptr_eq(tb, &b), "the tape in its document's block");
            at
        }
        home::Home::Placed(_) => return Err("a tape the parser wrote is its writer's".into()),
    };
    let (src_len, tape_len) = (text.src.as_str().len(), text.tape.as_bytes().len());
    Ok((Placed { block, at: src_at, len: src_len }, Placed { block, at: tape_at, len: tape_len }))
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
fn place_in(b: &Arc<rerail_blocks::block::Block>, len: usize, fill: &mut dyn FnMut(&mut [u8]) -> std::io::Result<()>) -> std::io::Result<home::Home> {
    let mut m = Mutable::<u8>::alloc_aligned(b, len + 16, 64, "a document's source").ok_or_else(|| std::io::Error::other("a tape block's room"))?;
    fill(&mut m.write()[..len])?;
    Ok(home::Home::Placed(Arc::new(SourceRoom { region: m.freeze(), len })))
}

/// The room a document of a source of `len` bytes needs in its slab: the source and a word past it, and the parser's
/// tape at its bound (80 times the source). It is address space; what is resident is what the two write.
fn room(len: usize) -> usize {
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
