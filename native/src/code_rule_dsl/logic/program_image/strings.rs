//! A string table (decision 0173 §4): the bytes, each string's end offset, and a linear hash table of 64-byte
//! buckets, text → id. The table grows one bucket at a time: an insert splits at most one bucket, so no insert
//! rebuilds it. The same three regions are read in place from an image (`find_in`).
//!
//! A bucket is eight words: seven (tag << 32 | id) slots, then (next overflow bucket + 1) << 32 | count.

pub const SLOTS: usize = 7;
/// Mean entries a bucket before the table grows by one.
const LOAD: u64 = 5;
const START_LEVEL: u32 = 3;

pub fn hash(bytes: &[u8]) -> u64 {
    use std::hash::BuildHasher;
    foldhash::quality::FixedState::with_seed(0x0173_5172).hash_one(bytes)
}

pub struct Table {
    pub buckets: Vec<[u64; 8]>,
    pub over: Vec<[u64; 8]>,
    pub level: u32,
    pub split: u32,
    pub count: u64,
}

impl Default for Table {
    fn default() -> Self {
        Table { buckets: vec![[0; 8]; 1 << START_LEVEL], over: Vec::new(), level: START_LEVEL, split: 0, count: 0 }
    }
}

#[inline]
fn address(h: u64, level: u32, split: u32) -> usize {
    let b = h & ((1u64 << level) - 1);
    (if b < split as u64 { h & ((1u64 << (level + 1)) - 1) } else { b }) as usize
}

fn count(b: &[u64; 8]) -> usize {
    (b[7] & 0xff) as usize
}
fn next(b: &[u64; 8]) -> u32 {
    (b[7] >> 32) as u32
}

/// The id of the entry whose hash is `h` and for which `eq` holds, in the table of these regions.
pub fn find_in(buckets: &[[u64; 8]], over: &[[u64; 8]], level: u32, split: u32, h: u64, eq: impl Fn(u32) -> bool) -> Option<u32> {
    if buckets.is_empty() {
        return None;
    }
    let tag = (h >> 32) as u32;
    let mut b = buckets.get(address(h, level, split))?;
    loop {
        for &slot in &b[..count(b).min(SLOTS)] {
            if (slot >> 32) as u32 == tag && eq(slot as u32) {
                return Some(slot as u32);
            }
        }
        match next(b) {
            0 => return None,
            n => b = over.get(n as usize - 1)?,
        }
    }
}

impl Table {
    fn put(&mut self, at: usize, tag: u32, id: u32) {
        // `None`: the main bucket `at`; `Some(i)`: overflow bucket i.
        let mut cur: Option<usize> = None;
        loop {
            let b = match cur {
                None => &mut self.buckets[at],
                Some(i) => &mut self.over[i],
            };
            let c = count(b);
            if c < SLOTS {
                b[c] = (tag as u64) << 32 | id as u64;
                b[7] = (b[7] & !0xff) | (c as u64 + 1);
                return;
            }
            match next(b) {
                0 => {
                    let n = self.over.len();
                    let b = match cur {
                        None => &mut self.buckets[at],
                        Some(i) => &mut self.over[i],
                    };
                    b[7] = (b[7] & 0xffff_ffff) | ((n as u64 + 1) << 32);
                    self.over.push([0; 8]);
                    cur = Some(n);
                }
                n => cur = Some(n as usize - 1),
            }
        }
    }
    /// Adds `id` under hash `h`, then grows by one bucket if the load passed its bound.
    pub fn insert(&mut self, h: u64, id: u32, hash_of: impl Fn(u32) -> u64) {
        let at = address(h, self.level, self.split);
        self.put(at, (h >> 32) as u32, id);
        self.count += 1;
        if self.count > LOAD * self.buckets.len() as u64 {
            self.split_one(hash_of);
        }
    }
    /// Splits the bucket at the split point into it and one new bucket at the end. Its overflow buckets are left
    /// unused (a rebuild drops them).
    fn split_one(&mut self, hash_of: impl Fn(u32) -> u64) {
        let s = self.split as usize;
        let mut entries = Vec::new();
        let mut b = self.buckets[s];
        loop {
            entries.extend_from_slice(&b[..count(&b)]);
            match next(&b) {
                0 => break,
                n => b = self.over[n as usize - 1],
            }
        }
        self.buckets[s] = [0; 8];
        self.buckets.push([0; 8]);
        self.split += 1;
        if self.split == 1 << self.level {
            self.level += 1;
            self.split = 0;
        }
        for slot in entries {
            let id = slot as u32;
            let at = address(hash_of(id), self.level, self.split);
            self.put(at, (slot >> 32) as u32, id);
        }
    }
}

/// Texts, each once, with their ids: an image's string table as it is built.
#[derive(Default)]
pub struct Strings {
    bytes: Vec<u8>,
    ends: Vec<u32>,
    hashes: Vec<u64>,
    table: Table,
}

impl Strings {
    pub fn len(&self) -> usize {
        self.ends.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }
    fn get(&self, id: u32) -> &[u8] {
        let start = if id == 0 { 0 } else { self.ends[id as usize - 1] as usize };
        &self.bytes[start..self.ends[id as usize] as usize]
    }
    pub fn find(&self, s: &str) -> Option<u32> {
        let t = &self.table;
        find_in(&t.buckets, &t.over, t.level, t.split, hash(s.as_bytes()), |id| self.get(id) == s.as_bytes())
    }
    /// The id of `s`, given one if it has none.
    pub fn intern(&mut self, s: &str) -> u32 {
        let h = hash(s.as_bytes());
        let t = &self.table;
        if let Some(id) = find_in(&t.buckets, &t.over, t.level, t.split, h, |id| self.get(id) == s.as_bytes()) {
            return id;
        }
        let id = self.ends.len() as u32;
        self.bytes.extend_from_slice(s.as_bytes());
        self.ends.push(self.bytes.len() as u32);
        self.hashes.push(h);
        let hashes = &self.hashes;
        self.table.insert(h, id, |i| hashes[i as usize]);
        id
    }
    pub fn table(&self) -> &Table {
        &self.table
    }
    pub fn into_parts(self) -> (Vec<u8>, Vec<u32>, Table) {
        (self.bytes, self.ends, self.table)
    }
}
