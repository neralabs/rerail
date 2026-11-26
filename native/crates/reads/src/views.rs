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
    /// core.rerail's positions past the nodes (`Locs`), for the `Loc` views: the tokens, then where the text is the
    /// document's own (1) the whole text and each line, then the end points (`ends`, one a node, or none).
    pub tokens: u32,
    pub own_text: u32,
    pub ends: u32,
    pub source_len: u32,
    /// A guest's placement in its host (`Placement`): 0 none (a file's own tree), `PLACED_COPIED` (host byte = guest
    /// byte + `delta`, clamped to `within`), `PLACED_AT` (every span at `within`), `PLACED_SEGMENTS` (an embedder's,
    /// by its `segments`), `PLACED_TABLE` (a string's normalized pieces: each position's host span in `placed`).
    pub placement: u32,
    /// The key of the file the document is part of (decision 0431 §4): its own, or a program's file's. Its
    /// `docof`; its ids keep `key`.
    pub file: u32,
    pub delta: i64,
    pub within_lo: u32,
    pub within_hi: u32,
    /// `PLACED_TABLE`'s: by local id (a node, then each position past them), the host span `start << 32 | end` the
    /// guest's placement gives its span (`Placement::host`; u64s). A string's normalized pieces, which no word maps:
    /// its spans placed once, as the host places them.
    pub placed: Run,
    /// The tape's rows' derived columns (`tape_rows`), each made on a program's first read of its relation, empty
    /// before: each row's gap's atom of `structural_before` and `structural_after` (u32s, by row); `rr_token_named`'s
    /// parent and place among its parent's tape children (`[u32; 2]`s, by row); each node's place in `visit_index`'s
    /// order and its `tree_digest` atom (u32s, by local id).
    pub gaps: [Run; 2],
    pub named: Run,
    pub visits: Run,
    pub digests: Run,
    /// By the tape's code (`tape_number`), the atom of its kind's name (u32s): the `op` view's `s` of an anonymous
    /// child of that code (`view_op`), the host's and the kernels' one table. Interned once where a batch reads `op`;
    /// empty where it does not.
    pub names: Run,
    /// `PLACED_SEGMENTS`' segments (`SEGMENT_WORDS` u32s each, in program order): an embedder's program's map.
    pub segments: Run,
    /// Which place a numbered node's is, read in its tape's auxiliary index (decision 0656 step 4): `PLACE_NAMED`
    /// (`tape_place`, core.rerail's shape) or `PLACE_PAST_COMMENTS` (`tape_place_past_comments`, T's). An outermost
    /// anchor's is its rank among the anchors (`ids` item `ANCHOR_ITEM`).
    pub place: u32,
    pub pad: u32,
}
const _: () = assert!(std::mem::size_of::<ViewDoc>() == 16 + 16 * 2 + 16 + 24 + 16 * 7 + 8);

/// A language's place in the codes' directory (decision 0697, `rerail_tape_format::codes_dir`, in the binary): by
/// tape tag, the offsets from the directory's first byte of its codes' kind atoms and field atoms (u32s, by the tape's
/// code: its table's number, `tape_number`); 0 for a tag with no table. The `node` view's kind is the kind atom its
/// `kind` column holds (its name's canonical code's), the `field` view's the field atom (`tag << 16 | field`).
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct CodesLang {
    pub kinds: u32,
    pub fields: u32,
}

/// Run `k` (0 kinds, 1 fields) of language `tag` in the codes' directory at `dir` (4-aligned): its `CodesLang` at the
/// tag's place, its run's u32s at its offset, as many as the kinds' run holds (each run as long); empty for a tag with
/// no table.
///
/// # Safety
/// `dir` is a codes' directory's first byte (`rerail_tape_format::codes_dir`, or a home of one), alive for `'a`.
pub unsafe fn code_atoms<'a>(dir: *const u8, tag: u8, k: usize) -> &'a [u32] {
    const LANG: usize = std::mem::size_of::<CodesLang>();
    // SAFETY: the tag's place and its run, inside the directory (the caller's).
    unsafe {
        let lang = dir.add(usize::from(tag) * LANG).cast::<CodesLang>().read_unaligned();
        if lang.kinds == 0 {
            return &[];
        }
        let n = (lang.fields - lang.kinds) as usize / 4;
        let run = dir.add(if k == 0 { lang.kinds } else { lang.fields } as usize).cast::<u32>();
        assert!(run.is_aligned(), "a codes' directory 4-aligned");
        std::slice::from_raw_parts(run, n)
    }
}

