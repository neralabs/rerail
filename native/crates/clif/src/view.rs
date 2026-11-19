//! A document's tape as its reads are told where it lies: byte offsets from a base (the document's
//! first byte in its batch's chunk block on the device, decision 0289 §4; the tape's own bytes on the
//! host), its format, its widths. One definition: the device's `DocView` is this, and its MSL is
//! written from it ([`ukl`]), with each field's offset asserted against Rust's.

/// A document's view (120 bytes, u32s; the device's `DocView`, `device_tape.h`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TapeView {
    /// The document's first byte in its batch's chunk block (the device's; 0 on the host): every
    /// offset below counts from it, so a copy into another block changes `at` alone (0289 §4).
    pub at: u32,
    pub nodes: u32,
    pub node_base: u32,
    /// The tape's format: 15, 16, or `F16Z_FORMAT` (a 16z tape as stored, read through a cache of its decoded
    /// blocks, decision 0353 step 4b).
    pub format: u32,
    /// Format 15: `size`'s, `off`'s and `tail`'s widths in bytes, at bits 0, 4 and 8; bit 12: tails
    /// measured from the node's own start. Format 16: the slot, gap and length widths at bits 0, 4
    /// and 8, the checkpoints' at 12, and the four side lists' value widths at 16, 20, 24 and 28.
    pub widths: u32,
    /// Format 15's columns (code, size, base, off, tail). Format 16 (block-major, decision 0426): block
    /// 0's start, then the bytes a row holds before the slots, the gaps, the lengths and the checkpoints
    /// (a block of R rows holds column r at `R * columns[r]` from its start).
    pub columns: [u32; 5],
    /// Format 16: the blocks' stride, in bytes (a block is `F16_PART` nodes).
    pub stride: u32,
    /// Format 16: the four side lists (codes' low parts, escaped slots, gaps, lengths), and each
    /// one's part starts (u32s, a part a 1,024 nodes).
    pub side: [u32; 4],
    pub parts: [u32; 4],
    /// Format 16: its language's cutoff and the numbers a prefix byte covers.
    pub cut: u32,
    pub per: u32,
    /// The code table (the device's `DeviceCode`s; format 16's: the language's).
    pub codes: u32,
    pub source: u32,
    pub source_len: u32,
    /// Format 15: line starts (u32, document-local bytes), the first 0. Format 16: its newline index
    /// (`rerail_tape_format::newlines`). And how many lines.
    pub lines: u32,
    pub line_count: u32,
    /// The code table's length.
    pub code_count: u32,
    /// The document's place in a batch's numbering of source bytes and lines.
    pub source_base: u32,
    pub line_base: u32,
    /// Where the stored tape starts.
    pub tape: u32,
}
const _: () = assert!(std::mem::size_of::<TapeView>() == 120);

/// The view in UKL: `struct TapeView`, `DocView` its name in the kernels, and each field's offset and
/// the size asserted against this definition's, so the kernels cannot read a layout of their own.
pub fn ukl() -> String {
    use std::mem::offset_of;
    let fields: [(&str, &str, usize); 20] = [
        ("uint32_t", "at", offset_of!(TapeView, at)),
        ("uint32_t", "nodes", offset_of!(TapeView, nodes)),
        ("uint32_t", "node_base", offset_of!(TapeView, node_base)),
        ("uint32_t", "format", offset_of!(TapeView, format)),
        ("uint32_t", "widths", offset_of!(TapeView, widths)),
        ("uint32_t", "columns[5]", offset_of!(TapeView, columns)),
        ("uint32_t", "stride", offset_of!(TapeView, stride)),
        ("uint32_t", "side[4]", offset_of!(TapeView, side)),
        ("uint32_t", "parts[4]", offset_of!(TapeView, parts)),
        ("uint32_t", "cut", offset_of!(TapeView, cut)),
        ("uint32_t", "per", offset_of!(TapeView, per)),
        ("uint32_t", "codes", offset_of!(TapeView, codes)),
        ("uint32_t", "source", offset_of!(TapeView, source)),
        ("uint32_t", "source_len", offset_of!(TapeView, source_len)),
        ("uint32_t", "lines", offset_of!(TapeView, lines)),
        ("uint32_t", "line_count", offset_of!(TapeView, line_count)),
        ("uint32_t", "code_count", offset_of!(TapeView, code_count)),
        ("uint32_t", "source_base", offset_of!(TapeView, source_base)),
        ("uint32_t", "line_base", offset_of!(TapeView, line_base)),
        ("uint32_t", "tape", offset_of!(TapeView, tape)),
    ];
    let mut s = String::from("#include <ukl/backend.h>\ntypedef struct TapeView {\n");
    for (ty, name, _) in fields.iter() {
        s += &format!("    {ty} {name};\n");
    }
    s += "} TapeView;\ntypedef TapeView DocView;\n";
    s += &format!("UKL_STATIC_ASSERT(sizeof(TapeView) == {}, \"TapeView: size\");\n", std::mem::size_of::<TapeView>());
    for (_, name, at) in fields.iter() {
        let name = name.split('[').next().unwrap_or(name);
        s += &format!("UKL_STATIC_ASSERT(__builtin_offsetof(TapeView, {name}) == {at}, \"TapeView.{name}\");\n");
    }
    s
}

