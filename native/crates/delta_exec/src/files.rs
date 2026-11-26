//! A document through the engine's file layer (decision 0686, B4; *0686: Operations* §5, §9; P§12 item 1; B§4's
//! tape port, B§7).
//!
//! A document is named by its path relative to the root, and answered as the engine answers a path today:
//!
//! - the observer's digest step (`Observer::digest_step`) reuses the answer it holds while the file's stat holds,
//!   under the racy rule; no byte is read then;
//! - a file it must read is read once, by the observer's placed read (`DigestRead::read_placed`), into the home the
//!   slabs' placer makes (`rerail_slabs::placed::placer`: a slab's room, or a vector where there is no device), and
//!   its digest recorded (`Observer::digest_read`);
//! - the rr parser writes the tape into a tape block's room (`rerail_rr_syntax::tape_in`, `InBlock`), where it lies;
//! - the document, its source and its tape, is a blob named by its content (`Blobs`, P§12 item 1), never versioned:
//!   changed bytes are a new blob.
//!
//! The tape port's bistate (`Doc`) is ∅ → tape on the first answer, the identity while the digest is unchanged, and
//! the old tape → the new one when it changed: the file is the grain. A file that does not parse, is not UTF-8, or
//! cannot be read is refused, naming its path. Nothing here reads a file itself.

use crate::dir::Directory;
use crate::tape::Text;
use rerail_blocks::block::kind;
use rerail_blocks::maps::InBlock;
use rerail_observe::content_store::observe::{self, DigestRead, DigestStep};
use rerail_rr_syntax::tape_format::home;
use sha2::{Digest as _, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// A blob's name: its content's sha256 (P§12 item 1).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct BlobId(pub [u8; 32]);

/// The documents as blobs: each its source and its tape, where they lie, by its name. A blob housed twice is one.
#[derive(Default)]
pub struct Blobs {
    docs: HashMap<BlobId, Arc<Text>>,
}

impl Blobs {
    /// Blob `id`, read where it lies.
    pub fn get(&self, id: &BlobId) -> Option<&Arc<Text>> {
        self.docs.get(id)
    }

    /// Houses `text` where its bytes lie, named by its content: its source's and its tape's bytes.
    pub fn house(&mut self, text: Text) -> BlobId {
        let mut h = Sha256::new();
        h.update((text.src.as_str().len() as u64).to_le_bytes());
        h.update(text.src.as_str().as_bytes());
        h.update(text.tape.as_bytes());
        let id = BlobId(h.finalize().into());
        self.docs.entry(id).or_insert_with(|| Arc::new(text));
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
        let (digest, source) = match read {
            Some(r) => {
                let home = r.read_placed(&rerail_slabs::placed::placer).ok_or_else(|| format!("{path}: could not be read"))?;
                self.reads += 1;
                let hashed = DigestRead::hashed(&home);
                let d = observe::with(Some(&self.store), |o| o.digest_read(r, Some(hashed))).ok_or_else(|| format!("{path}: could not be read"))?;
                (d, home)
            }
            None => {
                let home = rerail_observe::git_workspace::snapshot::read_placed(&abs, &rerail_slabs::placed::placer).map_err(|e| format!("{path}: {e}"))?;
                self.reads += 1;
                (DigestRead::hashed(&home).1, home)
            }
        };
        if let Some((k, b)) = kept
            && k == digest
        {
            // Read again (its stat changed, or a check), the same bytes: the identity.
            return Ok(doc(Some(b), b));
        }
        let src = home::Text::new(source).map_err(|e| format!("{path}: not UTF-8: {e}"))?;
        let room = tape_room(dir, src.as_str().len());
        let text = Text::parse_in(src, room).map_err(|e| format!("{path}: {e}"))?;
        let id = blobs.house(text);
        self.kept.insert(path.to_string(), (digest, id));
        Ok(doc(kept.map(|k| k.1), id))
    }
}

/// A tape block's room for a source of `len` bytes: a sparse reservation the parser's writes reach into as the tape
/// grows (nothing is resident until written). The block lives while its tape does.
fn tape_room(dir: &mut Directory, len: usize) -> Arc<dyn rerail_rr_syntax::tape_format::VectorRoom> {
    let reserve = (64 * len).max(1 << 20).next_power_of_two();
    let id = dir.make_capped(kind::TAPE, 8, reserve, reserve);
    let b = dir.block(id).expect("just made");
    dir.release(id);
    Arc::new(InBlock(b))
}

#[cfg(test)]
#[path = "files_tests.rs"]
mod tests;
