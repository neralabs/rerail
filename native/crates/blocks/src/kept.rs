//! A space's kept facts (decision 0401 F1–F3): the relations its last runs derived, each with what a run checks
//! before it binds one (its rows' digest, the relations it was derived from and their digests, its columns' value
//! types), in a block in a file beside the store, mapped by every process that opens it.
//!
//! The root holds a directory of entries sorted by key (`rel`, `ident`, `part`): an entry is found by a binary search
//! in place, and a relation's entries (its documents' and its granules') are adjacent. Each entry's rows are a frozen
//! `relation::Relation` in the same block, copied whole from the run's own block (`Relation::try_copy_into`): its
//! columns' chunks, canonical order, hash indexes and statistics as freeze made them. A reader binds one as a view
//! over the mapping; no row is decoded or copied.
//!
//! A space keeps a base and one cumulative delta. A delta's entries replace the base's of the same key, and a delta
//! records a relation the base holds and the space no longer keeps as a gone entry. `KeptView` reads the two as one.
//! A block is written once, by `KeptW`, under a name no reader opens, and frozen before the store's rows name it.

use crate::block::{Block, FROZEN, kind};
use crate::refs::{Ptr, Span};
use crate::relation::{Relation, View};
use crate::structure;
use std::cmp::Ordering;
use std::io;
use std::path::Path;

structure! {
    /// A kept block's root.
    pub struct KeptRoot {
        /// Its maker's tag, which the store's row of it records too: a block not the row's is refused.
        pub tag: u64,
        /// In a delta, the tag of the base it is read over; 0 in a base.
        pub base_tag: u64,
        pub entries: Span<KeptEntry>,
        pub reads: Span<ReadDigest>,
        /// The names of relations, and the entries' value types.
        pub strings: Span<u8>,
    }
}

structure! {
    /// One kept relation (or, in a delta, one the space no longer keeps: `gone`).
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
        pub gone: u32,
        pub pad: u32,
        pub rows: Ptr<Relation>,
        /// Its dead rows (bit `id`), as the run kept them beside its rows; empty when none is.
        pub dead: Span<u64>,
        /// Each row's level (decision 0090), when the run derived them; empty otherwise.
        pub levels: Span<u32>,
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
    pub rows: &'b Relation,
    /// Its dead rows (bit `id`); empty when none is.
    pub dead: &'b [u64],
    /// Each row's level; empty when the run derived none.
    pub levels: &'b [u32],
    reads: &'b [ReadDigest],
    strings: &'b [u8],
}

impl<'b> Kept<'b> {
    /// The relations it was derived from, each with that read's digest.
    pub fn reads(&self) -> impl Iterator<Item = (&'b str, u64)> + 'b {
        let strings = self.strings;
        self.reads.iter().map(move |r| (str_at(strings, r.rel_at, r.rel_len), r.digest))
    }

    /// Its rows, read where they lie.
    pub fn view(&self) -> View<'b> {
        View::full(self.rows)
    }
}

/// What reads a space's kept facts: one block, or a base and its delta read as one.
pub trait KeptRead {
    fn get(&self, key: &FactKey) -> Option<Kept<'_>>;
    /// Every entry for `rel`, in key order (its parts).
    fn of<'s>(&'s self, rel: &str) -> Box<dyn Iterator<Item = Kept<'s>> + 's>;
    /// Every entry, in key order.
    fn all(&self) -> Box<dyn Iterator<Item = Kept<'_>> + '_>;
}

fn str_at(strings: &[u8], at: u32, len: u32) -> &str {
    // Written from `&str`s by the block's maker; a block is checked by its tag before it is read.
    std::str::from_utf8(&strings[at as usize..(at + len) as usize]).unwrap_or("")
}

/// A frozen kept block, mapped read-only from its file.
pub struct KeptFacts {
    block: Block,
}

// SAFETY: a frozen block's mapping, read only.
unsafe impl Send for KeptFacts {}
unsafe impl Sync for KeptFacts {}

/// An entry as `KeptFacts::find` gives it: kept, or gone (a delta's).
enum Found<'b> {
    Kept(Kept<'b>),
    Gone,
}

