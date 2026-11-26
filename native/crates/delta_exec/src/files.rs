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
//!   lies: the block is the document's source and tape, which an operation reads as a slot (`intern`, `k0_pass`);
//! - the document, its source and its tape, is a blob named by its content (`Blobs`, P§12 item 1), never versioned:
//!   changed bytes are a new blob.
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

/// A blob's name: its content's sha256 (P§12 item 1).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct BlobId(pub [u8; 32]);

/// A document as a blob: its tape block, the source and the tape in it (each by offset and length), and its text, which
/// reads both where they lie.
pub struct DocBlob {
    pub block: BlockId,
    pub src: (usize, usize),
    pub tape: (usize, usize),
    pub text: Arc<Text>,
}

/// The documents as blobs, by name. A blob housed twice is one.
#[derive(Default)]
pub struct Blobs {
    docs: HashMap<BlobId, DocBlob>,
}

impl Blobs {
    /// Blob `id`, read where it lies.
    pub fn get(&self, id: &BlobId) -> Option<&DocBlob> {
        self.docs.get(id)
    }

    /// Houses `doc` where its bytes lie, named by its content: its source's and its tape's bytes. A second of the
    /// same content is the first, and its block is let go.
    pub fn house(&mut self, doc: DocBlob, dir: &mut Directory) -> BlobId {
        let mut h = Sha256::new();
        h.update((doc.text.src.as_str().len() as u64).to_le_bytes());
        h.update(doc.text.src.as_str().as_bytes());
        h.update(doc.text.tape.as_bytes());
        let id = BlobId(h.finalize().into());
        if self.docs.contains_key(&id) {
            dir.release(doc.block);
        } else {
            self.docs.insert(id, doc);
        }
        id
    }

    /// How many it holds.
    pub fn len(&self) -> usize {
        self.docs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }
}

/// A document's tape port: its path relative to the root, the module its answer names, and its bistate, from the
/// tape the last answer gave (`None`: a cold run) to this one's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Doc {
    pub path: String,
    pub module: String,
    pub before: Option<BlobId>,
    pub after: BlobId,
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
    kept: HashMap<String, ([u8; 32], BlobId)>,
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
        let doc = |before: Option<BlobId>, after| Doc { path: path.to_string(), module: module_of(path), before, after };
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
        let parsed = parse(block, b, source).map_err(|e| format!("{path}: {e}"));
        let d = match parsed {
            Ok(d) => d,
            Err(e) => {
                dir.release(block);
                return Err(e);
            }
        };
        let id = blobs.house(d, dir);
        self.kept.insert(path.to_string(), (digest, id));
        Ok(doc(kept.map(|k| k.1), id))
    }
}

/// A document of a source in hand (a test's, the bootstrap's own), in a tape block of its own, as a read places one.
pub fn doc_of(dir: &mut Directory, src: &str) -> Result<DocBlob, String> {
    let (block, b) = tape_block(dir, src.len());
    let home = place_in(&b, src.len(), &mut |out| {
        out.copy_from_slice(src.as_bytes()); // copy: load (a source in hand, written into its room)
        Ok(())
    })
    .map_err(|e| e.to_string())?;
    parse(block, b, home)
}

/// A document's source in its tape block's room: the tape parsed from it, written into the same block.
fn parse(block: BlockId, b: Arc<rerail_blocks::block::Block>, source: home::Home) -> Result<DocBlob, String> {
    let src_at = source.room().and_then(|r| r.as_any().downcast_ref::<SourceRoom>()).map(|r| r.region.region().at()).ok_or("a source placed in its tape block")?;
    let src = home::Text::new(source).map_err(|e| format!("not UTF-8: {e}"))?;
    let text = Text::parse_in(src, Arc::new(InBlock(b.clone())))?;
    let tape = match text.tape.home() {
        home::Home::Own(v) => {
            let (tb, at) = v.place().ok_or("a tape written in its block")?;
            assert!(Arc::ptr_eq(tb, &b), "the tape in its document's block");
            (at, text.tape.as_bytes().len())
        }
        home::Home::Placed(_) => return Err("a tape the parser wrote is its writer's".into()),
    };
    Ok(DocBlob { block, src: (src_at, text.src.as_str().len()), tape, text: Arc::new(text) })
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
