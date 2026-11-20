//! A space's kept facts (decisions 0401, 0456): the relations its last runs derived, each with what a run checks
//! before it binds one (its rows' digest, the relations it was derived from and their digests, its columns' value
//! types), in a chain of frozen files beside the store, mapped by every process that reads the space.
//!
//! Each file is a block whose root holds a directory of entries sorted by key (`rel`, `ident`, `part`): an entry is
//! found by a binary search in place, and a relation's entries (its documents' and its granules') are adjacent. An
//! entry's rows are a `packed::Packed` (0456 K1): its live rows in canonical order, each column an `IntCol` in the
//! bits its values need, and its rows' levels. A packed relation is one contiguous range of its file with relative
//! references only, so a roll-up moves it by `memcpy`.
//!
//! A space's chain is appended to (0456 K4): a commit writes one file of what it puts, never rewriting an older one.
//! An entry is the relation's rows, or a gone entry (no longer kept), or a reads-only entry (new reads, the rows the
//! newest older entry's). `KeptChain` reads the files as one, newest first.

use crate::block::{Block, FROZEN, Flush, kind};
use crate::packed::{Packed, Prepared};
use crate::refs::{Ptr, Span};
use crate::structure;
use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

structure! {
    /// A kept file's root.
    pub struct KeptRoot {
        /// Its maker's tag, which the store's row of it records too: a file not the row's is refused.
        pub tag: u64,
        pub entries: Span<KeptEntry>,
        pub reads: Span<ReadDigest>,
        /// The names of relations, and the entries' value types.
        pub strings: Span<u8>,
    }
}

/// An entry's rows are its own.
pub const ROWS: u32 = 0;
/// The relation is no longer kept.
pub const GONE: u32 = 1;
/// The rows are the newest older entry's of the same key; the reads are this entry's.
pub const SAME: u32 = 2;

structure! {
    /// One kept relation.
    pub struct KeptEntry {
        pub ident: [u8; 32],
        pub part: i64,
        pub digest: u64,
        pub rel_at: u32,
        pub rel_len: u32,
        pub reads_at: u32,
        pub nreads: u32,
        pub types_at: u32,
        pub ntypes: u32,
        /// `ROWS`, `GONE` or `SAME`.
        pub what: u32,
        pub pad: u32,
        pub rows: Ptr<Packed>,
        /// Its rows' range of the file (`ROWS`): what a roll-up copies.
        pub rows_at: u64,
        pub rows_len: u64,
    }
}

structure! {
    /// A relation an entry was derived from, and that read's digest.
    pub struct ReadDigest {
        pub rel_at: u32,
        pub rel_len: u32,
        pub digest: u64,
    }
}

/// A kept relation's key: its relation's name, its identity, its partition (`0` whole, then its documents' and its
/// granules').
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FactKey<'a> {
    pub rel: &'a str,
    pub ident: &'a [u8; 32],
    pub part: i64,
}

/// A kept relation, where it lies: its key, what a run checks of it, and its rows.
#[derive(Clone, Copy)]
pub struct Kept<'b> {
    pub key: FactKey<'b>,
    pub digest: u64,
    /// Each column's value type.
    pub types: &'b [u8],
    pub rows: &'b Packed,
    reads: &'b [ReadDigest],
    strings: &'b [u8],
    /// The file its rows lie in, and their range there.
    file: &'b Block,
    range: (usize, usize),
}

impl<'b> Kept<'b> {
    /// The relations it was derived from, each with that read's digest.
    pub fn reads(&self) -> impl Iterator<Item = (&'b str, u64)> + 'b {
        let strings = self.strings;
        self.reads.iter().map(move |r| (str_at(strings, r.rel_at, r.rel_len), r.digest))
    }

    /// Its rows' bytes in their file.
    pub fn bytes(&self) -> usize {
        self.range.1
    }

    /// Row `i` (in canonical order).
    pub fn row(&self, i: usize) -> Vec<i64> {
        (0..self.rows.arity as usize).map(|c| self.rows.get(i, c)).collect()
    }
}

/// What reads a space's kept facts.
pub trait KeptRead {
    fn get(&self, key: &FactKey) -> Option<Kept<'_>>;
    /// Every entry for `rel`, in key order (its parts).
    fn of<'s>(&'s self, rel: &str) -> Box<dyn Iterator<Item = Kept<'s>> + 's>;
    /// Every entry, in key order.
    fn all(&self) -> Box<dyn Iterator<Item = Kept<'_>> + '_>;
}

