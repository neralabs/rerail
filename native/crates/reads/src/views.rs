//! The views' entries' layout (decision 0353 step 4c), which the facts write and every reader reads where it lies: a
//! document's entry (`ViewDoc`), a batch's directory of its documents (`ViewDir`, `ViewRow`, `ViewHome`), and the
//! directory's searches on the host, written directly. The views' reads in CLIF are `rerail_clif::view_reads`'s,
//! which re-exports these.

use crate::ids::Run;

/// A document's entry for its views' reads (`ViewDoc` in the kernels).
#[repr(C)]
#[derive(Debug, Default)]
pub struct ViewDoc {
    /// The document's key: an id's high 32 bits.
    pub key: u32,
    /// Its dialect's tape tag.
    pub tag: u32,
    /// 1 when ids are local ids (`RERAIL_NODE_IDS=preorder`: no granule table).
    pub preorder: u32,
    /// The projection's numbered nodes.
    pub nodes: u32,
    /// The projection's numbering (`ids::Numbering`, one).
    pub numbering: Run,
    /// By the tape's code (its table's number, `tape_number`): the kind atom its `kind` column holds (u32s, its
    /// name's canonical code's), and the field atom (`tag << 16 | field`).
    pub kinds: Run,
    pub fields: Run,
    /// Each numbered node's place among its parent's children (u32s, by local id): the walk's, derived once
    /// (`Projected::idx`, decision 0353 step 3), as the tape holds no sibling's place.
    pub places: Run,
    /// core.rerail's positions past the nodes (`Locs`), for the `Loc` views: the tokens, then where the text is the
    /// document's own (1) the whole text and each line, then the end points (`ends`, one a node, or none).
    pub tokens: u32,
    pub own_text: u32,
    pub ends: u32,
    pub source_len: u32,
    /// A guest's placement in its host (`Placement`): 0 none (a file's own tree), `PLACED_COPIED` (host byte = guest
    /// byte + `delta`, clamped to `within`), `PLACED_AT` (every span at `within`).
    pub placement: u32,
    /// The key of the file the document is part of (decision 0431 §4): its own, or a program's file's. Its
    /// `docof`; its ids keep `key`.
    pub file: u32,
    pub delta: i64,
    pub within_lo: u32,
    pub within_hi: u32,
}
const _: () = assert!(std::mem::size_of::<ViewDoc>() == 16 + 16 * 4 + 16 + 24);

/// `ViewDoc::placement`'s kinds.
pub const PLACED_COPIED: u32 = 1;
pub const PLACED_AT: u32 = 2;

/// A document of a batch, as an executor finds its views' rows (decision 0353 step 4d, and step 3 on the CPU): its
/// entry's offset in the batch's block, its tape's and its lines' places (a home of the directory's, `ViewHome`, and
/// an offset in it), its tape's view (the reads' own), and its first row in each view (`VIEWS`). Nothing in it is an
/// address: the host and the device read the same row.
///
/// The view's 16z cache offset (`TapeView::side`, the cache's address less the tape's) is the device's, as the
/// kernels read the view where it lies; the host's reads take the view with their own, from the same homes
/// (`ViewDir::cache`).
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct ViewRow {
    pub entry: u64,
    /// Offsets in their homes (`homes`: the tape's, then the lines').
    pub tape: u64,
    pub lines: u64,
    pub homes: [u32; 2],
    pub rows: [u32; VIEWS.len()],
    pub view: crate::view::TapeView,
}
const _: () = assert!(std::mem::size_of::<ViewRow>() == (8 + 8 + 8 + 8 + 4 * VIEWS.len() + std::mem::size_of::<crate::view::TapeView>()).next_multiple_of(8));

/// Where a home of a directory's tapes and lines lies for each executor: a slab's (or a tape's vector's) host
/// address, and its device address (`gpu`; 0 where the device has none: no device, or the home is a vector of the
/// host's). A row's tape is at its home's base plus its offset, the executor's base.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct ViewHome {
    pub host: u64,
    pub gpu: u64,
}
const _: () = assert!(std::mem::size_of::<ViewHome>() == 16);

