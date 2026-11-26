//! The symbols (decision 0686, B5): every string a program names (a relation, a variable, a kind, a string constant)
//! as an id, in one atoms block (decision 0401), the one program global every level shares: K-form rows and facts
//! hold ids, and an id names the same string at every level and in every run.
//!
//! The atoms block is a directory block, written by two operations only (read-write): `intern`, which adds a changed
//! document's identifiers and strings, and `intern_texts`, which adds the texts a rule's functions answered with. Everything else reads it where it lies: `find` and `name` here, the views
//! through a document's symbols block, never interning.

use crate::dir::{BlockId, Directory};
use rerail_blocks::atoms::{atoms_bytes, count_in, find_in, lay_out_in, name_in};
use rerail_blocks::block::kind;
use rerail_blocks::maps::hash_bytes;
use rerail_rr_syntax::tape::{FIELD_NAMES, KIND_NAMES};

/// How many atoms, and string bytes, the block has room for (address space only: nothing is resident until written).
const CAP: u32 = 1 << 20;
const BYTES_CAP: u64 = 64 << 20;

/// The atoms block, by its directory id. Ids start at 1, so 0 is never a symbol.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Atoms(pub BlockId);

impl Atoms {
    /// An empty atoms block.
    pub fn new(dir: &mut Directory) -> Atoms {
        let reserve = atoms_bytes(CAP, BYTES_CAP) + (1 << 20);
        let id = dir.make_capped(kind::ATOMS, 8, reserve, reserve);
        lay_out_in(&dir.block(id).expect("just made"), 1, CAP, BYTES_CAP);
        Atoms(id)
    }

    /// The store's atoms block, in the file at `path` (decision 0401: the store's atoms, which never give an id out
    /// again): the file's block as it lies when there is one, else a new one, laid out empty. The same text keeps its
    /// id in every later process.
    pub fn open(dir: &mut Directory, path: &std::path::Path) -> Result<Atoms, String> {
        let reserve = atoms_bytes(CAP, BYTES_CAP) + (1 << 20);
        let (id, new) = dir.file_block(path, kind::ATOMS, 8, reserve)?;
        if new {
            lay_out_in(&dir.block(id).expect("just made"), 1, CAP, BYTES_CAP);
        }
        Ok(Atoms(id))
    }

    /// The id of `s`, when it has one.
    pub fn find(self, dir: &Directory, s: &str) -> Option<i64> {
        let l = dir.read("atoms", &[self.0]).ok()?;
        find(l.block(0), s)
    }

    /// The string of `id`, for messages and tests.
    pub fn name(self, dir: &Directory, id: i64) -> String {
        let Ok(l) = dir.read("atoms", &[self.0]) else { return format!("#{id}") };
        // copy: not block data (a message's text)
        u32::try_from(id).ok().and_then(|i| name_in(l.block(0), i)).map_or_else(|| format!("#{id}"), str::to_owned)
    }

    /// How many atoms it holds.
    pub fn count(self, dir: &Directory) -> u32 {
        dir.read("atoms", &[self.0]).map_or(0, |l| count_in(l.block(0)))
    }
}

/// The id of `s` in the atoms block `atoms` (its bytes, where they lie), when it has one.
pub fn find(atoms: &[u8], s: &str) -> Option<i64> {
    find_in(atoms, s, hash_bytes(s.as_bytes())).map(i64::from)
}

/// The string of symbol `id` in the atoms block `atoms` (its bytes, where they lie): a name for a shape's choice and
/// for messages (`#id` when it has none).
pub fn name_of(atoms: &[u8], id: i64) -> String {
    // A node's name (`tape::name`, negative): the node it names, in its document.
    if id < 0 {
        let at = -id;
        return format!("<node {} of document {}>", at & 0xffff_ffff, at >> 32);
    }
    // copy: not block data (a name's text)
    u32::try_from(id).ok().and_then(|i| rerail_blocks::atoms::name_in(atoms, i)).map_or_else(|| format!("#{id}"), str::to_owned)
}

/// The kinds' and fields' names as symbols, found once in the atoms (`intern` adds them), so a view compares ids.
pub struct Names {
    kind: Vec<i64>,
    field: Vec<i64>,
}

impl Names {
    pub fn new(atoms: &[u8]) -> Names {
        let ids = |names: &[&str]| names.iter().map(|n| find(atoms, n).unwrap_or(0)).collect();
        Names { kind: ids(KIND_NAMES), field: ids(FIELD_NAMES) }
    }

    pub fn kind(&self, k: u16) -> i64 {
        self.kind.get(k as usize).copied().unwrap_or(0)
    }

    pub fn field(&self, f: u16) -> i64 {
        self.field.get(f as usize).copied().unwrap_or(0)
    }
}
