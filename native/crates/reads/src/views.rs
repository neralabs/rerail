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
    /// order (u32s, by local id); each node's `tree_digest` hash (32 bytes each, by local id: its value the first 8,
    /// big-endian).
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
    /// The numbering's slot entries by bucket (`Numbering::slot_buckets`, about two words a slot entry), derived once
    /// at the entry's making in its block: an id's slot entry is its bucket's first and a step or two
    /// (`number_of_by_buckets`). Empty (a document numbered by preorder or tape position): no slots.
    pub slot_buckets: Run,
    /// Packed maps (`crate::packed`, about 1.25 bytes an entry; `node_low` two clusters a block, `packed::two`, 2
    /// bytes), derived once at the entry's making in its block: by number, a numbered node's tape node (`node_x`)
    /// and its id's low bits (`node_low`); by own place, a node's
    /// number (`by_place`, `Numbering::number_of_packed`); by `field` row, its node's tape node (`field_x`). An
    /// escaped entry, or an empty map, is read from the numbering.
    pub node_x: Run,
    pub node_low: Run,
    pub by_place: Run,
    pub field_x: Run,
}
const _: () = assert!(std::mem::size_of::<ViewDoc>() == 16 + 16 * 2 + 16 + 24 + 16 * 7 + 16 + 16 + 16 * 4);

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

/// `ViewDoc::preorder` where ids are tape positions.
pub const IDS_TAPE: u32 = 2;
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
    /// The document's own source text (a guest's its own program, not its host's): its first byte in home
    /// `homes[2]`, where a node's text is read in place (`view_text_span`, decision 0287 §2).
    pub source: u64,
    pub homes: [u32; 3],
    pub key: u32,
    pub rows: [u32; ROWS],
    pub view: crate::view::TapeView,
    /// The tape's length in bytes from `tape`: the host's reads read it as a slice (`rerail_tape_format::view_reads`).
    pub bytes: u32,
    /// Its place in the directory's order (`ViewDir::order`: the order of its views' rows), or `NO_DOC` for a number
    /// of the growth none of the directory's documents has.
    pub at: u32,
}
const _: () = assert!(std::mem::size_of::<ViewRow>() == (8 + 8 + 8 + 8 + 12 + 4 + 4 * ROWS + std::mem::size_of::<crate::view::TapeView>() + 4 + 4).next_multiple_of(8));

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
/// `rr_token_named`, `visit_index` and `tree_digest`, in that order (`doc` is a view's: `DOC_VIEW`).
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
/// homes (offsets in the block), how many of each, its 16z cache's home (its number + 1, 0 for none: a block of
/// its own, which both executors' reads share and write), its documents' keys (`keys`, an offset in the block:
/// `ndocs` `u32`s in the directory's order, `order`'s, each its row's entry's key), which `doc_of_key` searches
/// alone, and each view's rows in all (`ROWS`' order; with `counted`, once the rows' first rows are written).
///
/// A row is its document's number in its growth less `first` (plan "Chunk node numbers"): the same document has the
/// same row in every directory of the growth that holds it (a chunk's batch, the batch a round grew it into, a tape
/// value's batch, the layered directory of them), so an id's page names its row (`doc_of_id`). The rows span the
/// numbers from the directory's first to its last (`span`); a number between them that none of its documents has is
/// a row of key and place `NO_DOC`. The views' rows go by the directory's order (`order`: `ndocs` rows, each one's
/// place there its `ViewRow::at`): first the `nbatch` documents of the batch, in key order; past them, a chunk's tape
/// values admitted inside its fixpoint (`values::Layered`, `views_over`), in the order admitted, which a key's search
/// scans.
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
    /// Its growth's page table (`IdPages`, an offset in the block; plan "Chunk node numbers", step 1): each page of
    /// the ids of the batches of its growth (a chunk's rounds and the tape values admitted to it, decision 0431 §6,
    /// §8), its document's number in the growth and the local id it starts at. Every directory of the growth names
    /// the same table: an id handed out in one round is the same id in the next.
    pub pages: u64,
    /// Its order (an offset in the block of `ndocs` rows): the order of its views' rows.
    pub order: u64,
    /// The number of its first row in its growth, and how many rows from it (`span`).
    pub first: u32,
    pub span: u32,
}
const _: () = assert!(std::mem::size_of::<ViewDir>() == (48 + 4 * ROWS + 8 + 8 + 8 + 8).next_multiple_of(8));