/// A batch's documents as its views' reads find them, in the batch's block and self-relative: its own offset in the
/// block (`at`: the block's start is the directory's address less it, whoever reads it), its documents' rows and its
/// homes (offsets in the block), how many of each, its 16z cache's home (its number + 1, 0 for none: a block of
/// its own, which both executors' reads share and write), and its documents' keys (`keys`, an offset in the block:
/// `ndocs` `u32`s in row order, each its row's entry's key), which `doc_of_key` searches alone. The first `nbatch` are
/// the batch's documents, in key order; past them, a chunk's tape values admitted inside its fixpoint
/// (`values::Layered`), in the order admitted, which a key's search scans.
#[repr(C)]
#[derive(Debug, Default)]
pub struct ViewDir {
    pub at: u64,
    pub ndocs: u32,
    pub nhomes: u32,
    pub rows: u64,
    pub homes: u64,
    pub cache: u32,
    pub nbatch: u32,
    pub keys: u64,
}
const _: () = assert!(std::mem::size_of::<ViewDir>() == 48);

/// No document (`ViewDir::doc_of_key`).
pub const NO_DOC: u32 = u32::MAX;

/// The directory's searches on the host, written directly beside their CLIF (`dir_doc_of_row`, `dir_doc_of_key`,
/// decision 0353 step 3 on the CPU, step 2), which the kernels and the JIT read: each takes a memo, the document its
/// caller found last, tried before the bisection. A stale memo costs a comparison, never a wrong document.
impl ViewDir {
    /// The batch's block's start: the directory's address less its offset in it.
    fn block(&self) -> *const u8 {
        (self as *const Self).cast::<u8>().wrapping_sub(self.at as usize)
    }
    /// Its rows.
    ///
    /// # Safety
    /// The directory lies in its block as `build_dir` wrote it, and the block is alive.
    pub unsafe fn rows(&self) -> &[ViewRow] {
        // SAFETY: the caller's.
        unsafe { std::slice::from_raw_parts(self.block().add(self.rows as usize).cast::<ViewRow>(), self.ndocs as usize) }
    }
    /// Its homes.
    ///
    /// # Safety
    /// As `rows`.
    pub unsafe fn homes(&self) -> &[ViewHome] {
        // SAFETY: the caller's.
        unsafe { std::slice::from_raw_parts(self.block().add(self.homes as usize).cast::<ViewHome>(), self.nhomes as usize) }
    }
    /// Row `m`'s entry.
    ///
    /// # Safety
    /// As `rows`, and `m` below `ndocs`.
    pub unsafe fn entry(&self, m: u32) -> &ViewDoc {
        // SAFETY: the caller's.
        unsafe { &*self.block().add(self.rows()[m as usize].entry as usize).cast::<ViewDoc>() }
    }
    /// The document whose rows of view `k` (`VIEWS`) hold row `r`: the last whose first row is at or before it (an
    /// empty one starts where the next does); 0 for a directory of none.
    ///
    /// # Safety
    /// As `rows`.
    pub unsafe fn doc_of_row(&self, k: u32, r: u32, memo: u32) -> u32 {
        // SAFETY: the caller's.
        let rows = unsafe { self.rows() };
        let first = |m: u32| rows[m as usize].rows[k as usize];
        let n = self.ndocs;
        if memo < n && first(memo) <= r && (memo + 1 == n || first(memo + 1) > r) {
            return memo;
        }
        crate::ids::bisect_n(n, |m| first(m) <= r).saturating_sub(1)
    }
    /// Its documents' keys, in row order (`keys`).
    ///
    /// # Safety
    /// As `rows`.
    pub unsafe fn keys(&self) -> &[u32] {
        // SAFETY: the caller's.
        unsafe { std::slice::from_raw_parts(self.block().add(self.keys as usize).cast::<u32>(), self.ndocs as usize) }
    }
    /// The document of key `key`, or `NO_DOC`: memo `memo` tried first (a hit ends it), then the batch's documents
    /// by a lower bound of a fixed count of steps (`lower_bound`) and the values' by a scan, the first that is `key`.
    /// Past the memo, the steps are the directory's alone and each a select: a group's lanes each searching a key go
    /// in step (the CLIF's `dir_doc_of_key`).
    ///
    /// # Safety
    /// As `rows`.
    pub unsafe fn doc_of_key(&self, key: u32, memo: u32) -> u32 {
        // SAFETY: the caller's.
        let keys = unsafe { self.keys() };
        let (n, nbatch) = (self.ndocs, self.nbatch.min(self.ndocs));
        if memo < n && keys[memo as usize] == key {
            return memo;
        }
        let c = lower_bound(&keys[..nbatch as usize], key);
        if c < nbatch && keys[c as usize] == key {
            return c;
        }
        (nbatch..n).rev().fold(NO_DOC, |found, i| if keys[i as usize] == key { i } else { found })
    }
}