fn str_at(strings: &[u8], at: u32, len: u32) -> &str {
    // Written from `&str`s by the file's maker; a file is checked by its tag before it is read.
    std::str::from_utf8(&strings[at as usize..(at + len) as usize]).unwrap_or("")
}

/// A frozen kept file, mapped read-only.
///
/// It holds its block as `B`: its own mapping (`Block`, the default), or a holder that owns the mapping for another
/// reader too.
pub struct KeptFacts<B: Borrow<Block> = Block> {
    block: B,
}

// SAFETY: a frozen block's mapping, read only.
unsafe impl<B: Borrow<Block>> Send for KeptFacts<B> {}
unsafe impl<B: Borrow<Block>> Sync for KeptFacts<B> {}

/// An entry as one file holds it.
#[derive(Clone, Copy)]
enum Found<'b> {
    Kept(Kept<'b>),
    Gone,
    /// A reads-only entry: what it records, its rows an older file's.
    Same { digest: u64, types: &'b [u8], reads: &'b [ReadDigest], strings: &'b [u8] },
}

impl KeptFacts {
    /// The kept file at `path`, which the store's row records under `tag`: its kind, its state (frozen) and its tag
    /// checked.
    pub fn open(path: &Path, tag: u64) -> io::Result<KeptFacts> {
        let block = Block::map_file(path, kind::KEPT, false)?;
        KeptFacts::over(block, tag).map_err(|what| io::Error::new(io::ErrorKind::InvalidData, format!("{}: {what}", path.display())))
    }
}

impl<B: Borrow<Block>> KeptFacts<B> {
    /// A kept file held as `block` (mapped by `Block::map_file`, kind `KEPT`), checked: frozen, and tagged `tag`.
    pub fn over(block: B, tag: u64) -> Result<KeptFacts<B>, String> {
        let b = block.borrow();
        if b.header().kind != kind::KEPT {
            return Err(format!("a block of kind {}, not kept facts", b.header().kind));
        }
        if b.header().state != FROZEN {
            return Err("not frozen".into());
        }
        let got = b.root::<KeptRoot>().tag;
        if got != tag {
            return Err(format!("its tag is {got:016x}, the store's row says {tag:016x}"));
        }
        Ok(KeptFacts { block })
    }

    /// Its holder of the block, given back.
    pub fn into_inner(self) -> B {
        self.block
    }

    /// Its block.
    pub fn block(&self) -> &Block {
        self.block.borrow()
    }

    fn root(&self) -> &KeptRoot {
        self.block.borrow().root()
    }

    pub fn tag(&self) -> u64 {
        self.root().tag
    }

    /// The bytes of its file.
    pub fn bytes(&self) -> usize {
        self.block.borrow().header().length as usize
    }

    /// Its entries' count, of every kind.
    pub fn len(&self) -> usize {
        self.entries().len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries().is_empty()
    }

    /// Whether `rows` lie in this file's mapping.
    pub fn holds(&self, rows: &Packed) -> bool {
        let p = std::ptr::from_ref(rows) as usize;
        let base = self.block.borrow().base_ptr() as usize;
        p >= base && p < base + self.block.borrow().reserved()
    }

    fn entries(&self) -> &[KeptEntry] {
        self.root().entries.as_slice()
    }

    fn strings(&self) -> &[u8] {
        self.root().strings.as_slice()
    }

    fn key_of<'s>(&'s self, e: &'s KeptEntry) -> FactKey<'s> {
        FactKey { rel: str_at(self.strings(), e.rel_at, e.rel_len), ident: &e.ident, part: e.part }
    }

    fn entry<'s>(&'s self, e: &'s KeptEntry) -> Found<'s> {
        let reads = &self.root().reads.as_slice()[e.reads_at as usize..(e.reads_at + e.nreads) as usize];
        let types = &self.strings()[e.types_at as usize..(e.types_at + e.ntypes) as usize];
        match e.what {
            GONE => Found::Gone,
            SAME => Found::Same { digest: e.digest, types, reads, strings: self.strings() },
            _ => {
                let rows = e.rows.get().expect("a kept entry has rows");
                let range = (e.rows_at as usize, e.rows_len as usize);
                Found::Kept(Kept { key: self.key_of(e), digest: e.digest, types, rows, reads, strings: self.strings(), file: self.block(), range })
            }
        }
    }

    fn find(&self, key: &FactKey) -> Option<(FactKey<'_>, Found<'_>)> {
        let i = self.entries().binary_search_by(|e| self.key_of(e).cmp(key)).ok()?;
        let e = &self.entries()[i];
        Some((self.key_of(e), self.entry(e)))
    }

    /// The entries of `rel`, by a binary search for their range.
    fn range(&self, rel: &str) -> impl Iterator<Item = (FactKey<'_>, Found<'_>)> + '_ {
        let es = self.entries();
        let lo = es.partition_point(|e| str_at(self.strings(), e.rel_at, e.rel_len) < rel);
        let hi = lo + es[lo..].partition_point(|e| str_at(self.strings(), e.rel_at, e.rel_len) == rel);
        es[lo..hi].iter().map(move |e| (self.key_of(e), self.entry(e)))
    }

    fn every(&self) -> impl Iterator<Item = (FactKey<'_>, Found<'_>)> + '_ {
        self.entries().iter().map(move |e| (self.key_of(e), self.entry(e)))
    }
}

