//! A document through the engine's file layer (decision 0686, B4; *0686: Operations* §5, §9; P§12 item 1; B§4's
//! tape port, B§7).
//!
//! A document is named by its path relative to the root, and answered as the engine answers a path today:
//!
//! - the observer's digest step (`Observer::digest_step`) reuses the answer it holds while the file's stat holds,
//!   under the racy rule; no byte is read then;
//! - a file it must read is read once, by the observer's placed read (`DigestRead::read_placed`), into a room of the
//!   document's tape block (`place_in`), and its digest recorded (`Observer::digest_read`);
//! - the rr parser writes the tape into the same block's room (`rerail_rr_syntax::tape_in`, `InBlock`), where it
//!   lies; the block is then frozen: the document's source and tape, which an operation reads as a slot (`intern`,
//!   `k0_pass`) or through a reference to it;
//! - the source and the tape are each a blob (P§11.4: "source text, tapes: blobs, by content"), housed from their
//!   placed bytes (`Blobs::house`, P§12 item 1: `house(bytes: Placed) -> BlobId`, "takes bytes where they lie", as
//!   the store's objects are), named by the sha256 of their bytes and read where they lie (`Blobs::get`), never
//!   versioned: changed bytes are a new blob. Each is its own blob, so a new source whose tape is the same bytes as
//!   an earlier document's (the tape holds the shape, not the text) has its tape where that one's lies: a document's
//!   source and tape may lie in two blocks, and every reader takes the two places (`reads`).
//!
//! The tape port's bistate (`Doc`) is ∅ → tape on the first answer, the identity while the digest is unchanged, and
//! the old tape → the new one when it changed: the file is the grain. A file that does not parse, is not UTF-8, or
//! cannot be read is refused, naming its path. Nothing here reads a file itself.

use crate::dir::{BlockId, Directory};
use crate::tape::Text;
use rerail_blocks::block::kind;
use rerail_blocks::maps::InBlock;
use rerail_observe::content_store::observe::{self, DigestRead, DigestStep};
use rerail_blocks::maps::Placed as _;
use rerail_blocks::region::{Frozen, Mutable};
use rerail_rr_syntax::tape_format::home;
use sha2::{Digest as _, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// A blob's name: its bytes' sha256 (P§12 item 1).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct BlobId(pub [u8; 32]);

impl BlobId {
    /// Its 32 bytes as four words: how a row names it (P§11: "a relation refers to a blob by its id").
    pub fn words(&self) -> [i64; 4] {
        std::array::from_fn(|i| i64::from_le_bytes(self.0[8 * i..8 * i + 8].try_into().expect("eight bytes")))
    }

    /// The blob a row's four words name.
    pub fn of_words(w: &[i64]) -> BlobId {
        let mut b = [0u8; 32];
        for (i, x) in w.iter().take(4).enumerate() {
            b[8 * i..8 * i + 8].copy_from_slice(&x.to_le_bytes()); // copy: not block data (a name's bytes)
        }
        BlobId(b)
    }
}

/// Where bytes lie: a frozen block of the directory, an offset in it and a length (P§12's `Placed`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Placed {
    pub block: BlockId,
    pub at: usize,
    pub len: usize,
}

/// The blobs, by name, each where its bytes lie (P§12 item 1's `Blobs`): housed from placed bytes, never copied.
#[derive(Default)]
pub struct Blobs {
    by_id: HashMap<BlobId, Placed>,
}

impl Blobs {
    /// Where blob `id`'s bytes lie.
    pub fn get(&self, id: &BlobId) -> Option<Placed> {
        self.by_id.get(id).copied()
    }

    /// Houses the bytes at `p` where they lie, named by their sha256 (`house(bytes: Placed) -> BlobId`): the name,
    /// and whether they are new. A second of the same bytes is the first; its place is not kept.
    pub fn house(&mut self, dir: &Directory, p: Placed) -> Result<(BlobId, bool), String> {
        if !dir.is_frozen(p.block) {
            return Err(format!("a blob's bytes in {}, a block not frozen (a blob is never versioned)", p.block));
        }
        let id = {
            let l = dir.read("house", &[p.block])?;
            BlobId(Sha256::digest(&l.block(0)[p.at..p.at + p.len]).into())
        };
        if self.by_id.contains_key(&id) {
            return Ok((id, false));
        }
        self.by_id.insert(id, p);
        Ok((id, true))
    }