/// Format 16's numbers the reads are written for (`rerail_tape_format::f16`'s; a test there holds
/// them equal): checkpoints every `K` nodes, side lists in parts of `PART` nodes, blocks of `BLOCK`.
pub const F16_K: u32 = 8;
pub const F16_BLOCK: u32 = 256;
pub const F16_PART: u32 = 1024;
pub const F16_SUB: u32 = F16_PART / F16_BLOCK;
/// A 16z tape's format in its view (decision 0353 step 4b): format 16 stored a block of `F16Z_BLOCK` nodes at a
/// time in `F16Z_PIECES` pieces (decision 0426 §3), read in place, a block decoded where a read falls into an entry
/// of a rolling cache (`cache`). Its view's `columns` are the piece index's offset (each piece's start in the
/// payload, a u32; then each one's length; then each one's kind, a byte), the side lists' part starts' (list `k`'s
/// `blocks + 1` u32s, list after list), the checkpoints' (raw, `F16Z_BLOCK / F16_K` a block), the cache's entry
/// size (a power of two) and whether threads share the cache (1: a read checks its entry after it); `stride` is
/// the payload's offset; `side` is the cache's address less the tape's (an i64, low word first) and the tape's id
/// (a u64, its blocks' keys); `lines` and `line_count` its newline index, raw, as format 16's; its widths, cut and
/// per are format 16's.
pub const F16Z_FORMAT: u32 = 1016;
pub const F16Z_BLOCK: u32 = 1024;
/// The pieces a 16z block is stored in (decision 0426 §3): codes, slots, gaps, lengths, each its array's column in
/// the block then its side list's part.
pub const F16Z_PIECES: u32 = 4;

/// A decoded 16z block in a cache entry's payload, from the entry's first byte: the block's view (format 16's, its
/// offsets from the bytes) and its bytes (its regions, each side list's one part, and room for the decode's 8-byte
/// copies past its end).
pub mod z {
    pub const SLOT_VIEW: i32 = 16;
    pub const SLOT_BYTES: i32 = 144;
    /// The room past a block's bytes: its side lists' parts (32 bytes) and the decode's copies past its end.
    pub const SLACK: usize = 64;
    const _: () = assert!(SLOT_VIEW as usize + std::mem::size_of::<super::TapeView>() <= SLOT_BYTES as usize);
    /// How a piece is stored (its kind byte in the piece index): as its bytes, as one LZ4 block, or as a repeat of
    /// another piece (its payload that piece's number, a u32).
    pub const RAW: u8 = 0;
    pub const LZ4: u8 = 1;
    pub const REPEAT: u8 = 2;
}

