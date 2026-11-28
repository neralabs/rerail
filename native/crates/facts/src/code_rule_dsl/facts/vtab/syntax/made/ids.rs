//! A growth's ids (plan "Chunk node numbers"): its documents' numbers and its page table.
//!
//! Every batch of a growth (a chunk's rounds, decision 0431 §6, and the tape values admitted to it, §8) numbers its
//! documents' nodes, tokens and spans in one space of 32-bit ids, cut into pages (`view_reads::IdPages`). A document
//! is numbered when it first enters the growth, by key, and keeps its number in every batch after; it takes its
//! pages when it is first made in the growth (making order, so documents stay made lazily, decision 0494), in its
//! role's region (files, code guests, hidden documents: their keys' ranges): its nodes', tokens' and spans' local ids,
//! then its source's and lines'. Its later spans (its comments no node is, its matches) take local ids after those,
//! as they are made: in the rest of its base run's last page while they fit, then a page more each time one passes
//! the pages it holds. Most documents' ids are one run. A page names the document's number, the local id it starts
//! at, and its key; a directory's row of a document is its number less the directory's first. An id handed out in one
//! round is the same id in the next, whatever directory reads it.

use rerail_blocks::block::Block;
use rerail_clif::view_reads::{IdPages, PAGE, role_of_key};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// A growth's numbering: its page table in the growth's block, and its documents by key.
pub struct Ids {
    block: Arc<Block>,
    /// The page table's offset in the block.
    table: usize,
    numbered: parking_lot::RwLock<HashMap<u32, Arc<Numbered>>>,
}

/// A document of a growth: its number, and its ids once taken.
pub struct Numbered {
    pub number: u32,
    pub key: u32,
    /// Its base run: its first id, its base count (its nodes', tokens' and spans' local ids, `Projected::locs`, then
    /// its source's and lines' spans, from local id 0), and the local ids its pages hold (whole pages). `None` where
    /// its role's region had no room.
    base: OnceLock<Option<Base>>,
    /// The first id of each page its later spans took past its base run's, in order: page `j` holds local ids
    /// `room + j * PAGE ..`.
    more: parking_lot::RwLock<Vec<u32>>,
    /// The growth's block and its page table's offset there, where it takes its pages.
    block: Arc<Block>,
    table: usize,
    /// Its slot in the batch that made it, where its projection is: how an id of it leaves the chunk (`located`).
    slot: parking_lot::RwLock<std::sync::Weak<super::Slot>>,
}

#[derive(Clone, Copy)]
struct Base {
    id: u32,
    n: u32,
    room: u32,
}

impl Numbered {
    /// The id of its local id `local`, where it holds it (its base run's pages, or a page its later spans took).
    pub fn id_of(&self, local: u32) -> Option<u32> {
        let b = (*self.base.get()?)?;
        if local < b.room {
            return Some(b.id + local);
        }
        let past = local - b.room;
        let first = *self.more.read().get((past / PAGE) as usize)?;
        Some(first + past % PAGE)
    }
    /// Takes pages for `ids` ids from local id `local`: their first id.
    fn take(&self, local: u32, ids: u32) -> Option<u32> {
        let table = self.block.ptr::<IdPages>(self.table).cast_const();
        // SAFETY: the growth's page table, in its block, which `self` holds; a role of `ROLES`.
        unsafe { IdPages::take_pages(table, role_of_key(self.key), self.number, self.key, local, ids) }
    }
    /// The id of its later span of local id `local` (past its base count), taking the pages up to it where its pages
    /// do not hold it yet: `None` where its role's region has no room, or before its base run.
    pub fn later(&self, local: u32) -> Option<u32> {
        if let Some(id) = self.id_of(local) {
            return Some(id);
        }
        let b = (*self.base.get()?)?;
        let page = ((local - b.room) / PAGE) as usize;
        let mut more = self.more.write();
        while more.len() <= page {
            let from = b.room + more.len() as u32 * PAGE;
            let first = self.take(from, PAGE)?;
            more.push(first);
        }
        drop(more);
        self.id_of(local)
    }
    /// Records its slot, once it is made (`Docs::base`).
    pub(in super::super) fn made_in(&self, slot: &Arc<super::Slot>) {
        *self.slot.write() = Arc::downgrade(slot);
    }
    /// Its projection, while the batch that made it lives.
    fn projected<R>(&self, f: impl FnOnce(&super::super::Projected) -> R) -> Option<R> {
        let slot = self.slot.read().upgrade()?;
        let made = slot.made.get()?;
        Some(f(&made.p))
    }
    /// Its first id, once taken.
    pub fn first_id(&self) -> Option<u32> {
        self.base.get().copied().flatten().map(|b| b.id)
    }
    /// Its base count: its later spans' local ids follow it.
    pub fn base_count(&self) -> Option<u32> {
        self.base.get().copied().flatten().map(|b| b.n)
    }
    /// The ids it holds: the least and one past the greatest, its base run's and its later pages' hull; `None` before
    /// its making.
    pub fn ids_of(&self) -> Option<(u32, u32)> {
        let b = (*self.base.get()?)?;
        let (mut lo, mut hi) = (b.id, b.id + b.room);
        for &first in self.more.read().iter() {
            lo = lo.min(first);
            hi = hi.max(first + PAGE);
        }
        Some((lo, hi))
    }
}