/// The first place in `keys` (ascending) whose key is at least `key` (`keys.len()` where none is), in a count of steps
/// `keys.len()` alone sets: each halves the span, its base moved by a select.
pub fn lower_bound(keys: &[u32], key: u32) -> u32 {
    let (mut base, mut len) = (0usize, keys.len());
    if len == 0 {
        return 0;
    }
    while len > 1 {
        let half = len / 2;
        base = if keys[base + half - 1] < key { base + half } else { base };
        len -= half;
    }
    (base + usize::from(keys[base] < key)) as u32
}

/// A view's number in a kernel's reads (`ViewDir`'s rows' and `vread`'s): `node`, `child`, `field`, `core_node`,
/// `line`, then core.rerail's places (`Loc`): `line`, `end_line`, `col`, `docof`; then `tape`, `ast_depth` and
/// `node_order`.
pub const VIEWS: [&str; 12] = ["view_node", "view_child", "view_field", "view_core", "view_line", "view_loc_line", "view_loc_end", "view_loc_col", "view_loc_docof", "view_tape", "view_depth", "view_order"];

/// A field's byte offset in the entry.
pub mod at {
    use super::ViewDoc;
    use std::mem::offset_of;
    pub const KEY: i32 = offset_of!(ViewDoc, key) as i32;
    pub const TAG: i32 = offset_of!(ViewDoc, tag) as i32;
    pub const PREORDER: i32 = offset_of!(ViewDoc, preorder) as i32;
    pub const NODES: i32 = offset_of!(ViewDoc, nodes) as i32;
    pub const NUMBERING: i32 = offset_of!(ViewDoc, numbering) as i32;
    pub const KINDS: i32 = offset_of!(ViewDoc, kinds) as i32;
    pub const FIELDS: i32 = offset_of!(ViewDoc, fields) as i32;
    pub const PLACES: i32 = offset_of!(ViewDoc, places) as i32;
    pub const TOKENS: i32 = offset_of!(ViewDoc, tokens) as i32;
    pub const OWN_TEXT: i32 = offset_of!(ViewDoc, own_text) as i32;
    pub const ENDS: i32 = offset_of!(ViewDoc, ends) as i32;
    pub const SOURCE_LEN: i32 = offset_of!(ViewDoc, source_len) as i32;
    pub const PLACEMENT: i32 = offset_of!(ViewDoc, placement) as i32;
    pub const FILE: i32 = offset_of!(ViewDoc, file) as i32;
    pub const DELTA: i32 = offset_of!(ViewDoc, delta) as i32;
    pub const WITHIN_LO: i32 = offset_of!(ViewDoc, within_lo) as i32;
    pub const WITHIN_HI: i32 = offset_of!(ViewDoc, within_hi) as i32;
}

/// The `node` view's columns (`Viewed::columns`): the document, the node's id, its kind, start and end.
pub const NODE_COLUMNS: u32 = 5;
/// `child`'s: the parent's id, the child's, its place; `field`'s: the parent's id, the field atom, the child's id;
/// `core_node`'s: the document, the tape node, the id.
pub const CHILD_COLUMNS: u32 = 3;
pub const FIELD_COLUMNS: u32 = 3;
pub const CORE_COLUMNS: u32 = 3;
/// `line`'s: the node's id, the line its start is on (from 1); each `Loc` view's: the node's or span's id, its place.
pub const LINE_COLUMNS: u32 = 2;
pub const LOC_COLUMNS: u32 = 2;
/// `tape`'s: the node's id, its parent's id, the numbered nodes under it, its depth; `ast_depth`'s: the node's id,
/// its depth; `node_order`'s: the node's id, its order key (the document's key above it).
pub const TAPE_COLUMNS: u32 = 4;
pub const DEPTH_COLUMNS: u32 = 2;
pub const ORDER_COLUMNS: u32 = 2;


/// No row: a parent probe's end (`view_child_first`, `view_child_next`, `view_number`).
pub const NO_ROW: u32 = u32::MAX;