    /// How many it holds.
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
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

    /// The observer's counts (its stats, reuses, files read and hashed).
    pub fn counts(&self) -> observe::Counts {
        observe::with(Some(&self.store), |o| o.counts.clone())
    }

    /// Document `path` (relative to the root): its tape port, the tape housed in `blobs`, a tape block made in `dir`
    /// for a tape the parser writes.
    pub fn answer(&mut self, path: &str, blobs: &mut Blobs, dir: &mut Directory) -> Result<Doc, String> {
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
        let (block, b) = tape_block(dir, size);
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
            dir.release(block);
            return Err(format!("{path}: could not be read"));
        };
        self.reads += 1;
        if let Some((k, b)) = kept
            && k == digest
        {
            // Read again (its stat changed, or a check), the same bytes: the identity.
            dir.release(block);
            return Ok(doc(Some(b), b));
        }
        let id = house(dir, blobs, block, b, source).map_err(|e| format!("{path}: {e}"))?;
        self.kept.insert(path.to_string(), (digest, id));
        Ok(doc(kept.map(|k| k.1), id))
    }
}

/// A document of a source in hand (a test's, the bootstrap's own), in a tape block of its own, as a read places one.
pub fn doc_of(dir: &mut Directory, blobs: &mut Blobs, src: &str) -> Result<DocIds, String> {
    let (block, b) = tape_block(dir, src.len());
    let home = place_in(&b, src.len(), &mut |out| {
        out.copy_from_slice(src.as_bytes()); // copy: load (a source in hand, written into its room)
        Ok(())
    })
    .map_err(|e| e.to_string())?;
    house(dir, blobs, block, b, home)
}

/// A document's source in its tape block's room: the tape parsed from it into the same block, the block frozen, and
/// the two housed as blobs. The block is let go when it holds nothing new (both blobs housed already) or on an error.
fn house(dir: &mut Directory, blobs: &mut Blobs, block: BlockId, b: Arc<rerail_blocks::block::Block>, source: home::Home) -> Result<DocIds, String> {
    let placed = parse(block, b, source).and_then(|(s, t)| {
        dir.freeze(block)?;
        let (source, new_s) = blobs.house(dir, s)?;
        let (tape, new_t) = blobs.house(dir, t)?;
        Ok((DocIds { source, tape }, new_s || new_t))
    });
    match placed {
        Ok((ids, new)) => {
            if !new {
                dir.release(block);
            }
            Ok(ids)
        }
        Err(e) => {
            dir.release(block);
            Err(e)
        }
    }
}

/// A document's source in its tape block's room, and its tape parsed into the same block: where each lies. Every
/// handle on the block is let go here (the block holds no free list: letting a region go frees nothing), so the block
/// can be frozen.
fn parse(block: BlockId, b: Arc<rerail_blocks::block::Block>, source: home::Home) -> Result<(Placed, Placed), String> {
    let src_at = source.room().and_then(|r| r.as_any().downcast_ref::<SourceRoom>()).map(|r| r.region.region().at()).ok_or("a source placed in its tape block")?;
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

/// A document's tape block, for a source of about `len` bytes: a sparse reservation its source and the parser's
/// writes reach into (nothing is resident until written).
fn tape_block(dir: &mut Directory, len: usize) -> (BlockId, Arc<rerail_blocks::block::Block>) {
    let reserve = (80 * len).max(1 << 20).next_power_of_two();
    let id = dir.make_capped(kind::TAPE, 8, reserve, reserve);
    (id, dir.block(id).expect("just made"))
}

#[cfg(test)]
#[path = "files_tests.rs"]
pub(crate) mod tests;