/// A space's chain of kept files, oldest first, read as one: a key's newest entry is its entry; a gone entry hides
/// it; a reads-only entry reads the rows of the newest older entry with rows.
pub struct KeptChain<'a, B: Borrow<Block> = Block> {
    pub files: &'a [KeptFacts<B>],
}

/// `found` (a newer file's entry for `key`) with its rows: a reads-only entry's from `older` (the key's older
/// entries, newest first).
fn resolve<'b>(key: FactKey<'b>, found: Found<'b>, mut older: impl Iterator<Item = Found<'b>>) -> Option<Kept<'b>> {
    match found {
        Found::Kept(k) => Some(k),
        Found::Gone => None,
        Found::Same { digest, types, reads, strings } => {
            let rows = older.find_map(|f| match f {
                Found::Kept(k) => Some(Some(k)),
                Found::Gone => Some(None),
                Found::Same { .. } => None,
            })??;
            Some(Kept { key, digest, types, reads, strings, ..rows })
        }
    }
}

/// A key's newest entry in a run of a chain's files, as `KeptChain::newest` gives it.
pub enum Newest<'b> {
    /// Its rows lie in these files (a reads-only entry's resolved, with its own reads).
    Kept(Kept<'b>),
    Gone,
    /// A reads-only entry whose rows lie in an older file than these.
    Same(Same<'b>),
}

/// What a reads-only entry records.
#[derive(Clone, Copy)]
pub struct Same<'b> {
    pub digest: u64,
    pub types: &'b [u8],
    reads: &'b [ReadDigest],
    strings: &'b [u8],
}

impl<'b> Same<'b> {
    pub fn reads(&self) -> impl Iterator<Item = (&'b str, u64)> + 'b {
        let strings = self.strings;
        self.reads.iter().map(move |r| (str_at(strings, r.rel_at, r.rel_len), r.digest))
    }
}

impl<B: Borrow<Block>> KeptChain<'_, B> {
    /// Each key's newest entry in these files, in key order, a reads-only entry resolved where its rows lie in them:
    /// what a merge of these files (the newest of a chain, K4) writes.
    pub fn newest(&self) -> Vec<(FactKey<'_>, Newest<'_>)> {
        let mut by: BTreeMap<FactKey<'_>, Vec<Found<'_>>> = BTreeMap::new();
        for f in self.files.iter().rev() {
            for (k, found) in f.every() {
                by.entry(k).or_default().push(found);
            }
        }
        by.into_iter()
            .map(|(k, fs)| {
                let n = match fs[0] {
                    Found::Kept(kept) => Newest::Kept(kept),
                    Found::Gone => Newest::Gone,
                    Found::Same { digest, types, reads, strings } => match resolve(k, fs[0], fs.into_iter().skip(1)) {
                        Some(kept) => Newest::Kept(kept),
                        None => Newest::Same(Same { digest, types, reads, strings }),
                    },
                };
                (k, n)
            })
            .collect()
    }

