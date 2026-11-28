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
    /// How ids are numbered: 0 by definition (the granule table), 1 local ids (`RERAIL_NODE_IDS=preorder`), `IDS_TAPE`
    /// tape positions (`RERAIL_NODE_IDS=tape`: a node's or token's id is its tape node, a span's in the top slots).
    pub preorder: u32,
    /// The projection's numbered nodes.
    pub nodes: u32,
    /// The projection's numbering (`ids::Numbering`, one).
    pub numbering: Run,
    /// By the tape's code (its table's number, `tape_number`): the kind atom its `kind` column holds (u32s, its
    /// name's canonical code's), and the field atom (`tag << 16 | field`).
    pub kinds: Run,
    pub fields: Run,
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
    /// `doc`'s row (`tape_rows::DocRow`, its program reading it): the document's path's and module's atoms, interned
    /// once in its batch's atoms (`tape_rows::derive`), as the host's rows give them.
    pub doc_path: u32,
    pub doc_module: u32,
    /// 1 when every numbered node but the root has a numbered tape parent (`Projected::parented`): the root's
    /// children are its numbered tape children, walked as any parent's (`child_first`, `child_next`); 0 where a
    /// numbered node's parent is left out, its `child` parent the root.
    pub parented: u32,
    /// SPIKE (shared strata work, A): by local id, the numbered node's tape node and its id's low bits (`[u32; 2]`s),
    /// derived once at the entry's making in its block; empty: read by the numbering (`num_select`, `num_low_of`).
    pub node_table: Run,
    /// SPIKE (local fixes, 1): the numbered nodes' local ids by their place among every granule's own nodes, in
    /// granule order (`Numbering::own_place_of`, `own_numbers`), derived once with `node_table`: an id's local is the
    /// numbering's slot entry and then one load, not a search of the granule's children. Empty where `node_table` is.
    pub node_by_id: Run,
}
const _: () = assert!(std::mem::size_of::<ViewDoc>() == 16 + 16 * 4 + 16 + 24 + 16 * 7 + 16 + 16 + 16);

/// `ViewDoc::place`'s kinds.
pub const PLACE_NAMED: u32 = 1;
pub const PLACE_PAST_COMMENTS: u32 = 2;
/// The rank index's item of an anchored projection's anchors (`Item::Anchor`).
pub const ANCHOR_ITEM: i64 = 11;

/// `ViewDoc::preorder` where ids are tape positions.
pub const IDS_TAPE: u32 = 2;

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
/// the batch's block, its tape's, its lines' and its source's places (a home of the directory's, `ViewHome`, and an
/// offset in it), and its tape's view (the reads' own). A node's text is its span of the source, read there
/// (`@rr_node_text_any`). Nothing in it is an address: the host and the device read the same row.
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
pub const SIBLING: usize = VIEWS.len();
/// `op`'s rows (each numbered node's anonymous children, `Item::Op`): `view_op`'s, in `VIEWS`.
pub const OP: usize = 12;
const _: () = assert!(matches!(VIEWS[OP].as_bytes(), b"view_op"));
/// The token order's four relations' (`tokens::TokRel`'s order; the engine's `tape_rows`): `TOK + rel`.
pub const TOK: usize = SIBLING + 1;
pub const SPREAD: usize = TOK + 4;
pub const END: usize = SPREAD + 1;
/// `text`'s rows (decision 0656 step 4: a view, no row made).
pub const TEXT: usize = END + 1;
/// The first of the tape's other relations' (`tape_rows`): `structural_before`, `structural_after`,
/// `rr_token_named`, `visit_index` and `tree_digest`, in that order (`doc` is a view's: `DOC`).
pub const TAPE: usize = TEXT + 1;
/// How many relations `TAPE` starts.
pub const TAPE_RELATIONS: usize = 5;

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
/// homes (offsets in the block), how many of each, its 16z cache's home (its number + 1, 0 for none: a block of its
/// own, which both executors' reads share and write), its documents' order by key (`by_key`: an offset in the block of
/// `ndocs` row numbers), and each view's rows in all (`ROWS`' order; with `counted`, once the rows' first rows are
/// written). The rows are in row order: a batch's own are in key order too, but a layered directory's are the chunk's
/// batch's documents, then each values batch's (`views_over`), each in key order but not all together, so a lookup by
/// key bisects `by_key`.
#[repr(C)]
#[derive(Debug, Default)]
pub struct ViewDir {
    pub at: u64,
    pub ndocs: u32,
    pub nhomes: u32,
    pub rows: u64,
    pub homes: u64,
    pub cache: u64,
    pub by_key: u64,
    pub ends: [u32; ROWS],
    pub counted: u32,
    pub pad: u32,
}
const _: () = assert!(std::mem::size_of::<ViewDir>() == (48 + 4 * ROWS + 8).next_multiple_of(8));

