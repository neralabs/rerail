//! The store's RAM working set (decision 0024 §5): what a push just wrote, held so the wave that
//! follows reads it without waiting on SQLite. Reads are the overlay ∪ SQLite: an entry is only
//! ever a copy of what SQLite holds (written through), keyed by identity, so a miss reads SQLite
//! and a hit equals it.
//!
//! - **Tapes** by their canonical digest (the tape name's binding): the decoded tape, as the
//!   engine holds it (type-erased here: the store does not know the engine's types).
//! - **Kind images** by tape: the node count and each kind's postings, tape-local facts of the
//!   tape (`TapePrefilter::kinds`). Terms and uncovered nodes are not here: the store-wide residue
//!   cap decides them in SQLite.
//!
//! Entries are evicted oldest first past `cap_bytes`, so the set stays a bounded part of the
//! process's budget.
use std::any::Any;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

/// The working set's bytes (estimated): 96 MB of the default 1 GB budget, scaled with it (decision 0126).
fn cap_bytes() -> usize {
    rerail_accel::budget::share(96 << 20)
}

/// A tape's kind image: its language, node count and per kind atom its postings, by atom.
#[derive(Debug)]
pub struct KindImage {
    pub lang: u32,
    pub nodes: u32,
    pub kinds: Vec<(u32, Vec<u8>)>,
}

impl KindImage {
    pub fn postings(&self, atom: u32) -> Option<&[u8]> {
        self.kinds.binary_search_by_key(&atom, |(kind, _)| *kind).ok().map(|at| self.kinds[at].1.as_slice())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Key {
    Tape([u8; 32]),
    Kinds(i64),
}

enum Entry {
    Tape(Arc<dyn Any + Send + Sync>),
    Kinds(Arc<KindImage>),
}

#[derive(Default)]
struct Inner {
    entries: HashMap<Key, (Entry, usize)>,
    order: VecDeque<Key>,
    bytes: usize,
}

#[derive(Default)]
pub struct WorkingSet(parking_lot::Mutex<Inner>);

impl WorkingSet {
    fn insert(&self, key: Key, entry: Entry, bytes: usize) {
        let mut inner = self.0.lock();
        if let Some((_, old)) = inner.entries.remove(&key) {
            inner.bytes -= old;
        }
        inner.entries.insert(key, (entry, bytes));
        inner.order.push_back(key);
        inner.bytes += bytes;
        while inner.bytes > cap_bytes() {
            let Some(oldest) = inner.order.pop_front() else { break };
            if let Some((_, bytes)) = inner.entries.remove(&oldest) {
                inner.bytes -= bytes;
            }
        }
        // Keys evicted or replaced leave stale order entries: compacted when they dominate.
        if inner.order.len() > 2 * inner.entries.len() + 64 {
            let Inner { entries, order, .. } = &mut *inner;
            order.retain(|key| entries.contains_key(key));
        }
    }

    /// Holds the decoded tape of digest `digest` (`bytes`: its estimated size).
    pub fn put_tape(&self, digest: [u8; 32], tape: Arc<dyn Any + Send + Sync>, bytes: usize) {
        self.insert(Key::Tape(digest), Entry::Tape(tape), bytes);
    }

    pub fn tape(&self, digest: [u8; 32]) -> Option<Arc<dyn Any + Send + Sync>> {
        match self.0.lock().entries.get(&Key::Tape(digest)) {
            Some((Entry::Tape(tape), _)) => Some(Arc::clone(tape)),
            _ => None,
        }
    }

    pub fn put_kinds(&self, tape: i64, image: KindImage) {
        let bytes = image.kinds.iter().map(|(_, postings)| postings.len() + 32).sum::<usize>() + 64;
        self.insert(Key::Kinds(tape), Entry::Kinds(Arc::new(image)), bytes);
    }

    pub fn kinds(&self, tape: i64) -> Option<Arc<KindImage>> {
        match self.0.lock().entries.get(&Key::Kinds(tape)) {
            Some((Entry::Kinds(image), _)) => Some(Arc::clone(image)),
            _ => None,
        }
    }

    /// Drops everything: tapes left the store in bulk (reclamation), whose ids may be reused.
    pub fn clear(&self) {
        *self.0.lock() = Inner::default();
    }

    pub fn forget_kinds(&self, tape: i64) {
        let mut inner = self.0.lock();
        if let Some((_, bytes)) = inner.entries.remove(&Key::Kinds(tape)) {
            inner.bytes -= bytes;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_working_set_holds_what_was_written_and_evicts_oldest_past_its_cap() {
        let set = WorkingSet::default();
        set.put_kinds(1, KindImage { lang: 3, nodes: 10, kinds: vec![(5, vec![1, 2]), (9, vec![0])] });
        let image = set.kinds(1).unwrap();
        assert_eq!((image.lang, image.nodes, image.postings(9)), (3, 10, Some(&[0_u8][..])));
        assert!(image.postings(6).is_none());
        set.put_tape([7; 32], Arc::new(42_u32), cap_bytes());
        assert!(set.kinds(1).is_none(), "the oldest entry goes past the cap");
        assert_eq!(*set.tape([7; 32]).unwrap().downcast::<u32>().unwrap(), 42);
        set.forget_kinds(1);
        set.put_kinds(2, KindImage { lang: 3, nodes: 1, kinds: Vec::new() });
        assert!(set.tape([7; 32]).is_none() && set.kinds(2).is_some());
    }
}