    /// Every key's entries across the files, `pick`ed per file, each key's newest first, resolved.
    fn merged<'s, I>(&'s self, pick: impl Fn(&'s KeptFacts<B>) -> I) -> Vec<Kept<'s>>
    where
        I: Iterator<Item = (FactKey<'s>, Found<'s>)>,
    {
        let mut by: BTreeMap<FactKey<'s>, Vec<Found<'s>>> = BTreeMap::new();
        for f in self.files.iter().rev() {
            for (k, found) in pick(f) {
                by.entry(k).or_default().push(found);
            }
        }
        by.into_iter().filter_map(|(k, fs)| resolve(k, fs[0], fs.into_iter().skip(1))).collect()
    }

    /// The bytes of rows the chain's files hold that newer entries replaced or hid: what a roll-up would drop.
    pub fn superseded_bytes(&self) -> usize {
        let mut seen: std::collections::HashSet<FactKey> = std::collections::HashSet::new();
        let mut out = 0;
        for f in self.files.iter().rev() {
            for (k, found) in f.every() {
                let newer = !seen.insert(k);
                match found {
                    // A reads-only entry reads the next older rows: those are not superseded.
                    Found::Same { .. } => {
                        seen.remove(&k);
                    }
                    Found::Kept(kept) if newer => out += kept.bytes(),
                    _ => {}
                }
            }
        }
        out
    }

    /// The bytes of the files.
    pub fn bytes(&self) -> usize {
        self.files.iter().map(|f| f.bytes()).sum()
    }
}

impl<'a, B: Borrow<Block>> KeptChain<'a, B> {
    /// `key`'s entry, borrowed from the files, not from this chain (`KeptRead::get`, held past the chain).
    pub fn entry(&self, key: &FactKey) -> Option<Kept<'a>> {
        let files: &'a [KeptFacts<B>] = self.files;
        let mut found = files.iter().rev().filter_map(|f| f.find(key));
        let (own, first) = found.next()?;
        resolve(own, first, found.map(|(_, f)| f))
    }
}

impl<B: Borrow<Block>> KeptRead for KeptChain<'_, B> {
    fn get(&self, key: &FactKey) -> Option<Kept<'_>> {
        self.entry(key)
    }
    fn of<'s>(&'s self, rel: &str) -> Box<dyn Iterator<Item = Kept<'s>> + 's> {
        let rel = rel.to_owned();
        Box::new(self.merged(move |f| f.range(&rel).collect::<Vec<_>>().into_iter()).into_iter())
    }
    fn all(&self) -> Box<dyn Iterator<Item = Kept<'_>> + '_> {
        Box::new(self.merged(|f| f.every()).into_iter())
    }
}

/// An entry as `KeptW` holds it until `finish` writes the directory.
struct Pending {
    rel: String,
    ident: [u8; 32],
    part: i64,
    digest: u64,
    reads: Vec<(String, u64)>,
    types: Vec<u8>,
    what: u32,
    /// Its rows' range in the file (`ROWS`).
    rows: (usize, usize),
}

/// What an entry records beside its rows.
#[derive(Clone, Copy)]
pub struct Meta<'a> {
    pub key: FactKey<'a>,
    pub digest: u64,
    pub reads: &'a [(&'a str, u64)],
    pub types: &'a [u8],
}

/// What a put wrote: its rows and bytes, and whether they came in canonical order (no sort).
#[derive(Clone, Copy, Debug, Default)]
pub struct Put {
    pub rows: usize,
    pub bytes: usize,
    pub in_order: bool,
}

/// The columns a packed entry records its order on (`Packed::okey`).
const MAX_ORDER: usize = 8;