/// `ViewDoc::place`'s kinds.
pub const PLACE_NAMED: u32 = 1;
pub const PLACE_PAST_COMMENTS: u32 = 2;
/// The rank index's item of an anchored projection's anchors (`Item::Anchor`).
pub const ANCHOR_ITEM: i64 = 11;

/// `ViewDoc::placement`'s kinds.
pub const PLACED_COPIED: u32 = 1;
pub const PLACED_AT: u32 = 2;
/// An embedder's program (`Placement::Segments`): a span stands where `SegmentMap::file_ranges(..).primary()` puts it.
pub const PLACED_SEGMENTS: u32 = 3;
/// A segment's u32s: its program range, its host range, and 1 where it is copied (`SegmentMap::words`).
pub const SEGMENT_WORDS: i64 = 5;
/// A string's normalized pieces (`Placement::Pieces`): each position's host span in `placed`.
pub const PLACED_TABLE: u32 = 4;

/// A document of a batch, as an executor finds its views' rows (decision 0353 step 4d, and step 3 on the CPU). The
/// search layer, made with the batch from its documents' keys and counts alone (none made): its key and its first row
/// in each view (`ROWS`). The reads layer, written when the batch's views are read in place: its entry's offset in
/// the batch's block, its tape's and its lines' places (a home of the directory's, `ViewHome`, and an offset in it),
/// and its tape's view (the reads' own). Nothing in it is an address: the host and the device read the same row.
///
/// The view's 16z cache offset (`TapeView::side`, the cache's address less the tape's) is the device's, as the
/// kernels read the view where it lies; the host's reads take the view with their own, from the same homes
/// (`ViewDir::cache`).
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct ViewRow {
    pub entry: u64,
    /// Offsets in their homes (`homes`: the tape's, the lines', then the source's).
    pub tape: u64,
    pub lines: u64,
    /// The document's own source text (a guest's its own program, not its host's): its first byte in home
    /// `homes[2]`, where a node's text is read in place (`view_text_span`, decision 0287 §2).
    pub source: u64,
    pub homes: [u32; 3],
    pub key: u32,
    pub rows: [u32; ROWS],
    pub view: crate::view::TapeView,
    /// The tape's length in bytes from `tape`: the host's reads read it as a slice (`rerail_tape_format::view_reads`).
    pub bytes: u32,
}
const _: () = assert!(std::mem::size_of::<ViewRow>() == (8 + 8 + 8 + 8 + 12 + 4 + 4 * ROWS + std::mem::size_of::<crate::view::TapeView>() + 4).next_multiple_of(8));

/// A row's first rows: one per view of `VIEWS`, then the views no kernel reads, the host's alone: `child`'s sibling
/// view (`SIBLING`), the token order's four (`TOK`), `spread` (`SPREAD`), `end_point` (`END`) and `text` (`TEXT`: its
/// nodes with text, then its positions with text), and the other relations of the tape's answers (`TAPE`, the
/// engine's `tape_rows`). Every view's rows are found in the directory, whoever reads them.
pub const ROWS: usize = TAPE + TAPE_RELATIONS;
/// `op`'s rows (each numbered node's anonymous children, `Item::Op`): `view_op`'s, in `VIEWS`.
pub const OP: usize = 12;
const _: () = assert!(matches!(VIEWS[OP].as_bytes(), b"view_op"));
pub const SIBLING: usize = VIEWS.len();
/// The token order's four relations' (`tokens::TokRel`'s order; the engine's `tape_rows`): `TOK + rel`.
pub const TOK: usize = SIBLING + 1;
pub const SPREAD: usize = TOK + 4;
pub const END: usize = SPREAD + 1;
/// `text`'s rows (decision 0656 step 4: a view, no row made).
pub const TEXT: usize = END + 1;
/// The first of the tape's other relations' (`tape_rows`): `structural_before`, `structural_after`,
/// `rr_token_named`, `visit_index`, `tree_digest` and `doc`, in that order.
pub const TAPE: usize = TEXT + 1;
/// How many relations `TAPE` starts.
pub const TAPE_RELATIONS: usize = 6;

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
/// its own, which both executors' reads share and write), its documents' keys (`keys`, an offset in the block:
/// `ndocs` `u32`s in row order, each its row's entry's key), which `doc_of_key` searches alone, and each view's rows
/// in all (`ROWS`' order; with `counted`, once the rows' first rows are written). The first `nbatch` documents are
/// the batch's, in key order; past them, a chunk's tape values admitted inside its fixpoint (`values::Layered`,
/// `views_over`), in the order admitted, which a key's search scans.
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
    pub ends: [u32; ROWS],
    pub counted: u32,
    /// The codes' directory's home (decision 0697): the binary's, at its host address and the device's.
    pub codes: u32,
}
const _: () = assert!(std::mem::size_of::<ViewDir>() == (48 + 4 * ROWS + 8).next_multiple_of(8));