/// A rolling cache (decision 0353 step 4b, the user's design): a key (a u64) to an entry of fixed size, in `gens`
/// generations, each a hash index (u32 slots: an entry's number plus one) and an arena of entries. Of the
/// generations, `gens - 1` are live at an epoch `E` (the head, `E`, and the frozen ones behind it, `E - 1` ...) and
/// one, `E + 1`, is a spare, cleared ahead. A lookup looks in the live ones, head first; a miss computes its entry
/// into the head; a full head makes the spare the head (one compare-and-swap of the epoch) and the thread that
/// turned it clears the next spare, the oldest live one; a hit in the oldest is copied into the head. A miss waits
/// for no thread: two that miss one key both compute it. Every loop is bounded; a lookup that finds no room
/// counts a failure (`FAILED`) and reads the dead entry, so a caller that shares the cache refuses its work.
pub mod cache {
    /// The header's u32s, from the cache's first byte.
    pub const EPOCH: i32 = 0;
    pub const GENS: i32 = 4;
    /// Entries a generation.
    pub const ENTRIES: i32 = 8;
    /// An entry's size, its log2.
    pub const SHIFT: i32 = 12;
    /// Index slots a generation, a power of two.
    pub const SLOTS: i32 = 16;
    pub const GEN_STRIDE: i32 = 20;
    /// Where a generation's entries start, from its first byte.
    pub const FIRST_ENTRY: i32 = 24;
    /// The dead entry's offset from the cache.
    pub const DEAD: i32 = 28;
    /// Counts: lookups that found no room, misses, entries copied to the head, generations turned.
    pub const FAILED: i32 = 32;
    pub const MISSES: i32 = 36;
    pub const PROMOTED: i32 = 40;
    pub const ROTATED: i32 = 44;
    /// The statistics mode (1: the counts below kept too, for tuning a cache's size), and its counts: lookups that
    /// found their entry complete, ones that found it being written and waited for it, and blocks decoded while
    /// another thread decoded the same one (a wait run out, or an index lost to it).
    pub const STATS: i32 = 48;
    pub const HITS: i32 = 52;
    pub const WAITED: i32 = 56;
    pub const DUPLICATES: i32 = 60;
    pub const HEAD: usize = 64;
    /// A generation's u32s: the epoch it holds (0 while it is cleared) and its entries taken; its index after.
    pub const STAMP: i32 = 0;
    pub const USED: i32 = 4;
    pub const INDEX: i32 = 16;
    /// An entry's: its key (a u64) and the epoch it was completed in (0 while it is written); its payload after.
    pub const KEY: i32 = 0;
    pub const READY: i32 = 8;
    pub const PAYLOAD: i32 = 16;
    /// An entry's completion while a writer of that epoch writes it (the epoch's bit 31 set).
    pub const WRITING: u32 = 1 << 31;
    /// A lookup's probes of a generation's index, its tries, and its waits for an entry another thread writes.
    pub const PROBES: u32 = 16;
    pub const TRIES: u32 = 4;
    pub const SPIN: u32 = 1 << 16;
    /// An index slot's u32: its key's tag (16 bits, never 0) above its entry's number + 1 (0 while its writer has
    /// claimed the slot and not yet an entry), so a generation holds at most `NUMBER` entries.
    pub const TAG_SHIFT: u32 = 16;
    pub const NUMBER: u32 = (1 << TAG_SHIFT) - 1;
    /// The room past the dead entry: an entry's masked loads reach at most an entry and a word past it.
    pub const SLACK: usize = 256;
    /// A guard after the slack, of `GUARD_BYTE`s: no read or write of the cache reaches it (decision 0427), and
    /// `Spec::check` finds it as `init` wrote it, else a writer missed its own memory.
    pub const GUARD: usize = 256;
    pub const GUARD_BYTE: u8 = 0xA5;