/// The `node` view's rows by kind over a views' directory (decision 0706 step 0: `node` probed by its kind reads its
/// batches' lists, no index of the view), in the directory's block beside it, self-relative: the directory's kinds,
/// ascending (`kinds`, i64s), each kind's candidates `vstart[k]..vstart[k + 1]` (u32s, `nkinds + 1`), and the
/// segments they lie in (`segs`, ascending by `v`): each a kind's run of one batch's list (`KindRows`' `at`, one
/// `[document, local id, tape node]` a node, the document the batch's), where its first candidate is (`v`), that
/// batch's first row of the directory (`doc_base`), and the run's first entry (`at`, an offset in the block). A
/// candidate `q`'s row: its segment's entry `q - v`, `rows[doc_base + document].rows[0] + local id`. Offsets are in
/// the block, which both executors read at their own base.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct KindTable {
    pub nkinds: u32,
    pub nsegs: u32,
    pub kinds: u64,
    pub vstart: u64,
    pub segs: u64,
}
const _: () = assert!(std::mem::size_of::<KindTable>() == 32);

/// A segment of a `KindTable`.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct KindSeg {
    pub v: u32,
    pub doc_base: u32,
    pub at: u64,
}
const _: () = assert!(std::mem::size_of::<KindSeg>() == 16);

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
    /// The row of the document `i`th by key (`by_key`).
    ///
    /// # Safety
    /// As `block_of`, `i` below `ndocs`, the order written.
    pub unsafe fn by_key_of(this: *const Self, i: u32) -> u32 {
        // SAFETY: the caller's.
        unsafe { Self::block_of(this).add(std::ptr::addr_of!((*this).by_key).read() as usize).cast::<u32>().add(i as usize).read() }
    }
    /// The order by key's address, for its writer (`made::directory`, `views_over`).
    ///
    /// # Safety
    /// As `block_of`, the order allocated.
    pub unsafe fn by_key_mut(this: *mut Self) -> *mut u32 {
        // SAFETY: the caller's.
        unsafe { Self::block_of(this).add(std::ptr::addr_of!((*this).by_key).read() as usize).cast::<u32>().cast_mut() }
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
    /// The document of key `key` (bisecting the order by key, `by_key`), or `NO_DOC`.
    ///
    /// # Safety
    /// As `key_of`.
    pub unsafe fn doc_of_key(this: *const Self, key: u32, memo: u32) -> u32 {
        // SAFETY: the caller's.
        unsafe {
            let key_of = |m: u32| Self::key_of(this, m);
            let n = Self::len_of(this);
            if memo < n && key_of(memo) == key {
                return memo;
            }
            let row = |i: u32| Self::by_key_of(this, i);
            let c = crate::ids::bisect_n(n, |i| key_of(row(i)) <= key);
            if c > 0 && key_of(row(c - 1)) == key { row(c - 1) } else { NO_DOC }
        }
    }
}

/// A view's number in a kernel's reads (`ViewDir`'s rows' and `vread`'s): `node`, `child`, `field`, `core_node`,
/// `line`, then core.rerail's places (`Loc`): `line`, `end_line`, `col`, `docof`; then `tape`, `ast_depth` and
/// `node_order`; then `op`; then `doc` (a row per file: its key, its path's and module's atoms).
pub const VIEWS: [&str; 14] = ["view_node", "view_child", "view_field", "view_core", "view_line", "view_loc_line", "view_loc_end", "view_loc_col", "view_loc_docof", "view_tape", "view_depth", "view_order", "view_op", "view_doc"];
/// `doc`'s slot (`VIEWS`' last): its first rows, one a file.
pub const DOC_VIEW: usize = VIEWS.len() - 1;

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
    pub const DOC_PATH: i32 = offset_of!(ViewDoc, doc_path) as i32;
    pub const DOC_MODULE: i32 = offset_of!(ViewDoc, doc_module) as i32;
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
/// `doc`'s: the document's key, its path's atom, its module's.
pub const DOC_COLUMNS: u32 = 3;

/// No row: a parent probe's end (`view_child_first`, `view_child_next`, `view_number`).
pub const NO_ROW: u32 = u32::MAX;