impl Ids {
    /// A numbering in `block`, its page table reserved there (address space until its pages are taken).
    pub fn new(block: &Arc<Block>) -> Ids {
        let table = block.try_alloc_chunk(IdPages::BYTES, 64).expect("a growth's block holds its page table");
        // SAFETY: the table's header, just allocated in the block (not zeroed: written here, before any page is taken).
        unsafe { block.ptr::<IdPages>(table).write(IdPages::default()) };
        Ids { block: Arc::clone(block), table, numbered: Default::default() }
    }
    /// The page table's offset in the growth's block (`ViewDir::pages`).
    pub fn table_at(&self) -> usize {
        self.table
    }
    fn table(&self) -> *const IdPages {
        self.block.ptr::<IdPages>(self.table).cast_const()
    }
    /// The document of key `key`, numbered now if the growth has not numbered it.
    pub fn numbered(&self, key: u32) -> Arc<Numbered> {
        if let Some(n) = self.numbered.read().get(&key) {
            return Arc::clone(n);
        }
        let mut all = self.numbered.write();
        let number = all.len() as u32;
        Arc::clone(all.entry(key).or_insert_with(|| Arc::new(Numbered { number, key, base: OnceLock::new(), more: Default::default(), block: Arc::clone(&self.block), table: self.table, slot: Default::default() })))
    }
    /// The document of key `key`, if the growth has numbered it.
    pub fn numbered_if(&self, key: u32) -> Option<Arc<Numbered>> {
        self.numbered.read().get(&key).cloned()
    }
    /// How many documents the growth has numbered.
    pub fn count(&self) -> u32 {
        self.numbered.read().len() as u32
    }
    /// `d`'s first id, its base run (`ids` ids) taken on the first ask (its making in the growth).
    pub fn base(&self, d: &Numbered, ids: impl FnOnce() -> u32) -> Option<u32> {
        d.base
            .get_or_init(|| {
                let n = ids();
                let room = n.div_ceil(PAGE).max(1) * PAGE;
                d.take(0, room).map(|id| Base { id, n, room })
            })
            .map(|b| b.id)
    }
    /// The page table's page of id `id`: its document's number, the local id it starts at, and its key.
    pub fn page_of(&self, id: u32) -> rerail_clif::view_reads::Page {
        // SAFETY: the growth's page table; `id` read only where it was handed out (`base`, `later`).
        unsafe { IdPages::page_of(self.table(), id) }
    }
    /// The key of id `id`'s document.
    pub fn key_of(&self, id: u32) -> u32 {
        // SAFETY: as `page_of`.
        unsafe { IdPages::key_of_id(self.table(), id) }
    }
    /// The key of the document of value `value`, where it is an id the growth handed out (its page taken); `None` for
    /// any other value.
    pub fn key_of_value(&self, value: i64) -> Option<u32> {
        let id = u32::try_from(value).ok()?;
        // SAFETY: the growth's page table, read only once the page is taken.
        unsafe { IdPages::taken(self.table(), id).then(|| IdPages::key_of_id(self.table(), id)) }
    }
    /// The local id of value `value` in its document, where it is an id the growth handed out.
    pub fn local_of_value(&self, value: i64) -> Option<u32> {
        let id = u32::try_from(value).ok()?;
        // SAFETY: as `key_of_value`.
        unsafe { IdPages::taken(self.table(), id).then(|| { let p = IdPages::page_of(self.table(), id); p.local + (id & (rerail_clif::view_reads::PAGE - 1)) }) }
    }
}

/// The growth's numbering as the executor and the corpus phase ask it (`numbering::Numbering`).
impl rerail_facts_base::code_rule_dsl::facts::numbering::Numbering for Ids {
    fn key_of(&self, id: i64) -> i64 {
        if id < 0 {
            return rerail_blocks::located::Located::key_wide(id);
        }
        self.key_of_value(id).map_or(-1, i64::from)
    }
    fn local_of(&self, id: i64) -> u32 {
        self.local_of_value(id).unwrap_or(u32::MAX)
    }
    fn ids_of(&self, key: u32) -> Option<(u32, u32)> {
        self.numbered_if(key)?.ids_of()
    }
    fn located(&self, id: i64) -> i64 {
        if id < 0 {
            return id;
        }
        let (Some(key), Some(local)) = (self.key_of_value(id), self.local_of_value(id)) else { return id };
        self.numbered_if(key).and_then(|n| n.projected(|p| p.located(local))).unwrap_or(id)
    }
    fn of_located(&self, value: i64) -> Option<i64> {
        if value < 0 {
            return Some(value);
        }
        let at = rerail_blocks::located::Located(value);
        self.numbered_if(at.key())?.projected(|p| p.local_of(at.low()).filter(|_| p.is_numbered()).map(|l| p.global(l)))?
    }
}