impl KeptFacts {
    /// The kept block at `path`, which the store's row records under `tag`: its kind, its state (frozen) and its tag
    /// checked.
    pub fn open(path: &Path, tag: u64) -> io::Result<KeptFacts> {
        let block = Block::map_file(path, kind::KEPT, false)?;
        let invalid = |what: String| io::Error::new(io::ErrorKind::InvalidData, format!("{}: {what}", path.display()));
        if block.header().state != FROZEN {
            return Err(invalid("not frozen".into()));
        }
        let got = block.root::<KeptRoot>().tag;
        if got != tag {
            return Err(invalid(format!("its tag is {got:016x}, the store's row says {tag:016x}")));
        }
        Ok(KeptFacts { block })
    }

    fn root(&self) -> &KeptRoot {
        self.block.root()
    }

    pub fn tag(&self) -> u64 {
        self.root().tag
    }

    /// Its block, mapped read-only: a reader that binds blocks by value shares it (`Block::share`).
    pub fn block(&self) -> &Block {
        &self.block
    }

    /// The tag of the base this delta is read over; `None` for a base.
    pub fn base_tag(&self) -> Option<u64> {
        Some(self.root().base_tag).filter(|&t| t != 0)
    }

    /// The bytes of its file (what a roll-up rule weighs).
    pub fn bytes(&self) -> usize {
        self.block.header().length as usize
    }

    /// Whether `rows` lie in this block's mapping.
    pub fn holds(&self, rows: &Relation) -> bool {
        let p = std::ptr::from_ref(rows) as usize;
        let base = self.block.base_ptr() as usize;
        p >= base && p < base + self.block.reserved()
    }

    /// The keys this delta records as gone (none in a base).
    pub fn gone_keys(&self) -> impl Iterator<Item = FactKey<'_>> + '_ {
        self.entries().iter().filter(|e| e.gone != 0).map(move |e| self.key_of(e))
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
        if e.gone != 0 {
            return Found::Gone;
        }
        let reads = &self.root().reads.as_slice()[e.reads_at as usize..(e.reads_at + e.nreads) as usize];
        let types = &self.strings()[e.types_at as usize..(e.types_at + e.ntypes) as usize];
        let rows = e.rows.get().expect("a kept entry has rows");
        Found::Kept(Kept { key: self.key_of(e), digest: e.digest, types, rows, dead: e.dead.as_slice(), levels: e.levels.as_slice(), reads, strings: self.strings() })
    }

    fn find(&self, key: &FactKey) -> Option<Found<'_>> {
        let i = self.entries().binary_search_by(|e| self.key_of(e).cmp(key)).ok()?;
        Some(self.entry(&self.entries()[i]))
    }

    /// The entries of `rel`, kept and gone, by a binary search for their range.
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

fn kept_only<'b>(it: impl Iterator<Item = (FactKey<'b>, Found<'b>)> + 'b) -> impl Iterator<Item = Kept<'b>> + 'b {
    it.filter_map(|(_, f)| match f {
        Found::Kept(k) => Some(k),
        Found::Gone => None,
    })
}

impl KeptRead for KeptFacts {
    fn get(&self, key: &FactKey) -> Option<Kept<'_>> {
        match self.find(key)? {
            Found::Kept(k) => Some(k),
            Found::Gone => None,
        }
    }
    fn of<'s>(&'s self, rel: &str) -> Box<dyn Iterator<Item = Kept<'s>> + 's> {
        let rel = rel.to_owned();
        Box::new(kept_only(self.range(&rel).collect::<Vec<_>>().into_iter()))
    }
    fn all(&self) -> Box<dyn Iterator<Item = Kept<'_>> + '_> {
        Box::new(kept_only(self.every()))
    }
}

/// A space's base and its delta, read as one: the delta's entry for a key in place of the base's, and a key the
/// delta records as gone not kept.
pub struct KeptView<'a> {
    pub base: Option<&'a KeptFacts>,
    pub delta: Option<&'a KeptFacts>,
}

/// Two key-ordered runs of entries merged, the second's (the delta's) taking a key both hold.
fn merged<'b>(a: Vec<(FactKey<'b>, Found<'b>)>, b: Vec<(FactKey<'b>, Found<'b>)>) -> Vec<Kept<'b>> {
    let mut out = Vec::with_capacity(a.len() + b.len());
    let (mut a, mut b) = (a.into_iter().peekable(), b.into_iter().peekable());
    loop {
        let take_b = match (a.peek(), b.peek()) {
            (None, None) => break,
            (Some(_), None) => false,
            (None, Some(_)) => true,
            (Some((ka, _)), Some((kb, _))) => match ka.cmp(kb) {
                Ordering::Less => false,
                Ordering::Greater => true,
                Ordering::Equal => {
                    a.next();
                    true
                }
            },
        };
        let (_, f) = if take_b { b.next() } else { a.next() }.expect("peeked");
        if let Found::Kept(k) = f {
            out.push(k);
        }
    }
    out
}

