//! One string interner for the graph: every name, key, path and content hash is a `Sym` (u32).
//! Strings live back to back in one byte arena; the table holds only ids. Append-only: a string
//! interned by any worktree keeps its id for every other (ids are shared across base and deltas).
//!
//! The store's single interner (C12, docs/final-shape/03-architecture.md §10.6): the `atoms` table
//! is dense and append-only, and each process holds an `Interner` mirroring a prefix of it. A
//! string the mirror lacks is looked up after the mirror catches up with the table's tail; one
//! that is new is appended under the write lock, so every process allocates the same ids. Fact
//! names, and later `tid`s, fact atoms, paths and type terms, are atoms.
use super::{ContentStore, StoreError, StoreResult};
use rusqlite::{Connection, params};
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

    /// Drops every string from `len` on (strings whose rows did not commit).
    pub(crate) fn truncate(&mut self, len: usize) {
        if len >= self.len() {
            return;
        }
        let end = if len == 0 { 0 } else { self.ends[len - 1] as usize };
        self.buf.truncate(end);
        self.ends.truncate(len);
        self.hashes.truncate(len);
        self.table.retain(|&mut id| (id as usize) < len);
    }

    /// Heap bytes held (capacities).
    pub fn bytes(&self) -> usize {
        self.buf.capacity() + self.ends.capacity() * 4 + self.hashes.capacity() * 8 + self.table.capacity() * 5
    }
}

impl Interner {
    /// Appends the table's atoms from the mirror's length on.
    pub(crate) fn catch_up(&mut self, connection: &Connection) -> StoreResult<()> {
        let mut tail = connection.prepare_cached("SELECT atom,value FROM atoms WHERE atom>=?1 ORDER BY atom")?;
        let rows = tail.query_map([self.len() as i64], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))?;
        for row in rows {
            let (atom, value) = row?;
            if atom != self.len() as i64 || self.get(&value).is_some() {
                return Err(StoreError::Corrupt("atoms are not dense and distinct".to_owned()));
            }
            self.intern(&value);
        }
        Ok(())
    }
}

/// Who may append to a database's `atoms` table now. A planner run allocates the symbols it makes from its own
/// mirror of the table (`logic::native`'s, flushed at its end) and so must be the table's only writer while it runs:
/// it holds the database (`hold`), and the store's interning waits for the release (`ContentStore::intern_atoms`)
/// unless it is on the holder's thread. (A chunk built on one thread while the run on another interned its names
/// between the run's ids, list step 29.)
#[derive(Default)]
struct Hold {
    /// The holding thread and how many holds it has taken (a store's interning inside a held run nests).
    holder: parking_lot::Mutex<Option<(std::thread::ThreadId, usize)>>,
    released: parking_lot::Condvar,
}

static HOLDS: std::sync::LazyLock<parking_lot::Mutex<std::collections::HashMap<std::path::PathBuf, std::sync::Arc<Hold>>>> =
    std::sync::LazyLock::new(Default::default);

fn hold_of(path: &std::path::Path) -> std::sync::Arc<Hold> {
    let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    std::sync::Arc::clone(HOLDS.lock().entry(key).or_default())
}

/// Takes (`true`, waiting for another thread's release) or gives back (`false`) the right to append to the `atoms`
/// table of the database at `path`, for the calling thread. Holds nest on one thread.
pub fn hold(path: &std::path::Path, held: bool) {
    let hold = hold_of(path);
    let mut holder = hold.holder.lock();
    let me = std::thread::current().id();
    if held {
        while holder.is_some_and(|(other, _)| other != me) {
            hold.released.wait(&mut holder);
        }
        let depth = holder.map_or(0, |(_, depth)| depth);
        *holder = Some((me, depth + 1));
    } else if let Some((other, depth)) = *holder
        && other == me
    {
        *holder = (depth > 1).then_some((me, depth - 1));
        if holder.is_none() {
            hold.released.notify_all();
        }
    }
}

impl ContentStore {
    /// The atom of each of `values`, appending the new ones to the table in one transaction.
    pub fn intern_atoms(&self, values: &[&str]) -> StoreResult<Vec<Sym>> {
        if let Some(found) = values.iter().map(|value| self.atoms.lock().get(value)).collect::<Option<Vec<_>>>() {
            return Ok(found);
        }
        // New rows are appended holding the table (`hold`): after a planner run that holds it, and
        // never between its catch-up and its own rows. The hold is taken before the mirror's lock,
        // so a holder interning on its own thread never waits on a thread waiting for its release.
        hold(&self.database_path, true);
        let mut atoms = self.atoms.lock();
        // The mirror keeps only committed rows: each attempt starts from what it held, and a
        // failed one is undone.
        let start = atoms.len();
        let result = self.immediate_transaction(|transaction| {
            atoms.truncate(start);
            atoms.catch_up(transaction)?;
            let mut insert = transaction.prepare_cached("INSERT INTO atoms(atom,value) VALUES (?1,?2)")?;
            values
                .iter()
                .map(|value| match atoms.get(value) {
                    Some(atom) => Ok(atom),
                    None => {
                        let atom = atoms.intern(value);
                        insert.execute(params![atom as i64, value])?;
                        Ok(atom)
                    }
                })
                .collect()
        });
        if result.is_err() {
            atoms.truncate(start);
        }
        drop(atoms);
        hold(&self.database_path, false);
        result
    }

    /// The atom of `value`, if the table holds it.
    pub fn atom(&self, value: &str) -> StoreResult<Option<Sym>> {
        let mut atoms = self.atoms.lock();
        if let Some(atom) = atoms.get(value) {
            return Ok(Some(atom));
        }
        let connection = self.connection.lock().expect("content store mutex poisoned");
        atoms.catch_up(&connection)?;
        Ok(atoms.get(value))
    }

    /// Heap bytes the mirror holds (counted against the fact pool, K1).
    pub fn atom_bytes(&self) -> usize {
        self.atoms.lock().bytes()
    }

    /// The string of `atom`, if the table holds it.
    pub fn atom_value(&self, atom: Sym) -> StoreResult<Option<String>> {
        let mut atoms = self.atoms.lock();
        if (atom as usize) >= atoms.len() {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            atoms.catch_up(&connection)?;
        }
        Ok(((atom as usize) < atoms.len()).then(|| atoms.str(atom).to_owned()))
    }
}

#[cfg(test)]
mod tests;