/// A kept file being written: the next of a space's chain, or a roll-up of it. Each put's rows are packed into it as
/// they are put; the directory, sorted, is written by `finish`, which freezes the file and puts it on the disk.
pub struct KeptW {
    block: Block,
    root: usize,
    pending: Vec<Pending>,
    #[cfg(not(unix))]
    path: std::path::PathBuf,
}

// SAFETY: one writer, through `&mut self`.
unsafe impl Send for KeptW {}

fn room() -> io::Error {
    io::Error::new(io::ErrorKind::StorageFull, "the kept file's room is spent")
}

impl KeptW {
    /// A new file at `path` (a name no reader opens, renamed into place once finished), tagged `tag`, with room for
    /// `reserve` bytes (its file is sparse: room not written takes no disk).
    pub fn create(path: &Path, tag: u64, reserve: usize) -> io::Result<KeptW> {
        assert_ne!(tag, 0, "a kept file's tag is not 0");
        #[cfg(unix)]
        let block = Block::file(path, kind::KEPT, reserve)?;
        #[cfg(not(unix))]
        let block = Block::anon(kind::KEPT, reserve);
        let root: *mut KeptRoot = block.try_alloc_t(1).ok_or_else(room)?;
        // SAFETY: the root just allocated in this open block, which has no other writer.
        unsafe { (*root).tag = tag };
        block.set_root(root);
        let root = block.offset_of(root);
        Ok(KeptW {
            block,
            root,
            pending: Vec::new(),
            #[cfg(not(unix))]
            path: path.to_owned(),
        })
    }

    /// Rows prepared by `packed::prepare` (K3: sorted unless they came in order, deduplicated, each column's
    /// encoding chosen), packed into this file.
    pub fn put(&mut self, meta: Meta, rows: &Prepared) -> io::Result<Put> {
        if self.block.reserved() - self.block.used() < rows.bytes() {
            return Err(room());
        }
        let (packed, stats) = rows.place_in(&self.block);
        let at = self.block.offset_of(std::ptr::from_ref(packed));
        self.pend(meta, ROWS, (at, self.block.used() - at));
        Ok(Put { rows: stats.rows, bytes: stats.bytes, in_order: rows.in_order })
    }

    /// `n` rows of `a` columns (`cols`, column-major) with each row's level when they have one, prepared and packed
    /// (`put`), in the calling thread's scratch. In canonical order (columns 0, 1, … ascending), the order recorded
    /// on its first `MAX_ORDER` columns, so a probe of a leading column is a range (`Packed::leads`); or as given.
    pub fn put_cols(&mut self, meta: Meta, cols: &[i64], n: usize, a: usize, levels: Option<&[u32]>, given: crate::packed::Order) -> io::Result<Put> {
        let order = (0..a.min(MAX_ORDER) as u16).collect();
        let spec = crate::relation::Spec { widths: vec![8; a], order, indexes: Vec::new(), min_log: 0 };
        crate::scratch::with(|s| {
            let prepared = crate::packed::prepare(s, cols, n, a, &spec, levels, given);
            self.put(meta, &prepared)
        })
    }

    /// An entry of another file of the chain, its rows copied whole (a roll-up, K4: one `memcpy`, nothing
    /// re-encoded), with `meta` in place of what it records when given (new reads, its rows the same).
    pub fn copy(&mut self, kept: &Kept, meta: Option<Meta>) -> io::Result<()> {
        let (from, len) = kept.range;
        let at = self.block.try_alloc(len, 8).ok_or_else(room)?;
        // SAFETY: `len` bytes just allocated in this open block; the source is the frozen file's range of the rows,
        // whose references are relative and all within it (`Prepared::place_in`), both 8-aligned.
        unsafe { std::ptr::copy_nonoverlapping(kept.file.base_ptr().add(from), self.block.base_ptr().add(at), len) };
        let reads: Vec<(&str, u64)> = kept.reads().collect();
        let meta = meta.unwrap_or(Meta { key: kept.key, digest: kept.digest, reads: &reads, types: kept.types });
        self.pend(meta, ROWS, (at, len));
        Ok(())
    }