impl<'a> KeptView<'a> {
    /// `get`, the entry borrowed from the blocks, not from this view.
    pub fn find(&self, key: &FactKey) -> Option<Kept<'a>> {
        if let Some(d) = self.delta {
            match d.find(key) {
                Some(Found::Kept(k)) => return Some(k),
                Some(Found::Gone) => return None,
                None => {}
            }
        }
        self.base?.get(key)
    }
}

impl KeptRead for KeptView<'_> {
    fn get(&self, key: &FactKey) -> Option<Kept<'_>> {
        self.find(key)
    }
    fn of<'s>(&'s self, rel: &str) -> Box<dyn Iterator<Item = Kept<'s>> + 's> {
        let a = self.base.map(|b| b.range(rel).collect()).unwrap_or_default();
        let b = self.delta.map(|d| d.range(rel).collect()).unwrap_or_default();
        Box::new(merged(a, b).into_iter())
    }
    fn all(&self) -> Box<dyn Iterator<Item = Kept<'_>> + '_> {
        let a = self.base.map(|b| b.every().collect()).unwrap_or_default();
        let b = self.delta.map(|d| d.every().collect()).unwrap_or_default();
        Box::new(merged(a, b).into_iter())
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
    /// Its rows' copy's offset in the block; `None` for a gone entry.
    rows: Option<usize>,
    /// Its dead bits' and levels' copies in the block: (offset, count).
    dead: (usize, usize),
    levels: (usize, usize),
}

/// What an entry records beside its rows.
#[derive(Clone, Copy)]
pub struct Meta<'a> {
    pub key: FactKey<'a>,
    pub digest: u64,
    pub reads: &'a [(&'a str, u64)],
    pub types: &'a [u8],
    pub dead: &'a [u64],
    pub levels: &'a [u32],
}

/// A kept block being written: a space's next base or delta. Its rows are copied in as they are put; the directory,
/// sorted, is written by `finish`, which freezes the block and puts it on the disk.
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
    io::Error::new(io::ErrorKind::StorageFull, "the kept block's room is spent")
}

fn full(path: &Path) -> io::Error {
    io::Error::new(io::ErrorKind::StorageFull, format!("{}: the kept block's room is spent", path_name(path)))
}

fn path_name(path: &Path) -> String {
    path.display().to_string()
}