/// A growth's ids (plan "Chunk node numbers"): 32 bits, cut into pages of `PAGE` ids. A document takes whole pages
/// when it is first made in the growth, in its role's region, and its later spans (its comments, its matches) further
/// pages when they are made; each page names its document's number and the local id it starts at, so an id's row in
/// any directory of the growth is one load, and its local id the same load (`doc_of_id`, `dir_doc_of_id`,
/// `dir_local_of_id`).
pub const PAGE_BITS: u32 = 12;
pub const PAGE: u32 = 1 << PAGE_BITS;
/// The pages of the 32-bit ids.
pub const PAGES: u32 = 1 << (32 - PAGE_BITS);
/// The documents' roles, each a region of pages: files, code guests, the hidden documents.
pub const ROLES: usize = 3;
/// Each role's first page, then the end: files from id 0, code guests from 2^30, hidden documents from 1.5 × 2^30,
/// the keys' ranges (`syntax::GUEST_KEYS`, `HIDDEN_KEYS`) in 32 bits, so a rule's bound on a role's ids is a
/// literal.
pub const ROLE_PAGES: [u32; ROLES + 1] = [0, (1 << 30) >> PAGE_BITS, ((1 << 30) + (1 << 29)) >> PAGE_BITS, PAGES];

/// The role of the document of key `key`: its key's range (files below 2^30, code guests below 1.5 × 2^30).
pub const fn role_of_key(key: u32) -> usize {
    if key < ROLE_PAGES[1] << PAGE_BITS {
        0
    } else if key < ROLE_PAGES[2] << PAGE_BITS {
        1
    } else {
        2
    }
}

/// A page of a growth's ids: its document's number in the growth and the local id of its first id.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    pub number: u32,
    pub local: u32,
}
const _: () = assert!(std::mem::size_of::<Page>() == 8);

/// A growth's page table, in its block: the pages taken so far in each role's region (`ROLE_PAGES`, from its
/// first), then `PAGES` pages (`Page`). A page is written when it is taken (`take_pages`), before any id of it is
/// handed out; one never taken is never read, so the table is reserved, not zeroed (its pages are address space until
/// written).
#[repr(C)]
#[derive(Debug, Default)]
pub struct IdPages {
    pub tops: [u32; ROLES],
    pub pad: u32,
}
const _: () = assert!(std::mem::size_of::<IdPages>() == 16);

impl IdPages {
    /// The bytes a table takes: its header and its pages.
    pub const BYTES: usize = std::mem::size_of::<IdPages>() + PAGES as usize * std::mem::size_of::<Page>();
    /// Page `p`'s address.
    ///
    /// # Safety
    /// `this` is a table of `BYTES` bytes, alive, and `p` below `PAGES`.
    unsafe fn page_at(this: *const Self, p: u32) -> *mut Page {
        // SAFETY: the caller's.
        unsafe { this.add(1).cast::<Page>().add(p as usize).cast_mut() }
    }
    /// The page of id `id`.
    ///
    /// # Safety
    /// As `page_at`, `id` handed out by `take_pages` (its page written).
    pub unsafe fn page_of(this: *const Self, id: u32) -> Page {
        // SAFETY: the caller's.
        unsafe { Self::page_at(this, id >> PAGE_BITS).read() }
    }
    /// Takes the pages that hold `ids` ids of the document of number `number`, local ids `local..local + ids`, in
    /// role `role`'s region, after the pages taken before (making order): its first id, or `None` where the region
    /// has no room for them. Many threads may take pages at once.
    ///
    /// # Safety
    /// As `page_at`, and `role` below `ROLES`.
    pub unsafe fn take_pages(this: *const Self, role: usize, number: u32, local: u32, ids: u32) -> Option<u32> {
        use std::sync::atomic::{AtomicU32, Ordering};
        let n = ids.div_ceil(PAGE);
        let room = ROLE_PAGES[role + 1] - ROLE_PAGES[role];
        // SAFETY: the caller's: the region's count of pages taken, written through atomics alone.
        let top = unsafe { AtomicU32::from_ptr(std::ptr::addr_of!((*this).tops[role]).cast_mut()) };
        let start = top.try_update(Ordering::AcqRel, Ordering::Acquire, |t| (t + n <= room).then_some(t + n)).ok()?;
        let first = ROLE_PAGES[role] + start;
        for k in 0..n {
            // SAFETY: the caller's: page `first + k`, below `PAGES`, taken by this call alone.
            unsafe { Self::page_at(this, first + k).write(Page { number, local: local + k * PAGE }) };
        }
        Some(first << PAGE_BITS)
    }
}

