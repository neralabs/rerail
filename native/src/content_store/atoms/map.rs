//! A fork's map between this process's atoms and the store's (decision 0170).
//!
//! Ids are given out without the lease (decision 0160), from this process's end. Another process may store atoms past
//! that end while some of this one's are pending: the two numbered different strings alike. Then this process forks.
//! Below the fork both agree. Past it, each keeps its own ids in memory (they index dense arrays, and every row it
//! holds names them), and the store keeps its own. Between them is this map:
//!
//! - **Strings first.** In a write transaction (SQLite's write lock held), the pending atoms are found in the store by
//!   their key; a string it has takes its id, and the rest are appended as new segments. The map is local id to
//!   store id. A writer's batch does this before its jobs make their rows (`begin_write`), so they are made with the
//!   store's ids, not made and then made again.
//! - **Rows written** through a typed boundary (a frame's symbol columns, the memo's) are mapped local to store
//!   (`to_store`); **rows read** store to local (`to_local`). Another process's atom is read and interned here when a
//!   row read names it (`hold`): only the ids met, not every atom the others stored (968,000 of them, read inside the
//!   write lock, held the other processes' writers past their wait).
//! - **Nothing else** holding a `Sym` is written or read after the fork: a write job that does not declare its rows
//!   mapped is dropped, as a cache's miss (`write_back::Syms`).
//!
//! A segment appended here is the map's only while it is in the store: a rolled-back savepoint or transaction takes
//! it away, and the next write sees that (`validate`) and forgets what it mapped.
use super::{Atoms, Interner, SEGMENT_BYTES, State, Sym, database_end, hash_str, key_end, key_segments};
use crate::content_store::{StoreError, StoreResult};
use rusqlite::{Connection, OptionalExtension};
use std::collections::HashMap;
use std::sync::atomic::Ordering;

const NONE: Sym = Sym::MAX;

pub(super) struct Map {
    /// Ids below are the same here and in the store.
    pub(super) fork: Sym,
    /// The store's id of local atom `fork + i`, or `NONE` (pending).
    to_store: Vec<Sym>,
    /// The local id of each store id past `fork` met so far.
    to_local: HashMap<Sym, Sym>,
    /// How many of `to_store` are set.
    mapped: usize,
    /// Segments appended here and not yet seen committed: their first id and the local atoms they hold.
    open: Vec<(Sym, Vec<Sym>)>,
}

impl Map {
    fn new(fork: Sym) -> Map {
        Map { fork, to_store: Vec::new(), to_local: HashMap::new(), mapped: 0, open: Vec::new() }
    }

    fn store_of(&self, local: Sym) -> Option<Sym> {
        if local < self.fork {
            return Some(local);
        }
        self.to_store.get((local - self.fork) as usize).copied().filter(|&s| s != NONE)
    }

    pub(super) fn local_of(&self, store: Sym) -> Option<Sym> {
        if store < self.fork { Some(store) } else { self.to_local.get(&store).copied() }
    }

    fn set(&mut self, local: Sym, store: Sym) {
        let i = (local - self.fork) as usize;
        if self.to_store.len() <= i {
            self.to_store.resize(i + 1, NONE);
        }
        if self.to_store[i] == NONE {
            self.mapped += 1;
        }
        self.to_store[i] = store;
        self.to_local.insert(store, local);
    }

    fn unset(&mut self, local: Sym) {
        let i = (local - self.fork) as usize;
        if let Some(s) = self.to_store.get_mut(i).filter(|s| **s != NONE) {
            self.to_local.remove(s);
            *s = NONE;
            self.mapped -= 1;
        }
    }

    /// Local atoms past the fork with no store id yet, or one in a segment not yet seen committed (its transaction may
    /// still roll back).
    pub(super) fn pending(&self, len: usize) -> usize {
        len - self.fork as usize - self.mapped + self.open.iter().map(|(_, l)| l.len()).sum::<usize>()
    }
}

/// The store's segments a lookup reads, each read once (its ends and bytes) and sliced per string: decoding a whole
/// segment (64 MB, 450,000 atoms) for each string looked up held the write lock for seconds while every other process's
/// writer waited.
struct Segments<'c> {
    connection: &'c Connection,
    held: std::collections::BTreeMap<Sym, (Vec<u8>, Vec<u8>)>,
}

impl<'c> Segments<'c> {
    fn new(connection: &'c Connection) -> Self {
        Segments { connection, held: Default::default() }
    }