/// No document (`ViewDir::doc_of_key`).
pub const NO_DOC: u32 = u32::MAX;

/// The directory's searches on the host, written directly beside their CLIF (`dir_doc_of_row`, `dir_doc_of_key`,
/// decision 0353 step 3 on the CPU, step 2), which the kernels and the JIT read: each takes a memo, the document its
/// caller found last, tried before the bisection. A stale memo costs a comparison, never a wrong document.
///
/// They read the search layer through the directory's address alone, field by field: the reads layer of the same
/// rows is written while they run (`build_dir`), so no reference to a row is made.
impl ViewDir {
    /// The batch's block's start: the directory's address less its offset in it.
    ///
    /// # Safety
    /// `this` is a directory in its block, which is alive.
    unsafe fn block_of(this: *const Self) -> *const u8 {
        // SAFETY: the caller's.
        this.cast::<u8>().wrapping_sub(unsafe { std::ptr::addr_of!((*this).at).read() } as usize)
    }
    /// Its rows' address.
    ///
    /// # Safety
    /// As `block_of`.
    unsafe fn rows_of(this: *const Self) -> *const ViewRow {
        // SAFETY: the caller's.
        unsafe { Self::block_of(this).add(std::ptr::addr_of!((*this).rows).read() as usize).cast::<ViewRow>() }
    }
    /// Its rows' address, for their writer (`made::directory`, `Batch::firsts`, `build_dir`).
    ///
    /// # Safety
    /// As `block_of`.
    pub unsafe fn block_rows(this: *const Self) -> *mut ViewRow {
        // SAFETY: the caller's.
        unsafe { Self::rows_of(this).cast_mut() }
    }
    /// Its homes' address, for their writer (`made::directory`, the reads layer's rows).
    ///
    /// # Safety
    /// As `block_of`, its homes allocated.
    pub unsafe fn homes_mut(this: *mut Self) -> *mut ViewHome {
        // SAFETY: the caller's.
        unsafe { Self::block_of(this).add(std::ptr::addr_of!((*this).homes).read() as usize).cast::<ViewHome>().cast_mut() }
    }
    /// How many documents.
    ///
    /// # Safety
    /// As `block_of`.
    pub unsafe fn len_of(this: *const Self) -> u32 {
        // SAFETY: the caller's.
        unsafe { std::ptr::addr_of!((*this).ndocs).read() }
    }
    /// Document `m`'s first row in view `k` (`ROWS`' order); `m == ndocs`: the view's rows in all.
    ///
    /// # Safety
    /// As `block_of`, `m` at most `ndocs`, the first rows written (`counted`).
    pub unsafe fn first_of(this: *const Self, k: usize, m: u32) -> u32 {
        // SAFETY: the caller's.
        unsafe {
            if m == Self::len_of(this) {
                return std::ptr::addr_of!((*this).ends[k]).read();
            }
            std::ptr::addr_of!((*Self::rows_of(this).add(m as usize)).rows[k]).read()
        }
    }
    /// Document `m`'s key.
    ///
    /// # Safety
    /// As `block_of`, `m` below `ndocs`.
    pub unsafe fn key_of(this: *const Self, m: u32) -> u32 {
        // SAFETY: the caller's.
        unsafe { std::ptr::addr_of!((*Self::rows_of(this).add(m as usize)).key).read() }
    }
    /// Its rows, the reads layer's too, where no one writes them.
    ///
    /// # Safety
    /// As `block_of`, and the directory is not being written.
    pub unsafe fn rows(&self) -> &[ViewRow] {
        // SAFETY: the caller's.
        unsafe { std::slice::from_raw_parts(Self::rows_of(self), self.ndocs as usize) }
    }
    /// Its homes.
    ///
    /// # Safety
    /// As `rows`.
    pub unsafe fn homes(&self) -> &[ViewHome] {
        // SAFETY: the caller's.
        unsafe { std::slice::from_raw_parts(Self::block_of(self).add(self.homes as usize).cast::<ViewHome>(), self.nhomes as usize) }
    }
    /// Row `m`'s entry.
    ///
    /// # Safety
    /// As `rows`, and `m` below `ndocs`.
    pub unsafe fn entry(&self, m: u32) -> &ViewDoc {
        // SAFETY: the caller's.
        unsafe { &*Self::block_of(self).add(self.rows()[m as usize].entry as usize).cast::<ViewDoc>() }
    }
    /// The document whose rows of view `k` (`ROWS`' order) hold row `r`: the last whose first row is at or before
    /// it (an empty one starts where the next does); 0 for a directory of none.
    ///
    /// # Safety
    /// As `first_of`.
    pub unsafe fn doc_of_row(this: *const Self, k: u32, r: u32, memo: u32) -> u32 {
        // SAFETY: the caller's.
        unsafe {
            let first = |m: u32| Self::first_of(this, k as usize, m);
            let n = Self::len_of(this);
            if memo < n && first(memo) <= r && (memo + 1 == n || first(memo + 1) > r) {
                return memo;
            }
            crate::ids::bisect_n(n, |m| first(m) <= r).saturating_sub(1)
        }
    }
    /// Its documents' keys' address, in row order (`keys`), for their writer (`made::directory`, `views_over`) and the
    /// search by key.
    ///
    /// # Safety
    /// As `block_of`, the keys allocated.
    pub unsafe fn keys_of(this: *const Self) -> *mut u32 {
        // SAFETY: the caller's.
        unsafe { Self::block_of(this).add(std::ptr::addr_of!((*this).keys).read() as usize).cast::<u32>().cast_mut() }
    }
    /// The document of key `key`, or `NO_DOC`: memo `memo` tried first (a hit ends it), then the batch's documents
    /// by a lower bound of a fixed count of steps (`lower_bound`) and the values' by a scan, the first that is `key`.
    /// Past the memo, the steps are the directory's alone and each a select: a group's lanes each searching a key go
    /// in step (the CLIF's `dir_doc_of_key`).
    ///
    /// # Safety
    /// As `block_of`, the keys written.
    pub unsafe fn doc_of_key(this: *const Self, key: u32, memo: u32) -> u32 {
        // SAFETY: the caller's.
        unsafe {
            let n = Self::len_of(this);
            let keys = std::slice::from_raw_parts(Self::keys_of(this).cast_const(), n as usize);
            let nbatch = std::ptr::addr_of!((*this).nbatch).read().min(n);
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
/// `node_order`; then `op`.
pub const VIEWS: [&str; 13] = ["view_node", "view_child", "view_field", "view_core", "view_line", "view_loc_line", "view_loc_end", "view_loc_col", "view_loc_docof", "view_tape", "view_depth", "view_order", "view_op"];

/// A field's byte offset in the entry.
pub mod at {
    use super::ViewDoc;
    use std::mem::offset_of;
    pub const KEY: i32 = offset_of!(ViewDoc, key) as i32;
    pub const TAG: i32 = offset_of!(ViewDoc, tag) as i32;
    pub const PREORDER: i32 = offset_of!(ViewDoc, preorder) as i32;
    pub const NODES: i32 = offset_of!(ViewDoc, nodes) as i32;
    pub const NUMBERING: i32 = offset_of!(ViewDoc, numbering) as i32;
    pub const TOKENS: i32 = offset_of!(ViewDoc, tokens) as i32;
    pub const OWN_TEXT: i32 = offset_of!(ViewDoc, own_text) as i32;
    pub const ENDS: i32 = offset_of!(ViewDoc, ends) as i32;
    pub const SOURCE_LEN: i32 = offset_of!(ViewDoc, source_len) as i32;
    pub const PLACEMENT: i32 = offset_of!(ViewDoc, placement) as i32;
    pub const FILE: i32 = offset_of!(ViewDoc, file) as i32;
    pub const DELTA: i32 = offset_of!(ViewDoc, delta) as i32;
    pub const WITHIN_LO: i32 = offset_of!(ViewDoc, within_lo) as i32;
    pub const WITHIN_HI: i32 = offset_of!(ViewDoc, within_hi) as i32;
    pub const PLACED: i32 = offset_of!(ViewDoc, placed) as i32;
    pub const NAMES: i32 = offset_of!(ViewDoc, names) as i32;
    pub const SEGMENTS: i32 = offset_of!(ViewDoc, segments) as i32;
    pub const PLACE: i32 = offset_of!(ViewDoc, place) as i32;
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
/// `op`'s: its node's id, the anonymous child's name atom.
pub const OP_COLUMNS: u32 = 2;

/// No row: a parent probe's end (`view_child_first`, `view_child_next`, `view_number`).
pub const NO_ROW: u32 = u32::MAX;
/// `view_line_span`'s answer where the line is not read there (a guest's lines are its host's; past the last line).
pub const NO_SPAN: u64 = u64::MAX;