/// The `node` view's rows by kind over a views' directory (decision 0706 step 0: `node` probed by its kind reads its
/// batches' lists, no index of the view), in the directory's block beside it, self-relative: the directory's kinds,
/// ascending (`kinds`, i64s), each kind's candidates `vstart[k]..vstart[k + 1]` (u32s, `nkinds + 1`), and the
/// segments they lie in (`segs`, ascending by `v`): each a kind's rows of one batch (`KindRows`, decision 0700: the
/// batch's `node` rows, u32s), where its first candidate is (`v`), that batch's first document of the directory
/// (`doc_base`), and its first row (`at`, an offset in the block). A candidate `q`'s row: its segment's entry `q - v`,
/// past `rows[doc_base].rows[0]` (the batch's first `node` row there). Offsets are in the block, which both executors
/// read at their own base.
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
    /// How many rows: the numbers from its first to its last.
    ///
    /// # Safety
    /// As `block_of`.
    pub unsafe fn span_of(this: *const Self) -> u32 {
        // SAFETY: the caller's.
        unsafe { std::ptr::addr_of!((*this).span).read() }
    }
    /// The row `m`th in its order (`order`).
    ///
    /// # Safety
    /// As `block_of`, `m` below `ndocs`, the order written.
    pub unsafe fn row_at(this: *const Self, m: u32) -> u32 {
        // SAFETY: the caller's.
        unsafe { Self::block_of(this).add(std::ptr::addr_of!((*this).order).read() as usize).cast::<u32>().add(m as usize).read() }
    }
    /// Row `row`'s place in its order (`ViewRow::at`), `NO_DOC` for a number none of its documents has.
    ///
    /// # Safety
    /// As `block_of`, `row` below `span`.
    pub unsafe fn at_of(this: *const Self, row: u32) -> u32 {
        // SAFETY: the caller's.
        unsafe { std::ptr::addr_of!((*Self::rows_of(this).add(row as usize)).at).read() }
    }
    /// The order's address, for its writer (`made::directory`, `views_over`).
    ///
    /// # Safety
    /// As `block_of`, the order allocated.
    pub unsafe fn order_mut(this: *mut Self) -> *mut u32 {
        // SAFETY: the caller's.
        unsafe { Self::block_of(this).add(std::ptr::addr_of!((*this).order).read() as usize).cast::<u32>().cast_mut() }
    }
    /// The `m`th document's first row in view `k` (`ROWS`' order), in its order; `m == ndocs`: the view's rows in all.
    ///
    /// # Safety
    /// As `block_of`, `m` at most `ndocs`, the order and the first rows written (`counted`).
    pub unsafe fn first_of(this: *const Self, k: usize, m: u32) -> u32 {
        // SAFETY: the caller's.
        unsafe {
            if m == Self::len_of(this) {
                return std::ptr::addr_of!((*this).ends[k]).read();
            }
            std::ptr::addr_of!((*Self::rows_of(this).add(Self::row_at(this, m) as usize)).rows[k]).read()
        }
    }
    /// Row `m`'s key (`NO_DOC` for a number none of its documents has).
    ///
    /// # Safety
    /// As `block_of`, `m` below `span`.
    pub unsafe fn key_of(this: *const Self, m: u32) -> u32 {
        // SAFETY: the caller's.
        unsafe { std::ptr::addr_of!((*Self::rows_of(this).add(m as usize)).key).read() }
    }
    /// Its rows (`span` of them, by number), the reads layer's too, where no one writes them.
    ///
    /// # Safety
    /// As `block_of`, and the directory is not being written.
    pub unsafe fn rows(&self) -> &[ViewRow] {
        // SAFETY: the caller's.
        unsafe { std::slice::from_raw_parts(Self::rows_of(self), self.span as usize) }
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
    /// As `rows`, and `m` a row of one of its documents.
    pub unsafe fn entry(&self, m: u32) -> &ViewDoc {
        // SAFETY: the caller's.
        unsafe { &*Self::block_of(self).add(self.rows()[m as usize].entry as usize).cast::<ViewDoc>() }
    }
    /// The row of the document whose rows of view `k` (`ROWS`' order) hold row `r`: the last in its order whose
    /// first row is at or before it (an empty one starts where the next does); 0 for a directory of none. `memo` a
    /// row, tried first.
    ///
    /// # Safety
    /// As `first_of`.
    pub unsafe fn doc_of_row(this: *const Self, k: u32, r: u32, memo: u32) -> u32 {
        // SAFETY: the caller's.
        unsafe {
            let first = |m: u32| Self::first_of(this, k as usize, m);
            let n = Self::len_of(this);
            if n == 0 {
                return 0;
            }
            if memo < Self::span_of(this) {
                let at = Self::at_of(this, memo);
                if at < n && first(at) <= r && (at + 1 == n || first(at + 1) > r) {
                    return memo;
                }
            }
            Self::row_at(this, crate::ids::bisect_n(n, |m| first(m) <= r).saturating_sub(1))
        }
    }
    /// Its documents' keys' address, in its order (`keys`, the `m`th the key of `order`'s `m`th row), for their writer
    /// (`made::directory`, `views_over`) and the search by key.
    ///
    /// # Safety
    /// As `block_of`, the keys allocated.
    pub unsafe fn keys_of(this: *const Self) -> *mut u32 {
        // SAFETY: the caller's.
        unsafe { Self::block_of(this).add(std::ptr::addr_of!((*this).keys).read() as usize).cast::<u32>().cast_mut() }
    }
    /// The row of the document of key `key`, or `NO_DOC`: row `memo` tried first (a hit ends it), then the batch's
    /// documents by a lower bound of a fixed count of steps (`lower_bound`) and the values' by a scan, the first that
    /// is `key`, each found in the order (`keys`) and its row read there (`row_at`). Past the memo, the steps are the
    /// directory's alone and each a select: a group's lanes each searching a key go in step (the CLIF's
    /// `dir_doc_of_key`).
    ///
    /// # Safety
    /// As `block_of`, the keys and the order written.
    pub unsafe fn doc_of_key(this: *const Self, key: u32, memo: u32) -> u32 {
        // SAFETY: the caller's.
        unsafe {
            if memo < Self::span_of(this) && Self::key_of(this, memo) == key {
                return memo;
            }
            let n = Self::len_of(this);
            let keys = std::slice::from_raw_parts(Self::keys_of(this).cast_const(), n as usize);
            let nbatch = std::ptr::addr_of!((*this).nbatch).read().min(n);
            let c = lower_bound(&keys[..nbatch as usize], key);
            if c < nbatch && keys[c as usize] == key {
                return Self::row_at(this, c);
            }
            let m = (nbatch..n).rev().fold(NO_DOC, |found, i| if keys[i as usize] == key { i } else { found });
            if m == NO_DOC { NO_DOC } else { Self::row_at(this, m) }
        }
    }
    /// Its growth's page table.
    ///
    /// # Safety
    /// As `block_of`, the table allocated.
    pub unsafe fn table_of(this: *const Self) -> *const IdPages {
        // SAFETY: the caller's.
        unsafe { Self::block_of(this).add(std::ptr::addr_of!((*this).pages).read() as usize).cast::<IdPages>() }
    }
    /// The row of the document of id `id`, or `NO_DOC` where its number is none of the directory's rows: its page's
    /// number less the directory's first (one load: a row is a number, in every directory of the growth). A row of
    /// a number none of its documents has has key `NO_DOC`.
    ///
    /// # Safety
    /// As `table_of`, `id` handed out by `IdPages::take_pages`.
    pub unsafe fn doc_of_id(this: *const Self, id: u32) -> u32 {
        // SAFETY: the caller's.
        unsafe {
            let row = IdPages::page_of(Self::table_of(this), id).number.wrapping_sub(std::ptr::addr_of!((*this).first).read());
            if row < Self::span_of(this) { row } else { NO_DOC }
        }
    }
    /// The local id of id `id` in its document: its page's first, and its place in the page.
    ///
    /// # Safety
    /// As `doc_of_id`.
    pub unsafe fn local_of_id(this: *const Self, id: u32) -> u32 {
        // SAFETY: the caller's.
        unsafe { IdPages::page_of(Self::table_of(this), id).local + (id & (PAGE - 1)) }
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
/// `node_order`; then `op`, then `doc` (a row per file: its key, its path's and module's atoms).
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
/// `view_line_span`'s answer where the line is not read there (a guest's lines are its host's; past the last line).
pub const NO_SPAN: u64 = u64::MAX;