    /// A cache's shape.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Spec {
        /// Generations, 3 or more (one is the spare).
        pub gens: u32,
        pub entries: u32,
        /// An entry's size, its log2.
        pub shift: u32,
        /// Index slots a generation, a power of two of at least twice the entries.
        pub slots: u32,
        /// Whether its lookups keep the statistics mode's counts (`STATS`).
        pub stats: bool,
    }

    impl Spec {
        /// The smallest index for `entries`.
        pub fn new(gens: u32, entries: u32, entry_bytes: usize) -> Spec {
            let shift = entry_bytes.next_power_of_two().trailing_zeros();
            let entries = entries.clamp(1, NUMBER - 1);
            Spec { gens: gens.max(3), entries, shift, slots: (2 * entries).next_power_of_two().max(8), stats: false }
        }
        /// A cache for `blocks` blocks of at most `entry` bytes decoded, in at most `budget` bytes (decision 0353
        /// step 4b): three generations, each of four times the blocks (64 at least) where the budget allows, else
        /// as many as it allows. The room past the blocks is the head's: a block promoted from the oldest takes a
        /// second entry (with one entry a block, the device test's batch of five blocks turned 25 times in a pass
        /// and 8 reads found no room).
        pub fn sized(blocks: usize, entry: usize, budget: usize) -> Spec {
            let gens = 3;
            let want = (4 * blocks).max(64);
            let whole = Spec::new(gens, u32::try_from(want).unwrap_or(u32::MAX), entry);
            let fits = (budget / gens as usize).saturating_sub(whole.first_entry()) / whole.entry_bytes();
            Spec::new(gens, u32::try_from(want.min(fits).max(1)).unwrap_or(u32::MAX), entry)
        }
        /// `sized` within the run's budget for a batch's cache (`RERAIL_ZCACHE_MB`, else 48 MB: an eighth of a
        /// chunk's), in the statistics mode with `RERAIL_ZCACHE_STATS=1`: the views' directory's and the residency
        /// table's.
        pub fn configured(blocks: usize, entry: usize) -> Spec {
            let mb = std::env::var("RERAIL_ZCACHE_MB").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(48);
            let spec = Spec::sized(blocks, entry, mb << 20);
            if std::env::var_os("RERAIL_ZCACHE_STATS").is_some_and(|v| v == "1") { spec.with_stats() } else { spec }
        }
        /// The same cache in the statistics mode.
        pub fn with_stats(self) -> Spec {
            Spec { stats: true, ..self }
        }
        pub fn entry_bytes(&self) -> usize {
            1 << self.shift
        }
        pub fn first_entry(&self) -> usize {
            (INDEX as usize + 4 * self.slots as usize).next_multiple_of(64)
        }
        pub fn gen_stride(&self) -> usize {
            self.first_entry() + (self.entries as usize) * self.entry_bytes()
        }
        pub fn dead(&self) -> usize {
            HEAD + self.gens as usize * self.gen_stride()
        }
        pub fn bytes(&self) -> usize {
            self.dead() + self.entry_bytes() + SLACK + GUARD
        }

        /// The cache in `buf` checked as `init` made it (decision 0427): its shape's words, from which every
        /// address its reads take is made, and its guard. A cache whose threads are done (or a sample while they
        /// run). `Err` says what was overwritten: a write that missed its own memory, never one of the cache's.
        pub fn check(buf: &[u8]) -> Result<Spec, String> {
            let get = |at: i32| buf.get(at as usize..at as usize + 4).map(|b| u32::from_le_bytes(b.try_into().expect("4 bytes")));
            let word = |at: i32| get(at).ok_or_else(|| format!("a cache of {} bytes, shorter than its header", buf.len()));
            let spec = Spec { gens: word(GENS)?, entries: word(ENTRIES)?, shift: word(SHIFT)?, slots: word(SLOTS)?, stats: word(STATS)? != 0 };
            if spec.gens < 3 || spec.shift >= 32 || !spec.slots.is_power_of_two() || spec.entries == 0 || spec.entries >= NUMBER || spec.slots < 2 * spec.entries {
                return Err(format!("a cache's shape overwritten: {spec:?}"));
            }
            let stated = [(GEN_STRIDE, spec.gen_stride()), (FIRST_ENTRY, spec.first_entry()), (DEAD, spec.dead())];
            if let Some((at, want)) = stated.into_iter().find(|&(at, want)| word(at).ok().map(|v| v as usize) != Some(want)) {
                return Err(format!("a cache's word at {at} overwritten: {:?}, not {want}", get(at)));
            }
            let guard = buf.get(spec.bytes() - GUARD..spec.bytes()).ok_or_else(|| format!("a cache of {} bytes for {spec:?}", buf.len()))?;
            if let Some(k) = guard.iter().position(|&b| b != GUARD_BYTE) {
                return Err(format!("a cache's guard overwritten at byte {k} of {GUARD} (a write past its slack)"));
            }
            Ok(spec)
        }

        /// `buf` (`bytes()` long, 8-aligned) made an empty cache: its header, each generation stamped (the live
        /// ones at the first epoch, the spare at the next), nothing taken.
        pub fn init(&self, buf: &mut [u8]) -> Result<(), String> {
            if buf.len() < self.bytes() || !self.slots.is_power_of_two() || self.gens < 3 {
                return Err(format!("a cache of {} bytes for {self:?}", buf.len()));
            }
            let word = |v: usize| u32::try_from(v).map_err(|_| format!("a cache past 4 GiB: {self:?}"));
            buf.fill(0);
            let mut put = |at: usize, v: u32| buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
            let epoch = self.gens;
            put(EPOCH as usize, epoch);
            put(GENS as usize, self.gens);
            put(ENTRIES as usize, self.entries);
            put(SHIFT as usize, self.shift);
            put(SLOTS as usize, self.slots);
            put(GEN_STRIDE as usize, word(self.gen_stride())?);
            put(FIRST_ENTRY as usize, word(self.first_entry())?);
            put(DEAD as usize, word(self.dead())?);
            put(STATS as usize, u32::from(self.stats));
            for k in 0..self.gens {
                // Live at `epoch - k` for k < gens - 1; the spare at `epoch + 1`.
                let stamp = if k + 1 < self.gens { epoch - k } else { epoch + 1 };
                put(HEAD + (stamp % self.gens) as usize * self.gen_stride() + STAMP as usize, stamp);
            }
            buf[self.bytes() - GUARD..self.bytes()].fill(GUARD_BYTE);
            Ok(())
        }
    }

    /// A cache's counts, read from its header (`cache` the cache's first byte): after the threads that use it are
    /// done, or as a sample while they run.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Counts {
        pub misses: u32,
        pub promoted: u32,
        pub rotated: u32,
        pub failed: u32,
        /// The statistics mode's (0 without it).
        pub hits: u32,
        pub waited: u32,
        pub duplicates: u32,
    }

    impl Counts {
        /// The counts of the cache `buf` (a whole cache), checked first (`Spec::check`): a cache found overwritten
        /// panics, naming what was (decision 0427): memory a writer missed is no state to go on from.
        pub fn checked(buf: &[u8]) -> Counts {
            if let Err(e) = Spec::check(buf) {
                panic!("a 16z block cache (decision 0427): {e}");
            }
            // SAFETY: a cache's header, checked just now, in `buf`.
            unsafe { Counts::read(buf.as_ptr()) }
        }

        /// # Safety
        /// `cache` is a cache's first byte (a `Spec::init`ed buffer), readable for its header.
        pub unsafe fn read(cache: *const u8) -> Counts {
            // SAFETY: the header's words, 4-aligned in a buffer the caller vouches for.
            let word = |at: i32| unsafe { (*cache.add(at as usize).cast::<std::sync::atomic::AtomicU32>()).load(std::sync::atomic::Ordering::Relaxed) };
            Counts { misses: word(MISSES), promoted: word(PROMOTED), rotated: word(ROTATED), failed: word(FAILED), hits: word(HITS), waited: word(WAITED), duplicates: word(DUPLICATES) }
        }
        /// Each count by its name in the profile (`zcache.*`).
        pub fn named(&self) -> [(&'static str, u32); 7] {
            [("zcache.misses", self.misses), ("zcache.promoted", self.promoted), ("zcache.rotated", self.rotated), ("zcache.failed", self.failed), ("zcache.hits", self.hits), ("zcache.waited", self.waited), ("zcache.duplicates", self.duplicates)]
        }
        /// Lookups that found their block (complete or waited for) of all lookups that kept counts.
        pub fn hit_ratio(&self) -> f64 {
            let found = f64::from(self.hits) + f64::from(self.waited);
            found / (found + f64::from(self.misses) + f64::from(self.failed)).max(1.0)
        }
    }
}