    /// A reads-only entry: the relation's rows are the chain's as they are, its reads and digest these.
    pub fn same(&mut self, meta: Meta) {
        self.pend(meta, SAME, (0, 0));
    }

    /// The relation is no longer kept.
    pub fn gone(&mut self, key: FactKey) {
        self.pend(Meta { key, digest: 0, reads: &[], types: &[] }, GONE, (0, 0));
    }

    fn pend(&mut self, meta: Meta, what: u32, rows: (usize, usize)) {
        self.pending.push(Pending {
            rel: meta.key.rel.to_owned(),
            ident: *meta.key.ident,
            part: meta.key.part,
            digest: meta.digest,
            reads: meta.reads.iter().map(|&(r, d)| (r.to_owned(), d)).collect(),
            types: meta.types.to_vec(),
            what,
            rows,
        });
    }

    /// The bytes written so far.
    pub fn used(&self) -> usize {
        self.block.used()
    }

    /// The directory written, sorted by key, then the file frozen (truncated to what it holds) and written back as
    /// `flush` says (decision 0481). A key put twice is refused.
    pub fn finish(mut self, flush: Flush) -> io::Result<()> {
        self.pending.sort_by(|a, b| (a.rel.as_str(), &a.ident, a.part).cmp(&(b.rel.as_str(), &b.ident, b.part)));
        if let Some(w) = self.pending.windows(2).find(|w| (w[0].rel.as_str(), &w[0].ident, w[0].part) == (w[1].rel.as_str(), &w[1].ident, w[1].part)) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("kept relation {} part {} put twice", w[0].rel, w[0].part)));
        }
        // The strings: each relation's name once, then each entry's types.
        let mut strings: Vec<u8> = Vec::new();
        let mut names: std::collections::HashMap<String, (u32, u32)> = std::collections::HashMap::new();
        let mut name = |s: &str, strings: &mut Vec<u8>| {
            *names.entry(s.to_owned()).or_insert_with(|| {
                let at = strings.len() as u32;
                strings.extend_from_slice(s.as_bytes());
                (at, s.len() as u32)
            })
        };
        let mut entries: Vec<(u32, u32, u32, u32)> = Vec::with_capacity(self.pending.len());
        let mut reads: Vec<(u32, u32, u64)> = Vec::new();
        for p in &self.pending {
            let (rel_at, rel_len) = name(&p.rel, &mut strings);
            let reads_at = reads.len() as u32;
            for (r, d) in &p.reads {
                let (at, len) = name(r, &mut strings);
                reads.push((at, len, *d));
            }
            let types_at = strings.len() as u32;
            strings.extend_from_slice(&p.types);
            entries.push((rel_at, rel_len, reads_at, types_at));
        }
        let es: *mut KeptEntry = self.block.try_alloc_t(self.pending.len().max(1)).ok_or_else(room)?;
        let rs: *mut ReadDigest = self.block.try_alloc_t(reads.len().max(1)).ok_or_else(room)?;
        let ss: *mut u8 = self.block.try_alloc_t(strings.len().max(1)).ok_or_else(room)?;
        let root: *mut KeptRoot = self.block.ptr(self.root);
        // SAFETY: the arrays were just allocated in this open block, sized as written; the root is its own.
        unsafe {
            std::ptr::copy_nonoverlapping(strings.as_ptr(), ss, strings.len());
            for (i, &(at, len, d)) in reads.iter().enumerate() {
                let r = &mut *rs.add(i);
                r.rel_at = at;
                r.rel_len = len;
                r.digest = d;
            }
            for (i, (p, &(rel_at, rel_len, reads_at, types_at))) in self.pending.iter().zip(&entries).enumerate() {
                let e = es.add(i);
                (*e).ident = p.ident;
                (*e).part = p.part;
                (*e).digest = p.digest;
                (*e).rel_at = rel_at;
                (*e).rel_len = rel_len;
                (*e).reads_at = reads_at;
                (*e).nreads = p.reads.len() as u32;
                (*e).types_at = types_at;
                (*e).ntypes = p.types.len() as u32;
                (*e).what = p.what;
                if p.what == ROWS {
                    Ptr::set(&raw const (*e).rows, self.block.ptr::<Packed>(p.rows.0));
                    (*e).rows_at = p.rows.0 as u64;
                    (*e).rows_len = p.rows.1 as u64;
                }
            }
            Span::set(&raw const (*root).entries, es, self.pending.len());
            Span::set(&raw const (*root).reads, rs, reads.len());
            Span::set(&raw const (*root).strings, ss, strings.len());
        }
        #[cfg(unix)]
        {
            self.block.sync_mapped(flush)?;
            let frozen = self.block.freeze();
            frozen.sync_mapped(flush)?;
        }
        #[cfg(not(unix))]
        {
            use std::io::Write as _;
            let frozen = self.block.freeze();
            let mut f = std::fs::File::create(&self.path)?;
            f.write_all(frozen.bytes(0, frozen.header().length as usize))?;
            if flush.waits() {
                f.sync_data()?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("rerail-kept-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn meta(key: FactKey<'_>, digest: u64) -> Meta<'_> {
        Meta { key, digest, reads: &[], types: &[] }
    }

    /// Rows of two columns, as columns.
    fn put(w: &mut KeptW, m: Meta, rows: &[(i64, i64)], levels: Option<&[u32]>) -> Put {
        let cols: Vec<i64> = rows.iter().map(|r| r.0).chain(rows.iter().map(|r| r.1)).collect();
        w.put_cols(m, &cols, rows.len(), 2, levels, crate::packed::Order::Canonical).unwrap()
    }

    fn rows_of(k: &Kept) -> Vec<Vec<i64>> {
        (0..k.rows.len()).map(|i| k.row(i)).collect()
    }

    const A: [u8; 32] = [1; 32];
    const B: [u8; 32] = [2; 32];

    /// A kept relation reads from a mapping of its file, its columns lying in that mapping: its rows in canonical
    /// order, its levels following them, what a run checks.
    #[test]
    fn a_kept_relation_is_read_where_it_lies() {
        let d = dir("lies");
        let data: Vec<(i64, i64)> = (0..5_000).map(|i| ((i * 7919) % 5_003, i % 13)).collect();
        let levels: Vec<u32> = (0..5_000).map(|i| i % 4).collect();
        let mut w = KeptW::create(&d.join("c1.blk"), 11, 64 << 20).unwrap();
        let reads = [("defs", 5), ("uses", 6)];
        let p = put(&mut w, Meta { reads: &reads, types: b"ii", ..meta(FactKey { rel: "calls", ident: &A, part: 0 }, 99) }, &data, Some(&levels));
        assert!(!p.in_order && p.rows == 5_000);
        assert!(p.bytes < 5_000 * 16 / 3, "{} bytes for 5,000 rows of two columns", p.bytes);
        w.finish(Flush::Synced).unwrap();
        let files = [KeptFacts::open(&d.join("c1.blk"), 11).unwrap()];
        let chain = KeptChain { files: &files };
        let got = chain.get(&FactKey { rel: "calls", ident: &A, part: 0 }).unwrap();
        assert!(files[0].holds(got.rows), "the rows lie in the kept file's mapping");
        assert_eq!((got.digest, got.types), (99, &b"ii"[..]));
        assert_eq!(got.reads().collect::<Vec<_>>(), [("defs", 5), ("uses", 6)]);
        let mut want: Vec<Vec<i64>> = data.iter().map(|r| vec![r.0, r.1]).collect();
        want.sort();
        assert_eq!(rows_of(&got), want);
        let lv = got.rows.levels().unwrap();
        for (i, row) in rows_of(&got).iter().enumerate() {
            let orig = data.iter().position(|r| (r.0, r.1) == (row[0], row[1])).unwrap();
            assert_eq!(lv.get(i), levels[orig] as i64);
        }
        assert!(chain.get(&FactKey { rel: "calls", ident: &B, part: 0 }).is_none());
        assert!(KeptFacts::open(&d.join("c1.blk"), 12).is_err(), "a file not the row's is refused");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// K-R4 (decision 0456): a later file's entry replaces an earlier one's, a gone entry hides it, a reads-only
    /// entry reads the earlier rows with its own reads; a relation's parts come in key order; a roll-up copies the
    /// live entries whole and reads the same.
    #[test]
    fn a_chain_reads_newest_first() {
        let d = dir("chain");
        let key = |rel, ident, part| FactKey { rel, ident, part };
        let mut w = KeptW::create(&d.join("c1.blk"), 1, 16 << 20).unwrap();
        put(&mut w, meta(key("p", &A, 0), 1), &[(1, 1)], None);
        put(&mut w, meta(key("p", &A, 7), 2), &[(2, 2)], None);
        put(&mut w, meta(key("q", &B, 0), 3), &[(3, 3), (4, 4)], Some(&[5, 6]));
        w.finish(Flush::Synced).unwrap();
        let mut w = KeptW::create(&d.join("c2.blk"), 2, 16 << 20).unwrap();
        put(&mut w, meta(key("p", &A, 7), 20), &[(3, 3)], None);
        w.gone(key("p", &A, 0));
        put(&mut w, meta(key("p", &A, 3), 30), &[(1, 1)], None);
        w.same(Meta { reads: &[("x", 9)], ..meta(key("q", &B, 0), 33) });
        w.finish(Flush::Synced).unwrap();
        let files = [KeptFacts::open(&d.join("c1.blk"), 1).unwrap(), KeptFacts::open(&d.join("c2.blk"), 2).unwrap()];
        let v = KeptChain { files: &files };
        assert_eq!(v.of("p").map(|k| (k.key.part, k.digest)).collect::<Vec<_>>(), [(3, 30), (7, 20)]);
        assert!(v.get(&key("p", &A, 0)).is_none(), "gone in the later file");
        assert_eq!(rows_of(&v.get(&key("p", &A, 7)).unwrap()), [vec![3, 3]]);
        let q = v.get(&key("q", &B, 0)).unwrap();
        assert_eq!((q.digest, q.reads().collect::<Vec<_>>(), rows_of(&q)), (33, vec![("x", 9)], vec![vec![3, 3], vec![4, 4]]));
        assert!(files[0].holds(q.rows), "a reads-only entry's rows are the earlier file's");
        assert_eq!(v.all().count(), 3);
        assert_eq!(KeptChain { files: &files[..1] }.all().count(), 3, "the first file alone is as it was");
        assert!(v.superseded_bytes() > 0, "p 0 and p 7 of the first file");
        // The roll-up: each live entry copied whole.
        let mut w = KeptW::create(&d.join("c3.blk"), 3, 16 << 20).unwrap();
        for k in v.all() {
            w.copy(&k, None).unwrap();
        }
        w.finish(Flush::Synced).unwrap();
        let rolled = [KeptFacts::open(&d.join("c3.blk"), 3).unwrap()];
        let r = KeptChain { files: &rolled };
        type Flat = Vec<(String, i64, u64, Vec<(String, u64)>, Vec<Vec<i64>>, Option<Vec<i64>>)>;
        let flat = |c: &dyn KeptRead| -> Flat {
            c.all()
                .map(|k| {
                    let reads = k.reads().map(|(a, b)| (a.to_owned(), b)).collect();
                    let levels = k.rows.levels().map(|l| (0..l.len()).map(|i| l.get(i)).collect());
                    (k.key.rel.to_owned(), k.key.part, k.digest, reads, rows_of(&k), levels)
                })
                .collect()
        };
        assert_eq!(flat(&r), flat(&v));
        assert_eq!(r.superseded_bytes(), 0);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A key put twice in one file is refused.
    #[test]
    fn a_key_put_twice_is_refused() {
        let d = dir("twice");
        let mut w = KeptW::create(&d.join("c1.blk"), 1, 16 << 20).unwrap();
        put(&mut w, meta(FactKey { rel: "p", ident: &A, part: 0 }, 1), &[(1, 1)], None);
        put(&mut w, meta(FactKey { rel: "p", ident: &A, part: 0 }, 1), &[(1, 1)], None);
        assert!(w.finish(Flush::Synced).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