    /// The store's string of `id`.
    fn string(&mut self, id: Sym) -> StoreResult<Option<String>> {
        let within = |held: &std::collections::BTreeMap<Sym, (Vec<u8>, Vec<u8>)>| held.range(..=id).next_back().filter(|(first, (ends, _))| ((id - **first) as usize) < ends.len() / 4).map(|(f, _)| *f);
        let first = match within(&self.held) {
            Some(first) => first,
            None => {
                let row: Option<(i64, Vec<u8>, Vec<u8>)> = self
                    .connection
                    .prepare_cached("SELECT first, ends, bytes FROM atom_segment WHERE first <= ?1 ORDER BY first DESC LIMIT 1")?
                    .query_row([i64::from(id)], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                    .optional()?;
                let Some((first, ends, bytes)) = row else { return Ok(None) };
                self.held.insert(first as Sym, (ends, bytes));
                match within(&self.held) {
                    Some(first) => first,
                    None => return Ok(None),
                }
            }
        };
        let (ends, bytes) = &self.held[&first];
        let i = (id - first) as usize;
        let at = |k: usize| u32::from_le_bytes(ends[k * 4..k * 4 + 4].try_into().expect("4 bytes")) as usize;
        let (start, end) = (if i == 0 { 0 } else { at(i - 1) }, at(i));
        let s = bytes.get(start..end).and_then(|s| std::str::from_utf8(s).ok()).ok_or_else(|| StoreError::Corrupt(format!("atom segment {first}: a string is not UTF-8 or out of its bytes")))?;
        Ok(Some(s.to_owned()))
    }
}

/// Holds store atoms `ids` (past the fork, not met yet): each string read from its segment (one read a segment) and
/// interned here; a string this process holds already maps to its atom.
fn hold(connection: &Connection, atoms: &mut Interner, map: &mut Map, ids: &[Sym]) -> StoreResult<()> {
    let mut ids: Vec<Sym> = ids.iter().copied().filter(|&id| map.local_of(id).is_none()).collect();
    ids.sort_unstable();
    ids.dedup();
    let mut segments = Segments::new(connection);
    for id in ids {
        let Some(s) = segments.string(id)? else { continue };
        let local = atoms.intern(&s);
        if local >= map.fork && map.store_of(local).is_none() {
            map.set(local, id);
        } else {
            // Held under another id already (below the fork, or mapped): reads of `id` still find it.
            map.to_local.insert(id, local);
        }
    }
    Ok(())
}

/// The store's atom of `value` (past the fork), held here: found by its key.
fn find(connection: &Connection, atoms: &mut Interner, map: &mut Map, value: &str) -> StoreResult<()> {
    let ids: Vec<i64> = connection
        .prepare_cached("SELECT id FROM atom_key WHERE h = ?1")?
        .query_map([hash_str(value) as i64], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let ids: Vec<Sym> = ids.into_iter().map(|id| id as Sym).filter(|&id| id >= map.fork).collect();
    hold(connection, atoms, map, &ids)
}

/// Forgets what segments rolled back took away (a failed savepoint or transaction): their atoms are pending again.
fn validate(connection: &Connection, map: &mut Map) -> StoreResult<()> {
    let mut kept = Vec::new();
    for (first, locals) in std::mem::take(&mut map.open) {
        let there: bool = connection
            .prepare_cached("SELECT EXISTS (SELECT 1 FROM atom_segment WHERE first = ?1)")?
            .query_row([i64::from(first)], |r| r.get(0))?;
        if there {
            kept.push((first, locals));
        } else {
            for l in locals {
                map.unset(l);
            }
        }
    }
    map.open = kept;
    Ok(())
}

/// Gives `locals` (those not mapped yet) store ids, in `connection`'s write transaction: a string the store holds
/// takes its id; the rest are appended. Returns how many were appended.
fn resolve(connection: &Connection, atoms: &mut Interner, map: &mut Map, locals: &[Sym]) -> StoreResult<usize> {
    let mut wanted: Vec<Sym> = locals.iter().copied().filter(|&l| map.store_of(l).is_none()).collect();
    wanted.sort_unstable();
    wanted.dedup();
    if wanted.is_empty() {
        return Ok(0);
    }
    if let Some(&bad) = wanted.iter().find(|&&l| l as usize >= atoms.len()) {
        return Err(StoreError::Invalid(format!("symbol {bad} is no atom of this process's (it holds {}): not mapped to the store's", atoms.len())));
    }
    let end = database_end(connection)?;
    if key_end(connection).is_none_or(|upto| upto < end) {
        key_segments(connection)?;
    }
    // By key: another process may have stored the string (past the fork, or below it, where this process did not
    // hold it when it interned it).
    let hashes: Vec<rusqlite::types::Value> = wanted.iter().map(|&l| rusqlite::types::Value::Integer(hash_str(atoms.str(l).expect("a local atom")) as i64)).collect();
    let found: Vec<(i64, i64)> = connection
        .prepare_cached("SELECT h, id FROM atom_key WHERE h IN rarray(?1)")?
        .query_map([std::rc::Rc::new(hashes)], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut by_hash: HashMap<u64, Vec<Sym>> = HashMap::new();
    for (h, id) in found {
        by_hash.entry(h as u64).or_default().push(id as Sym);
    }
    let mut new = Vec::new();
    let mut segments = Segments::new(connection);
    for l in wanted {
        let s = atoms.str(l).expect("a local atom").to_owned();
        let mut hit = None;
        for &id in by_hash.get(&hash_str(&s)).into_iter().flatten() {
            let theirs = match map.local_of(id).and_then(|x| atoms.str(x).map(str::to_owned)) {
                Some(t) => Some(t),
                None => segments.string(id)?,
            };
            if theirs.as_deref() == Some(s.as_str()) {
                hit = Some(id);
                break;
            }
        }
        match hit {
            Some(id) => {
                // Not `set`: a store id below the fork reads as itself, never as `l`.
                let i = (l - map.fork) as usize;
                if map.to_store.len() <= i {
                    map.to_store.resize(i + 1, NONE);
                }
                if map.to_store[i] == NONE {
                    map.mapped += 1;
                }
                map.to_store[i] = id;
                if id >= map.fork {
                    map.to_local.insert(id, l);
                }
            }
            None => new.push(l),
        }
    }
    if new.is_empty() {
        return Ok(0);
    }
    // Appended at the store's end, in segments of at most `SEGMENT_BYTES`, keyed in the same transaction.
    let mut first = database_end(connection)? as Sym;
    let keyed = key_end(connection) == Some(first as usize);
    let mut i = 0;
    while i < new.len() {
        let (mut ends, mut bytes) = (Vec::new(), Vec::new());
        let mut j = i;
        while j < new.len() && (j == i || bytes.len() + atoms.str(new[j]).expect("a local atom").len() <= SEGMENT_BYTES) {
            bytes.extend_from_slice(atoms.str(new[j]).expect("a local atom").as_bytes());
            ends.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            j += 1;
        }
        connection.prepare_cached("INSERT INTO atom_segment(first, ends, bytes) VALUES (?1, ?2, ?3)")?.execute(rusqlite::params![i64::from(first), ends, bytes])?;
        let mut keys: Vec<(i64, i64)> = new[i..j].iter().enumerate().map(|(k, &l)| (hash_str(atoms.str(l).expect("a local atom")) as i64, i64::from(first) + k as i64)).collect();
        keys.sort_unstable();
        if keyed {
            let mut insert = connection.prepare_cached("INSERT OR IGNORE INTO atom_key(h, id) VALUES (?1, ?2)")?;
            for (h, id) in keys {
                insert.execute([h, id])?;
            }
        }
        for (k, &l) in new[i..j].iter().enumerate() {
            map.set(l, first + k as Sym);
        }
        map.open.push((first, new[i..j].to_vec()));
        first += (j - i) as Sym;
        i = j;
    }
    if keyed {
        connection.prepare_cached("UPDATE atom_key_end SET upto = ?1")?.execute([i64::from(first)])?;
    }
    Ok(new.len())
}

/// Whether `connection` holds a write transaction on its main database.
fn writing(connection: &Connection) -> bool {
    // SAFETY: a read of the connection's transaction state on its own thread.
    unsafe { rusqlite::ffi::sqlite3_txn_state(connection.handle(), c"main".as_ptr()) == rusqlite::ffi::SQLITE_TXN_WRITE }
}

thread_local! {
    /// This thread's write declares its rows' symbols mapped, or holds none (`write_back::Syms`): its transaction may
    /// commit after a fork.
    static MAPPED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `f` as a write whose symbols are mapped (or that holds none): its transaction may commit after a fork.
pub fn mapped<R>(f: impl FnOnce() -> R) -> R {
    let was = MAPPED.with(|m| m.replace(true));
    let out = f();
    MAPPED.with(|m| m.set(was));
    out
}

pub(super) fn mapped_now() -> bool {
    MAPPED.with(std::cell::Cell::get)
}

impl Atoms {
    /// Whether this process's atoms forked from the store's (decision 0170).
    pub fn forked(&self) -> bool {
        self.forked.load(Ordering::Acquire)
    }

    /// Forks at this process's end (`base`): another process stored atoms past it while some here were pending.
    pub(super) fn fork_now(&self, connection: &Connection, state: &mut State) -> StoreResult<()> {
        if state.map.is_some() {
            return Ok(());
        }
        let base = state.atoms.base() as Sym;
        let end = database_end(connection)?;
        state.map = Some(Map::new(base));
        self.forked.store(true, Ordering::Release);
        eprintln!(
            "rerail: atoms: another process stored atoms {base}..{end} while this one held {} (decision 0170): mapping this process's ids to the store's from here",
            state.atoms.pending().0
        );
        let _ = connection;
        Ok(())
    }

    /// At the start of a write transaction that holds SQLite's write lock (`BEGIN IMMEDIATE`), before its rows are
    /// made: forks if another process stored past this one's pending atoms, and, forked, gives every pending atom its
    /// store id, so the rows are made with them.
    pub fn begin_write(&self, connection: &Connection) -> StoreResult<()> {
        if self.path.is_none() || !self.owns(connection) {
            return Ok(());
        }
        let mut state = self.state.write();
        if state.map.is_none() {
            let (base, len) = (state.atoms.base(), state.atoms.len());
            let end = database_end(connection)?;
            if !(end > base && len > base && end != self.wrote.load(Ordering::Acquire)) {
                return Ok(());
            }
            self.fork_now(connection, &mut state)?;
        }
        self.map_pending(connection, &mut state)
    }

    /// Forked, in a write transaction: what rolled back is forgotten, other processes' atoms are held, and every
    /// pending atom gets its store id.
    fn map_pending(&self, connection: &Connection, state: &mut State) -> StoreResult<()> {
        let State { atoms, map, .. } = state;
        let Some(map) = map.as_mut() else { return Ok(()) };
        validate(connection, map)?;
        let pending: Vec<Sym> = (map.fork..atoms.len() as Sym).filter(|&l| map.store_of(l).is_none()).collect();
        let n = resolve(connection, atoms, map, &pending)?;
        if n > 0 {
            let mut written = self.written.lock();
            *written = (written.0 + 1, written.1 + n as u64, written.2);
        }
        Ok(())
    }

    /// Forked: `write_pending` in the transaction about to commit. A write that does not declare its symbols mapped
    /// is refused: its rows may name this process's ids.
    pub(super) fn write_forked(&self, connection: &Connection) -> StoreResult<usize> {
        if !mapped_now() {
            return Err(StoreError::Invalid(super::DIVERGED_TEXT.to_owned()));
        }
        let mut state = self.state.write();
        self.map_pending(connection, &mut state)?;
        // What a savepoint rolled back since is forgotten before the commit.
        if let Some(map) = state.map.as_mut() {
            validate(connection, map)?;
        }
        Ok(0)
    }

    /// Forked: after a commit, every segment appended in it is the store's.
    pub(super) fn committed_forked(&self, state: &mut State) {
        let len = state.atoms.len();
        if let Some(map) = state.map.as_mut() {
            map.open.clear();
            if map.pending(len) == 0 {
                state.since = None;
            }
        }
    }

    /// The store's ids of `values` (symbols of this process's), in `connection`'s write transaction: unchanged unless
    /// forked; a pending atom gets its store id first.
    pub fn to_store(&self, connection: &Connection, values: &mut [i64]) -> StoreResult<()> {
        if !self.forked() {
            return Ok(());
        }
        if !writing(connection) {
            return Err(StoreError::Invalid("symbols are mapped to the store's in a write transaction".into()));
        }
        let mut state = self.state.write();
        let State { atoms, map, .. } = &mut *state;
        let map = map.as_mut().expect("forked");
        let fork = i64::from(map.fork);
        // A segment of an earlier transaction that rolled back is forgotten before its ids are used.
        if !map.open.is_empty() {
            validate(connection, map)?;
        }
        let wanted: Vec<Sym> = values.iter().filter(|&&v| v >= fork).map(|&v| v as Sym).filter(|&l| map.store_of(l).is_none()).collect();
        if !wanted.is_empty() {
            resolve(connection, atoms, map, &wanted)?;
        }
        for v in values.iter_mut().filter(|v| **v >= fork) {
            *v = i64::from(map.store_of(*v as Sym).ok_or_else(|| StoreError::Invalid(format!("atom {v} has no store id")))?);
        }
        Ok(())
    }

    /// This process's ids of `values` (symbols as the store holds them): unchanged unless forked; another process's
    /// atom is read and held here first.
    pub fn to_local(&self, values: &mut [i64]) -> StoreResult<()> {
        if !self.forked() {
            // Not forked: an id past this process's end is another process's, stored since. Learning the store's end
            // first moves this one's there, or, with atoms pending here, forks (decision 0170): either way the id is
            // not read as one of this process's own.
            let base = self.state.read().atoms.base() as i64;
            if !values.iter().any(|&v| v >= base) {
                return Ok(());
            }
            self.read_end()?;
            if !self.forked() {
                return Ok(());
            }
        }
        let missing: Vec<Sym> = {
            let state = self.state.read();
            let map = state.map.as_ref().expect("forked");
            values.iter().filter(|&&v| v >= i64::from(map.fork)).map(|&v| v as Sym).filter(|&v| map.local_of(v).is_none()).collect()
        };
        if !missing.is_empty() {
            self.with_reader(|connection| {
                let mut state = self.state.write();
                let State { atoms, map, .. } = &mut *state;
                hold(connection, atoms, map.as_mut().expect("forked"), &missing)
            })?;
        }
        let state = self.state.read();
        let map = state.map.as_ref().expect("forked");
        let fork = i64::from(map.fork);
        for v in values.iter_mut().filter(|v| **v >= fork) {
            *v = i64::from(map.local_of(*v as Sym).ok_or_else(|| StoreError::Corrupt(format!("store atom {v} is past the store's end")))?);
        }
        Ok(())
    }

    /// The store's ids of `values` (symbols of this process's) for a read, outside a write transaction: unchanged
    /// unless forked; an atom the store does not hold (pending here) is `None`, so a read keyed by it finds nothing.
    /// Nothing is appended.
    pub fn stored(&self, values: &[Sym]) -> StoreResult<Vec<Option<Sym>>> {
        if !self.forked() {
            return Ok(values.iter().map(|&v| Some(v)).collect());
        }
        let wanted: Vec<String> = {
            let state = self.state.read();
            let map = state.map.as_ref().expect("forked");
            values.iter().filter(|&&v| map.store_of(v).is_none()).filter_map(|&v| state.atoms.str(v).map(str::to_owned)).collect()
        };
        if !wanted.is_empty() {
            self.with_reader(|connection| {
                let mut state = self.state.write();
                let State { atoms, map, .. } = &mut *state;
                let map = map.as_mut().expect("forked");
                wanted.iter().try_for_each(|value| find(connection, atoms, map, value))
            })?;
        }
        let state = self.state.read();
        let map = state.map.as_ref().expect("forked");
        Ok(values.iter().map(|&v| map.store_of(v)).collect())
    }

    /// Refuses `what` once forked: it reads or writes symbols the map does not cover (decision 0170), so it fails
    /// rather than meaning other strings.
    pub fn refuse_forked(&self, what: &str) -> StoreResult<()> {
        if self.forked() {
            return Err(StoreError::Invalid(format!("{what}: {}", super::DIVERGED_TEXT)));
        }
        Ok(())
    }

    /// Forked: the store's atom of `value`, held here if another process stored it.
    pub(super) fn find_forked(&self, value: &str) -> StoreResult<()> {
        self.with_reader(|connection| {
            let mut state = self.state.write();
            let State { atoms, map, .. } = &mut *state;
            find(connection, atoms, map.as_mut().expect("forked"), value)
        })?;
        Ok(())
    }

    /// Forked: pending atoms (past the fork, with no store id).
    pub(super) fn pending_forked(&self, state: &State) -> Option<usize> {
        state.map.as_ref().map(|m| m.pending(state.atoms.len()))
    }
}

/// Maps the symbol columns (`types[c] == b's'`) of `rows` (flat, `types.len()` wide) with `f`.
pub fn map_columns(types: &[u8], rows: &mut [i64], f: impl FnOnce(&mut [i64]) -> StoreResult<()>) -> StoreResult<()> {
    let w = types.len();
    let syms: Vec<usize> = (0..w).filter(|&c| types[c] == b's').collect();
    if syms.is_empty() || rows.is_empty() {
        return Ok(());
    }
    let mut values: Vec<i64> = rows.chunks_exact(w).flat_map(|r| syms.iter().map(move |&c| r[c])).collect();
    f(&mut values)?;
    let mut it = values.into_iter();
    for r in rows.chunks_exact_mut(w) {
        for &c in &syms {
            r[c] = it.next().expect("one value a symbol column");
        }
    }
    Ok(())
}
