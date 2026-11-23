//! One string interner for the graph: every name, key, path and content hash is a `Sym` (u32).
//! Strings live back to back in one byte arena; the table holds only ids. Append-only: a string
//! interned by any worktree keeps its id for every other (ids are shared across base and deltas).
use std::hash::BuildHasher;

pub type Sym = u32;

/// The fixed hash every string key uses: the interner's table, and the dependency keys of
/// probes (`Engine::probe`), which must agree for strings that are not (yet) interned.
pub fn hash_str(s: &str) -> u64 {
    foldhash::quality::FixedState::with_seed(0x5EED_71).hash_one(s)
}

pub struct Interner {
    buf: Vec<u8>,
    /// ends[i] = end offset of string i in `buf` (start = ends[i - 1]).
    ends: Vec<u32>,
    hashes: Vec<u64>,
    table: hashbrown::HashTable<Sym>,
}

impl Default for Interner {
    fn default() -> Self {
        let mut i = Interner { buf: Vec::new(), ends: Vec::new(), hashes: Vec::new(), table: hashbrown::HashTable::new() };
        // Sym 0 is the empty string.
        i.intern("");
        i
    }
}

impl Interner {
    pub fn str(&self, s: Sym) -> &str {
        let s = s as usize;
        let start = if s == 0 { 0 } else { self.ends[s - 1] as usize };
        // SAFETY: only whole `&str`s are appended to `buf`.
        unsafe { std::str::from_utf8_unchecked(&self.buf[start..self.ends[s] as usize]) }
    }

    pub fn hash_of(&self, s: Sym) -> u64 {
        self.hashes[s as usize]
    }

    pub fn get(&self, s: &str) -> Option<Sym> {
        self.get_h(s, hash_str(s))
    }

    pub fn get_h(&self, s: &str, h: u64) -> Option<Sym> {
        self.table.find(h, |&id| self.hashes[id as usize] == h && self.str(id) == s).copied()
    }

    pub fn intern(&mut self, s: &str) -> Sym {
        let h = hash_str(s);
        if let Some(id) = self.get_h(s, h) {
            return id;
        }
        let id = self.ends.len() as Sym;
        self.buf.extend_from_slice(s.as_bytes());
        assert!(self.buf.len() <= u32::MAX as usize, "interner arena over 4 GiB");
        self.ends.push(self.buf.len() as u32);
        self.hashes.push(h);
        let hashes = &self.hashes;
        self.table.insert_unique(h, id, |&x| hashes[x as usize]);
        id
    }

    pub fn len(&self) -> usize {
        self.ends.len()
    }

    /// Heap bytes held (capacities).
    pub fn bytes(&self) -> usize {
        self.buf.capacity() + self.ends.capacity() * 4 + self.hashes.capacity() * 8 + self.table.capacity() * 5
    }
}