impl KeptW {
    /// A new block at `path` (a name no reader opens, renamed into place once finished), tagged `tag`, with room for
    /// `reserve` bytes (its file is sparse: room not written takes no disk). `base_tag` names the base a delta is read
    /// over; `None` for a base.
    pub fn create(path: &Path, tag: u64, base_tag: Option<u64>, reserve: usize) -> io::Result<KeptW> {
        assert_ne!(tag, 0, "a kept block's tag is not 0");
        #[cfg(unix)]
        let block = Block::file(path, kind::KEPT, reserve)?;
        #[cfg(not(unix))]
        let block = Block::anon(kind::KEPT, reserve);
        let root: *mut KeptRoot = block.try_alloc_t(1).ok_or_else(|| full(path))?;
        // SAFETY: the root just allocated in this open block, which has no other writer.
        unsafe {
            (*root).tag = tag;
            (*root).base_tag = base_tag.unwrap_or(0);
        }
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

    /// A relation a run derived (frozen, or a part nothing writes now), copied whole into this one, its columns
    /// `map`ped where they lie in the copy (a forked process's atom ids to the store's, 0401 F6): `None` maps none.
    /// The copy is returned.
    pub fn put(&mut self, meta: Meta, rows: &Relation, map: Option<(&[usize], &dyn Fn(i64) -> i64)>) -> io::Result<&Relation> {
        let copy = rows.try_copy_into(&self.block).ok_or_else(room)?;
        if let Some((cols, f)) = map {
            for &c in cols {
                // SAFETY: the copy is this writer's, in its open block, read by no one before `finish`.
                unsafe { copy.rows.map_col(c, f) };
            }
        }
        let at = self.block.offset_of(std::ptr::from_ref(copy));
        self.pend(meta, Some(at))?;
        // SAFETY: the copy lies in this writer's block, which outlives the borrow of `self`.
        Ok(unsafe { &*self.block.ptr::<Relation>(at) })
    }

    /// Rows not held as a relation (an input's granule, read from its sources): `n` rows of `widths.len()` columns,
    /// flat, pushed into a relation of this block. No index is built.
    pub fn put_rows(&mut self, meta: Meta, widths: &[u32], rows: &[i64], n: usize) -> io::Result<()> {
        let spec = crate::relation::Spec { widths: widths.to_vec(), order: Vec::new(), indexes: Vec::new(), min_log: crate::rows::MIN_LOG };
        let rel = Relation::try_create(&self.block, &spec, None).ok_or_else(room)?;
        if !rel.rows.try_reserve(&self.block, n as u64) {
            return Err(room());
        }
        if !widths.is_empty() {
            for (c, _) in widths.iter().enumerate() {
                let col: Vec<i64> = rows.chunks_exact(widths.len()).take(n).map(|r| r[c]).collect();
                rel.rows.write_col(c, &col);
            }
        }
        rel.rows.len.store(n as u32, std::sync::atomic::Ordering::Release);
        rel.live.store(n as u32, std::sync::atomic::Ordering::Relaxed);
        let at = self.block.offset_of(std::ptr::from_ref(rel));
        self.pend(meta, Some(at))
    }

    /// An entry of the previous delta (or base), kept as it is, `reads` in place of its own when given (its rows the
    /// same, what they were derived from read again): its rows copied whole.
    pub fn carry(&mut self, kept: &Kept, reads: Option<&[(&str, u64)]>) -> io::Result<()> {
        let own: Vec<(&str, u64)> = kept.reads().collect();
        let meta = Meta { key: kept.key, digest: kept.digest, reads: reads.unwrap_or(&own), types: kept.types, dead: kept.dead, levels: kept.levels };
        self.put(meta, kept.rows, None).map(|_| ())
    }

    /// In a delta: the base's entry for `key` is no longer kept.
    pub fn gone(&mut self, key: FactKey) {
        let none = Meta { key, digest: 0, reads: &[], types: &[], dead: &[], levels: &[] };
        self.pend(none, None).expect("a gone entry takes no room");
    }

    fn pend(&mut self, meta: Meta, rows: Option<usize>) -> io::Result<()> {
        let dead = self.copy_slice(meta.dead)?;
        let levels = self.copy_slice(meta.levels)?;
        self.pending.push(Pending {
            rel: meta.key.rel.to_owned(),
            ident: *meta.key.ident,
            part: meta.key.part,
            digest: meta.digest,
            reads: meta.reads.iter().map(|&(r, d)| (r.to_owned(), d)).collect(),
            types: meta.types.to_vec(),
            rows,
            dead,
            levels,
        });
        Ok(())
    }

    /// `items` copied into the block: (offset, count); none for none.
    fn copy_slice<T: Copy>(&self, items: &[T]) -> io::Result<(usize, usize)> {
        if items.is_empty() {
            return Ok((0, 0));
        }
        let p: *mut T = self.block.try_alloc_t(items.len()).ok_or_else(room)?;
        // SAFETY: `p` holds `items.len()` items, just allocated in this open block.
        unsafe { std::ptr::copy_nonoverlapping(items.as_ptr(), p, items.len()) };
        Ok((self.block.offset_of(p), items.len()))
    }

    /// The directory written, sorted by key, then the block frozen (its file truncated to what it holds) and put on
    /// the disk. A key put twice is refused.
    pub fn finish(mut self) -> io::Result<()> {
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
                match p.rows {
                    Some(at) => Ptr::set(&raw const (*e).rows, self.block.ptr::<Relation>(at)),
                    None => (*e).gone = 1,
                }
                if p.dead.1 > 0 {
                    Span::set(&raw const (*e).dead, self.block.ptr::<u64>(p.dead.0), p.dead.1);
                }
                if p.levels.1 > 0 {
                    Span::set(&raw const (*e).levels, self.block.ptr::<u32>(p.levels.0), p.levels.1);
                }
            }
            Span::set(&raw const (*root).entries, es, self.pending.len());
            Span::set(&raw const (*root).reads, rs, reads.len());
            Span::set(&raw const (*root).strings, ss, strings.len());
        }
        #[cfg(unix)]
        {
            self.block.sync_mapped()?;
            let frozen = self.block.freeze();
            frozen.sync_mapped()?;
        }
        #[cfg(not(unix))]
        {
            use std::io::Write as _;
            let frozen = self.block.freeze();
            let mut f = std::fs::File::create(&self.path)?;
            f.write_all(frozen.bytes(0, frozen.header().length as usize))?;
            f.sync_data()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relation::{Spec, freeze};

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("rerail-kept-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A frozen relation of two columns in a run's own block, indexed on its first: `rows` of them.
    fn made<'b>(block: &'b Block, rows: &[(i64, i64)]) -> &'b Relation {
        let spec = Spec { widths: vec![8, 4], order: vec![0], indexes: vec![vec![1]], min_log: 4 };
        let rel = Relation::create(block, &spec, None);
        let lock = std::sync::Mutex::new(());
        let w = crate::relation::RelW { block, rel, lock: &lock };
        for &(a, b) in rows {
            w.push(&[a, b]);
        }
        freeze(block, rel, &spec, None);
        rel
    }

