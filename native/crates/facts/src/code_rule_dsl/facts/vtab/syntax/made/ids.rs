//! A growth's ids (plan "Chunk node numbers", step 1): its documents' numbers and its page table.
//!
//! Every batch of a growth (a chunk's rounds, decision 0431 §6, and the tape values admitted to it, §8) numbers its
//! documents' nodes, tokens and spans in one space of 32-bit ids, cut into pages (`view_reads::IdPages`). A document
//! is numbered when it first enters the growth, by key, and keeps its number in every batch after; it takes its
//! pages when it is first made in the growth (making order, so documents stay made lazily, decision 0494), in its
//! role's region (files, code guests, hidden documents: their keys' ranges), and its later spans (its comments and
//! matches, made with its rows) one run of pages more. A page names the document's number and the local id it starts
//! at; each directory of the growth maps a number to its own row (`ViewDir::by_number`). An id handed out in one
//! round is the same id in the next, whatever directory reads it.

use rerail_blocks::block::Block;
use rerail_clif::view_reads::{IdPages, role_of_key};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
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
    /// Its first id: its nodes', tokens' and spans' (`Projected::locs`), then its source's and lines' spans, from
    /// local id 0. `None` where its role's region had no room.
    base: OnceLock<Option<u32>>,
    /// The local ids it holds so far (`base`'s, then `later`'s).
    next: AtomicU32,
    /// Its later spans' first id and first local id (its comments', then its matches'), taken once with its rows.
    later: OnceLock<Option<(u32, u32)>>,
}

impl Ids {
    /// A numbering in `block`, its page table reserved there (address space until its pages are taken).
    pub fn new(block: &Arc<Block>) -> Ids {
        let table = block.try_alloc_chunk_any(IdPages::BYTES, 64).expect("a growth's block holds its page table");
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
        Arc::clone(all.entry(key).or_insert_with(|| Arc::new(Numbered { number, key, base: OnceLock::new(), next: AtomicU32::new(0), later: OnceLock::new() })))
    }
    /// How many documents the growth has numbered.
    pub fn count(&self) -> u32 {
        self.numbered.read().len() as u32
    }
    /// Takes `ids` ids of `d` from its next local id: their first id and local id.
    fn take(&self, d: &Numbered, ids: u32) -> Option<(u32, u32)> {
        let local = d.next.fetch_add(ids, Ordering::AcqRel);
        // SAFETY: the growth's page table, in its block, which `self` holds; a role of `ROLES`.
        let id = unsafe { IdPages::take_pages(self.table(), role_of_key(d.key), d.number, local, ids) }?;
        Some((id, local))
    }
    /// `d`'s first id, its first `ids` ids taken on the first ask (its making in the growth).
    pub fn base(&self, d: &Numbered, ids: impl FnOnce() -> u32) -> Option<u32> {
        *d.base.get_or_init(|| self.take(d, ids()).map(|(id, _)| id))
    }
    /// `d`'s later spans' first id and local id, their `ids` ids taken on the first ask (its rows' making).
    pub fn later(&self, d: &Numbered, ids: u32) -> Option<(u32, u32)> {
        *d.later.get_or_init(|| self.take(d, ids))
    }
    /// The page table's page of id `id`: its document's number and the local id it starts at.
    pub fn page_of(&self, id: u32) -> rerail_clif::view_reads::Page {
        // SAFETY: the growth's page table; `id` read only where it was handed out (`base`, `later`).
        unsafe { IdPages::page_of(self.table(), id) }
    }
}