/// A reader's view of a tape (decision 0353 step 4b): the view, and for a 16z tape a cache of its own after it,
/// which its reads write: one thread reads through a reader at a time.
pub struct Reader {
    words: *mut u128,
    len: usize,
}

/// Where a reader's cache starts, from its view.
const READER_CACHE: usize = 128;

impl Reader {
    /// `view`'s reader over `tape` (the bytes its offsets count from): the view alone for format 15 or 16; for 16z,
    /// a cache of two live generations of two blocks after it, which the view names.
    pub fn new(view: &TapeView, tape: *const u8) -> Reader {
        let spec = (view.format == F16Z_FORMAT).then(|| cache::Spec::new(3, 2, view.columns[3] as usize));
        let bytes = READER_CACHE + spec.map_or(0, |s| s.bytes());
        let len = bytes.div_ceil(16);
        let words = Box::into_raw(vec![0u128; len].into_boxed_slice()).cast::<u128>();
        let mut view = *view;
        if let Some(spec) = spec {
            // SAFETY: the cache's bytes, after the view, in the buffer just made.
            let buf = unsafe { std::slice::from_raw_parts_mut(words.cast::<u8>().add(READER_CACHE), spec.bytes()) };
            spec.init(buf).expect("a reader's cache");
            let rel = (buf.as_ptr() as i64).wrapping_sub(tape as i64);
            (view.side[0], view.side[1]) = (rel as u32, (rel >> 32) as u32);
            view.columns[4] = 0;
        }
        // SAFETY: the buffer is at least `READER_CACHE` bytes, 16-aligned; the view is plain words.
        unsafe { words.cast::<TapeView>().write(view) };
        Reader { words, len }
    }