    fn meta(key: FactKey<'_>, digest: u64) -> Meta<'_> {
        Meta { key, digest, reads: &[], types: &[], dead: &[], levels: &[] }
    }

    fn rows_of(v: &View) -> Vec<Vec<i64>> {
        v.ordered().into_iter().map(|id| v.row(id)).collect()
    }

    const A: [u8; 32] = [1; 32];
    const B: [u8; 32] = [2; 32];

    /// F-R1 at the block: a kept relation reads as the run's did (its rows, canonical order, index probes), from a
    /// mapping of its file, its columns lying in that mapping.
    #[test]
    fn a_kept_relation_is_read_where_it_lies() {
        let d = dir("lies");
        let run = Block::anon(kind::FACTS, 64 << 20);
        let data: Vec<(i64, i64)> = (0..5_000).map(|i| ((i * 7919) % 5_003, i % 13)).collect();
        let rel = made(&run, &data);
        let mut w = KeptW::create(&d.join("b1.blk"), 11, None, 64 << 20).unwrap();
        let reads = [("defs", 5), ("uses", 6)];
        w.put(Meta { reads: &reads, types: &[1, 2], ..meta(FactKey { rel: "calls", ident: &A, part: 0 }, 99) }, rel, None).unwrap();
        w.finish().unwrap();
        let k = KeptFacts::open(&d.join("b1.blk"), 11).unwrap();
        let got = k.get(&FactKey { rel: "calls", ident: &A, part: 0 }).unwrap();
        assert!(k.holds(got.rows), "the rows lie in the kept block's mapping");
        let col0 = got.rows.rows.col(0) as *const _ as usize;
        assert!(col0 >= k.block.base_ptr() as usize && col0 < k.block.base_ptr() as usize + k.bytes());
        assert_eq!((got.digest, got.types), (99, &[1u8, 2][..]));
        assert_eq!(got.reads().collect::<Vec<_>>(), [("defs", 5), ("uses", 6)]);
        let (was, now) = (View::full(rel), got.view());
        assert_eq!(rows_of(&now), rows_of(&was));
        let ix = got.rows.find_index(&[1], got.rows.rows.len()).or(Some(0)).unwrap();
        assert_eq!(now.probe(ix, &[5]), was.probe(ix, &[5]), "an index probe answers as the run's");
        assert!(k.get(&FactKey { rel: "calls", ident: &B, part: 0 }).is_none());
        assert!(KeptFacts::open(&d.join("b1.blk"), 12).is_err(), "a block not the row's is refused");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A delta read over its base: its entries replace the base's, its gone entries hide them, and the rest of the
    /// base reads through; a relation's parts come in key order.
    #[test]
    fn a_delta_reads_over_its_base() {
        let d = dir("delta");
        let run = Block::anon(kind::FACTS, 64 << 20);
        let (r1, r2, r3) = (made(&run, &[(1, 1)]), made(&run, &[(2, 2)]), made(&run, &[(3, 3)]));
        let key = |rel, ident, part| FactKey { rel, ident, part };
        let mut w = KeptW::create(&d.join("b1.blk"), 1, None, 16 << 20).unwrap();
        w.put(meta(key("p", &A, 0), 1), r1, None).unwrap();
        w.put(meta(key("p", &A, 7), 2), r2, None).unwrap();
        w.put(meta(key("q", &B, 0), 3), r3, None).unwrap();
        w.finish().unwrap();
        let base = KeptFacts::open(&d.join("b1.blk"), 1).unwrap();
        let mut w = KeptW::create(&d.join("d2.blk"), 2, Some(1), 16 << 20).unwrap();
        w.put(meta(key("p", &A, 7), 20), r3, None).unwrap();
        w.gone(key("q", &B, 0));
        w.put(meta(key("p", &A, 3), 30), r1, None).unwrap();
        w.finish().unwrap();
        let delta = KeptFacts::open(&d.join("d2.blk"), 2).unwrap();
        assert_eq!(delta.base_tag(), Some(1));
        let v = KeptView { base: Some(&base), delta: Some(&delta) };
        assert_eq!(v.of("p").map(|k| (k.key.part, k.digest)).collect::<Vec<_>>(), [(0, 1), (3, 30), (7, 20)]);
        assert!(v.get(&key("q", &B, 0)).is_none(), "gone in the delta");
        assert_eq!(rows_of(&v.get(&key("p", &A, 7)).unwrap().view()), [vec![3, 3]]);
        assert_eq!(v.all().count(), 3);
        assert_eq!(base.all().count(), 3, "the base alone is as it was");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A key put twice in one block is refused.
    #[test]
    fn a_key_put_twice_is_refused() {
        let d = dir("twice");
        let run = Block::anon(kind::FACTS, 16 << 20);
        let r = made(&run, &[(1, 1)]);
        let mut w = KeptW::create(&d.join("b1.blk"), 1, None, 16 << 20).unwrap();
        w.put(meta(FactKey { rel: "p", ident: &A, part: 0 }, 1), r, None).unwrap();
        w.put(meta(FactKey { rel: "p", ident: &A, part: 0 }, 1), r, None).unwrap();
        assert!(w.finish().is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// F6: an input's granule put from flat rows; a forked process's ids mapped in the copy, the run's left as they
    /// were; dead bits and levels kept beside; a carry with new reads keeps the rows.
    #[test]
    fn the_keep_puts_rows_maps_ids_and_carries() {
        let d = dir("puts");
        let run = Block::anon(kind::FACTS, 16 << 20);
        let r = made(&run, &[(10, 1), (20, 2), (30, 3)]);
        let mut w = KeptW::create(&d.join("b1.blk"), 1, None, 16 << 20).unwrap();
        let f = |v: i64| v + 1000;
        let dead = [0b10u64];
        let levels = [1u32, 2, 3];
        w.put(Meta { dead: &dead, levels: &levels, ..meta(FactKey { rel: "m", ident: &A, part: 0 }, 1) }, r, Some((&[0], &f))).unwrap();
        w.put_rows(meta(FactKey { rel: "g", ident: &B, part: 5 }, 2), &[8, 8], &[1, 2, 3, 4, 5, 6], 3).unwrap();
        w.finish().unwrap();
        assert_eq!(rows_of(&View::full(r)), [vec![10, 1], vec![20, 2], vec![30, 3]], "the run's rows as they were");
        let k = KeptFacts::open(&d.join("b1.blk"), 1).unwrap();
        let m = k.get(&FactKey { rel: "m", ident: &A, part: 0 }).unwrap();
        assert_eq!(rows_of(&m.view()), [vec![1010, 1], vec![1020, 2], vec![1030, 3]]);
        assert_eq!((m.dead, m.levels), (&dead[..], &levels[..]));
        let g = k.get(&FactKey { rel: "g", ident: &B, part: 5 }).unwrap();
        assert_eq!((0..3).map(|i| g.view().row(i)).collect::<Vec<_>>(), [vec![1, 2], vec![3, 4], vec![5, 6]]);
        let mut w = KeptW::create(&d.join("d2.blk"), 2, Some(1), 16 << 20).unwrap();
        w.carry(&m, Some(&[("x", 9)])).unwrap();
        w.finish().unwrap();
        let k2 = KeptFacts::open(&d.join("d2.blk"), 2).unwrap();
        let c = k2.get(&m.key).unwrap();
        assert_eq!(c.reads().collect::<Vec<_>>(), [("x", 9)]);
        assert_eq!((rows_of(&c.view()), c.dead, c.levels), (rows_of(&m.view()), m.dead, m.levels));
        let _ = std::fs::remove_dir_all(&d);
    }
}