    /// The view the reads take: its address, its cache after it.
    pub fn view(&self) -> *const TapeView {
        self.words.cast()
    }

    /// Its 16z cache's counts (`None` for another format), checked (`cache::Spec::check`).
    pub fn counts(&self) -> Option<cache::Counts> {
        (16 * self.len > READER_CACHE).then(|| {
            // SAFETY: the cache's bytes after the view, in the buffer `new` made; no read runs while this borrows it.
            cache::Counts::checked(unsafe { std::slice::from_raw_parts(self.words.cast::<u8>().add(READER_CACHE), 16 * self.len - READER_CACHE) })
        })
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        // A 16z reader's cache checked as its reads leave it (decision 0427).
        if 16 * self.len > READER_CACHE {
            // SAFETY: the cache's bytes after the view, in the buffer `new` made, which no read uses now.
            let cache = unsafe { std::slice::from_raw_parts(self.words.cast::<u8>().add(READER_CACHE), 16 * self.len - READER_CACHE) };
            if let Err(e) = cache::Spec::check(cache) {
                panic!("a 16z reader's block cache (decision 0427): {e}");
            }
        }
        // SAFETY: the buffer `new` made, of `len` words.
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(self.words, self.len)) });
    }
}

// SAFETY: a reader owns its buffer; it is read (and its cache written) by one thread at a time.
unsafe impl Send for Reader {}

/// Format 15's checkpoint stride (`base`'s, a u32 every 16 nodes) and its tails-from-start flag.
pub const F15_STRIDE_SHIFT: u32 = 4;
pub const F15_TAIL_FROM_START: u32 = 1 << 12;

/// A field's byte offset in a view.
pub mod at {
    use super::TapeView;
    use std::mem::offset_of;
    pub const NODES: i32 = offset_of!(TapeView, nodes) as i32;
    pub const FORMAT: i32 = offset_of!(TapeView, format) as i32;
    pub const WIDTHS: i32 = offset_of!(TapeView, widths) as i32;
    pub const COLUMNS: i32 = offset_of!(TapeView, columns) as i32;
    pub const STRIDE: i32 = offset_of!(TapeView, stride) as i32;
    pub const SIDE: i32 = offset_of!(TapeView, side) as i32;
    pub const PARTS: i32 = offset_of!(TapeView, parts) as i32;
    pub const CUT: i32 = offset_of!(TapeView, cut) as i32;
    pub const PER: i32 = offset_of!(TapeView, per) as i32;
    pub const LINES: i32 = offset_of!(TapeView, lines) as i32;
    pub const LINE_COUNT: i32 = offset_of!(TapeView, line_count) as i32;
    pub const SOURCE_LEN: i32 = offset_of!(TapeView, source_len) as i32;
}

#[cfg(test)]
mod ukl_tests {
    /// The UKL lists every field: their words add up to the view's.
    #[test]
    fn the_views_ukl_names_every_word() {
        let msl = super::ukl();
        let words: usize = msl
            .lines()
            .filter_map(|l| l.trim().strip_prefix("uint32_t "))
            .map(|f| f.trim_end_matches(';').split_once('[').map_or(1, |(_, n)| n.trim_end_matches(']').parse::<usize>().unwrap()))
            .sum();
        assert_eq!(4 * words, std::mem::size_of::<super::TapeView>(), "{msl}");
    }
}
