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
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::atomic::Ordering;

const NONE: Sym = Sym::MAX;

pub(super) struct Map {
    /// Ids below are the same here and in the store, but those in `to_local` (`below`).
    pub(super) fork: Sym,
    /// The store's id of local atom `fork + i`, or `NONE` (pending).
    to_store: Vec<Sym>,
    /// The local id of each store id past `fork` met so far, and of each below it whose string this process holds
    /// under an id of its own (given out before the store's was held: a run over a store it does not hold whole).
    to_local: HashMap<Sym, Sym>,
    /// How many of `to_local`'s store ids are below `fork`: with none, those read as themselves without a lookup.
    below: usize,
    /// How many of `to_store` are set.
    mapped: usize,
    /// Segments appended here and not yet seen committed: their first id and the local atoms they hold.
    open: Vec<(Sym, Vec<Sym>)>,
}

impl Map {
    fn new(fork: Sym) -> Map {
        Map { fork, to_store: Vec::new(), to_local: HashMap::new(), below: 0, mapped: 0, open: Vec::new() }
    }

    fn store_of(&self, local: Sym) -> Option<Sym> {
        if local < self.fork {
            return Some(local);
        }
        self.to_store.get((local - self.fork) as usize).copied().filter(|&s| s != NONE)
    }

    pub(super) fn local_of(&self, store: Sym) -> Option<Sym> {
        if store < self.fork {
            if self.below == 0 {
                return Some(store);
            }
            return Some(self.to_local.get(&store).copied().unwrap_or(store));
        }
        self.to_local.get(&store).copied()
    }

    /// Store atom `store` reads as local atom `local`.
    fn link(&mut self, store: Sym, local: Sym) {
        if self.to_local.insert(store, local).is_none() && store < self.fork {
            self.below += 1;
        }
    }

    /// Whether store atom `store` (below the fork) reads as another local atom.
    pub(super) fn relinked(&self, store: Sym) -> bool {
        self.below > 0 && self.to_local.contains_key(&store)
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
        self.link(store, local);
    }

    fn unset(&mut self, local: Sym) {
        let i = (local - self.fork) as usize;
        if let Some(s) = self.to_store.get_mut(i).filter(|s| **s != NONE) {
            if self.to_local.remove(s).is_some() && *s < self.fork {
                self.below -= 1;
            }
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

/// The store's segments holding `ids`, read in one statement: the segments' bounds first (a few hundred rows), then
/// the ones holding an id, by their first ids. Each is sliced per string: decoding a whole segment (64 MB, 450,000
/// atoms) for each string looked up held the write lock for seconds while every other process's writer waited.
pub(super) fn fetch(connection: &Connection, ids: &[Sym]) -> StoreResult<Vec<(Sym, Vec<u8>, Vec<u8>)>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let bounds: Vec<(Sym, Sym)> = connection
        .prepare_cached("SELECT first, length(ends) / 4 FROM atom_segment ORDER BY first")?
        .query_map([], |r| Ok((r.get::<_, i64>(0)? as Sym, r.get::<_, i64>(1)? as Sym)))?
        .collect::<Result<_, _>>()?;
    let mut ids = ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    let mut firsts: Vec<i64> = Vec::new();
    let mut k = 0;
    for id in ids {
        while k < bounds.len() && bounds[k].0 + bounds[k].1 <= id {
            k += 1;
        }
        if k < bounds.len() && bounds[k].0 <= id && firsts.last() != Some(&i64::from(bounds[k].0)) {
            firsts.push(i64::from(bounds[k].0));
        }
    }
    if firsts.is_empty() {
        return Ok(Vec::new());
    }
    let firsts: Vec<rusqlite::types::Value> = firsts.into_iter().map(rusqlite::types::Value::Integer).collect();
    Ok(connection
        .prepare_cached("SELECT first, ends, bytes FROM atom_segment WHERE first IN rarray(?1) ORDER BY first")?
        .query_map([std::rc::Rc::new(firsts)], |r| Ok((r.get::<_, i64>(0)? as Sym, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?)
}

/// Segments read for one batch of ids (`fetch`), sliced per string.
struct Segments {
    held: std::collections::BTreeMap<Sym, (Vec<u8>, Vec<u8>)>,
}

impl Segments {
    fn read(connection: &Connection, ids: &[Sym]) -> StoreResult<Self> {
        Ok(Segments { held: fetch(connection, ids)?.into_iter().map(|(first, ends, bytes)| (first, (ends, bytes))).collect() })
    }

    /// The store's string of `id`, if a segment read holds it.
    fn string(&self, id: Sym) -> StoreResult<Option<String>> {
        let Some((&first, (ends, bytes))) = self.held.range(..=id).next_back() else { return Ok(None) };
        let i = (id - first) as usize;
        if i >= ends.len() / 4 {
            return Ok(None);
        }
        let at = |k: usize| u32::from_le_bytes(ends[k * 4..k * 4 + 4].try_into().expect("4 bytes")) as usize;
        let (start, end) = (if i == 0 { 0 } else { at(i - 1) }, at(i));
        let s = bytes.get(start..end).and_then(|s| std::str::from_utf8(s).ok()).ok_or_else(|| StoreError::Corrupt(format!("atom segment {first}: a string is not UTF-8 or out of its bytes")))?;
        Ok(Some(s.to_owned()))
    }
}

/// Holds store atoms `ids` not met yet, in one read of their segments (`fetch`). Past the fork, each string is
/// interned here; one this process holds already maps to its atom. Below it, the segments are held as they are, but a
/// string this process gave an id of its own first (a run over a store it did not hold whole, decision 0170) reads as
/// that id, and that id is the store's there.
fn hold(connection: &Connection, atoms: &mut Interner, map: &mut Map, ids: &[Sym]) -> StoreResult<()> {
    let mut ids: Vec<Sym> = ids
        .iter()
        .copied()
        .filter(|&id| if id < map.fork { !map.relinked(id) && !atoms.holds(id) } else { map.local_of(id).is_none() })
        .collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Ok(());
    }
    let segments = Segments::read(connection, &ids)?;
    let mut dups = Vec::new();
    for (&first, (ends, bytes)) in &segments.held {
        if first < map.fork {
            atoms.add_segment(first, ends, bytes, Some(&mut dups))?;
        }
    }
    for (store, local) in dups {
        if local >= map.fork && map.store_of(local).is_none() {
            map.set(local, store);
        } else {
            map.link(store, local);
        }
    }
    let fork = map.fork;
    for id in ids.into_iter().filter(|&id| id >= fork) {
        let Some(s) = segments.string(id)? else { continue };
        let local = atoms.intern(&s);
        if local >= map.fork && map.store_of(local).is_none() {
            map.set(local, id);
        } else {
            // Held under another id already (below the fork, or mapped): reads of `id` still find it.
            map.link(id, local);
        }
    }
    Ok(())
}

/// The store's atoms of `values`, held here: found by their key, in one statement.
fn find(connection: &Connection, atoms: &mut Interner, map: &mut Map, values: &[&str]) -> StoreResult<()> {
    if values.is_empty() {
        return Ok(());
    }
    let hashes: Vec<rusqlite::types::Value> = values.iter().map(|v| rusqlite::types::Value::Integer(hash_str(v) as i64)).collect();
    let ids: Vec<i64> = connection
        .prepare_cached("SELECT id FROM atom_key WHERE h IN rarray(?1)")?
        .query_map([std::rc::Rc::new(hashes)], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let ids: Vec<Sym> = ids.into_iter().map(|id| id as Sym).collect();
    hold(connection, atoms, map, &ids)
}

/// Forgets what segments rolled back took away (a failed savepoint or transaction): their atoms are pending again.
fn validate(connection: &Connection, map: &mut Map) -> StoreResult<()> {
    if map.open.is_empty() {
        return Ok(());
    }
    let firsts: Vec<rusqlite::types::Value> = map.open.iter().map(|(first, _)| rusqlite::types::Value::Integer(i64::from(*first))).collect();
    let there: std::collections::HashSet<i64> = connection
        .prepare_cached("SELECT first FROM atom_segment WHERE first IN rarray(?1)")?
        .query_map([std::rc::Rc::new(firsts)], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let mut kept = Vec::new();
    for (first, locals) in std::mem::take(&mut map.open) {
        if there.contains(&i64::from(first)) {
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
    let candidates: Vec<Sym> = by_hash.values().flatten().copied().filter(|&id| map.local_of(id).is_none_or(|x| atoms.str(x).is_none())).collect();
    let segments = Segments::read(connection, &candidates)?;
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
            // The store's, and read as `l` (below the fork too: this process does not hold it there, or it would
            // have found it when it gave out `l`).
            Some(id) => map.set(l, id),
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
            crate::content_store::bounded_insert::insert(
                connection,
                "INSERT OR IGNORE INTO atom_key(h, id)",
                "",
                keys.into_iter().map(|(h, id)| [rusqlite::types::Value::Integer(h), rusqlite::types::Value::Integer(id)]),
            )?;
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
            let fork = map.fork;
            // Past the fork, those not met; below it, those not held here (a run over a store it does not hold whole
            // holds what its rows name as they are read, `hold_named`, so its waves find them here).
            values
                .iter()
                .map(|&v| v as Sym)
                .filter(|&v| if v >= fork { map.local_of(v).is_none() } else { !map.relinked(v) && !state.atoms.holds(v) })
                .collect()
        };
        if !missing.is_empty() {
            self.hold_forked(&missing)?;
        }
        let state = self.state.read();
        let map = state.map.as_ref().expect("forked");
        let fork = i64::from(map.fork);
        let below = map.below > 0;
        for v in values.iter_mut().filter(|v| below || **v >= fork) {
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
            self.find_forked(&wanted.iter().map(String::as_str).collect::<Vec<_>>())?;
        }
        let state = self.state.read();
        let map = state.map.as_ref().expect("forked");
        Ok(values.iter().map(|&v| map.store_of(v)).collect())
    }

    /// Forked: the store's atoms of `values`, held here, in one read.
    pub(super) fn find_forked(&self, values: &[&str]) -> StoreResult<()> {
        self.with_reader(|connection| {
            let mut state = self.state.write();
            let State { atoms, map, .. } = &mut *state;
            find(connection, atoms, map.as_mut().expect("forked"), values)
        })?;
        Ok(())
    }

    /// Forked: holds store atoms `ids` (below the fork, their segments; past it, their strings), in one read.
    pub(super) fn hold_forked(&self, ids: &[Sym]) -> StoreResult<()> {
        self.with_reader(|connection| {
            let mut state = self.state.write();
            let State { atoms, map, .. } = &mut *state;
            hold(connection, atoms, map.as_mut().expect("forked"), ids)
        })?;
        Ok(())
    }

    /// Before a run over a store it does not hold whole (past `READ_WHOLE`): this process maps from its end, as a fork
    /// does, so that its waves give out ids without reading the store (decision 0170). A string the store has under
    /// an id this process does not hold gets one of its own, and the writer finds the store's by its key. One string
    /// keeps one id within the run because every id its stored rows name is held as the rows are read (`hold_named`).
    pub(super) fn map_from_end(&self) {
        let mut state = self.state.write();
        if state.map.is_some() {
            return;
        }
        let base = state.atoms.base() as Sym;
        state.map = Some(Map::new(base));
        self.forked.store(true, Ordering::Release);
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

/// Holds a stored segment (`Interner::add_segment`). Mapped, a string this process holds under an id of its own (given
/// out before the segment was held) is not held twice: the store's id reads as it, and it is the store's there.
pub(super) fn add_stored(state: &mut State, first: Sym, ends: &[u8], bytes: &[u8]) -> StoreResult<()> {
    let State { atoms, map, .. } = state;
    let Some(map) = map.as_mut() else { return atoms.add_segment(first, ends, bytes, None) };
    let mut dups = Vec::new();
    atoms.add_segment(first, ends, bytes, Some(&mut dups))?;
    for (store, local) in dups {
        if local >= map.fork && map.store_of(local).is_none() {
            map.set(local, store);
        } else {
            map.link(store, local);
        }
    }
    Ok(())
}

impl Atoms {
    /// Mapped: each of store atoms `ids` the store holds, with this process's atom of its string (held here first, in
    /// one read).
    pub(super) fn stored_pairs(&self, ids: &[Sym]) -> StoreResult<Vec<(Sym, Sym)>> {
        self.hold_forked(ids)?;
        let state = self.state.read();
        let map = state.map.as_ref().expect("mapped");
        Ok(ids.iter().filter_map(|&id| map.local_of(id).filter(|&l| state.atoms.str(l).is_some()).map(|l| (id, l))).collect())
    }

    /// Mapped: every store atom, with this process's atom of its string (all held here, in one read).
    pub(super) fn stored_pairs_all(&self) -> StoreResult<Vec<(Sym, Sym)>> {
        let end = self.with_reader(|connection| Ok(database_end(connection)?))?.unwrap_or(0);
        self.stored_pairs(&(0..end as Sym).collect::<Vec<_>>())
    }
}

/// Every stored column that holds atoms, and how it crosses the store's boundary when this process's numbering differs
/// from the store's (decision 0170). With `NOT_ATOMS`, it classifies every integer, untyped and blob column of the
/// store's schema: a test (`every_stored_column_is_classified`) fails on a column in neither, so a new column of atoms
/// is mapped, or said to be none, before it is kept.
pub(crate) const ATOM_COLUMNS: &[(&str, &str, &str)] = &[
    ("atom_segment", "first", "the store's atoms themselves: their numbering is the store's"),
    ("atom_segment", "ends", "the store's atoms themselves"),
    ("atom_segment", "bytes", "the store's atoms themselves"),
    ("atom_key", "h", "the store's key of its atoms (a string's hash)"),
    ("atom_key", "id", "the store's key of its atoms"),
    ("atom_key_end", "upto", "the store's key of its atoms"),
    ("name_bindings", "name_atom", "derivation.rs: mapped in the write transaction (`syms_to_store`), and back as read"),
    ("derivations", "output_atom", "derivation.rs: mapped in the write transaction (`derivations_to_store`), and back as read"),
    ("derivation_inputs", "output_atom", "derivation.rs: as `derivations`"),
    ("derivation_inputs", "name_atom", "derivation.rs: as `derivations`"),
    ("term_postings", "term", "prefilter.rs `Counts::flush`: mapped in the write transaction"),
    ("term_freq", "term", "prefilter.rs `Counts::flush`: mapped in the write transaction"),
    ("term_node_freq", "term", "prefilter.rs `Counts::flush`: mapped in the write transaction"),
    ("vocabulary_term", "term", "SQL over `term_freq` and the `atoms` table, which answers in the store's ids (vtab.rs)"),
    ("vocabulary_pending", "term", "a trigger on `term_freq`"),
    ("text_term", "term", "SQL over `term_freq` and the `atoms` table, in the store's ids (vocabulary.rs)"),
    ("text_pending", "term", "a trigger on `term_freq`"),
    ("frames", "data", "typed columns (`b's'`): mapped in the write transaction (frames.rs `keep_frame_refs`) and as decoded (`rows_of`); the memo's frames (`local`, `local_out`) by the plan's declarations (memo.rs `map_entries`)"),
    ("cross_blocks", "data", "the relations' `sym` columns (cross_memo.rs `Plan::syms`): mapped in `put_cross_rows` and `cross_rows`"),
    ("compiled_plans", "program", "a wave plan's SQL in this process's ids: reused only where the names it records intern to the same ids here (matches.rs `Wave::emit`)"),
    ("source_objects", "frame", "content-addressed sources: their makers keep no atom (tapes, program texts, tree counts of integer constants, the planning choices' own string table)"),
    ("stat_blob", "blob", "the kept statistics' exact counts of `sym` columns: mapped as written and read (keptstats.rs `Keeper::map_symbols`)"),
];

/// The integer, untyped and blob columns of the store's schema that hold no atom (ids of the store's own rows,
/// counts, digests, encoded bytes of integers or text). `fact_value_stats.val`: the slice's scope (text) and clause
/// numbers (`matched(c: number, …)`).
pub(crate) const NOT_ATOMS: &[(&str, &[&str])] = &[
    ("accel_models", &["model_key", "archive"]),
    ("atom_key_end", &["one"]),
    ("candidate_catalogs", &["catalog_id", "identity_hash_id"]),
    ("candidate_checks", &["check_id", "catalog_id", "policy_hash_id", "check_string_id"]),
    ("compiled_plan_reads", &["plan", "lo", "hi"]),
    ("compiled_plans", &["plan", "compiler"]),
    ("completed_directory_reuse", &["tree_hash_id", "package_hash_id", "catalog_hash_id"]),
    ("completed_workspace_revisions", &["workspace_id", "revision", "catalog_hash_id", "completed_at"]),
    ("context_check_results", &["source_id", "context_hash_id", "check_id", "codec_id", "raw_length", "frame"]),
    ("corpus_decided", &["singleton", "run", "time"]),
    ("corpus_live", &["source_id", "grammar_version", "tape_format"]),
    ("corpus_manifest_plans", &["manifest_id", "plan_digest"]),
    ("corpus_manifests", &["manifest_id", "root_id", "last_run", "last_time", "grammar_version", "tape_format", "plan_digest", "digest", "sources", "lane"]),
    ("corpus_plan_relations", &["plan_digest"]),
    ("corpus_plans", &["plan_digest", "live"]),
    ("corpus_roots", &["root_id"]),
    ("corpus_state", &["singleton", "run", "history_bytes", "due"]),
    ("corpus_stats", &["root_id", "size", "mtime_ns", "inode", "source_id", "recorded_ns"]),
    ("corpus_swept", &["singleton", "done"]),
    ("cross_blocks", &["source_id", "width", "rows"]),
    ("cross_components", &["source_id"]),
    ("cross_rows", &["source_id", "seq", "c0", "c1", "c2", "c3", "c4", "c5", "c6", "c7"]),
    ("derivation_inputs", &["ordinal", "digest"]),
    ("derivations", &["producer", "status", "cost"]),
    ("edge_freq", &["pkind", "ckind", "field", "n"]),
    ("fact_stats", &["rows", "ndv", "sq", "sqd"]),
    ("fact_value_stats", &["val", "n", "docs", "sqd"]),
    ("facts_completions", &["source_id", "fact_schema", "set_digest", "words", "codec_id", "raw_length", "counts"]),
    ("facts_plan_sets", &["set_digest", "ordinal", "plan_digest"]),
    ("facts_results", &["source_id", "fact_schema", "plan_digest", "codec_id", "raw_length", "frame"]),
    ("file_documents", &["tape_id", "ordinal", "parent_ordinal", "role_id", "via_id", "document_tape_id", "source_map", "host_errors"]),
    ("frame_confirmed", &["space", "data"]),
    ("frame_part_used", &["space", "ident", "part", "used"]),
    ("frame_program_used", &["space", "program", "run", "rels"]),
    ("frame_spaces", &["space", "runs"]),
    ("frame_used", &["space", "run", "rels"]),
    ("frames", &["space", "ident", "part", "digest", "reads", "width", "rows", "types", "used"]),
    ("git_tree_entries", &["tree_id", "name_string_id", "mode_id", "object_kind_id", "target_hash_id"]),
    ("git_trees", &["tree_id", "tree_hash_id", "complete", "entry_count"]),
    ("hashes", &["hash_id", "algorithm_id", "lane0", "lane1", "lane2", "lane3"]),
    ("kind_size", &["kind", "n", "size"]),
    ("local_check_results", &["source_id", "check_id", "codec_id", "raw_length", "frame"]),
    ("name_bindings", &["digest", "revision"]),
    ("object_pins", &["object_kind_id", "object_id", "owner_string_id", "lease_until"]),
    ("paths", &["path_id", "string_id"]),
    ("plan_feedback", &["plan", "samples", "updated_at"]),
    ("prefilter_sets", &["tape_id", "version", "nodes", "edges"]),
    ("push_pending", &["pending"]),
    ("search_index_sets", &["tape_id", "index_version", "node_count", "complete", "has_errors"]),
    ("search_kind_fts", &["token"]),
    ("search_kind_fts_config", &["k", "v"]),
    ("search_kind_fts_data", &["id", "block"]),
    ("search_kind_fts_docsize", &["id", "sz", "origin"]),
    ("search_kind_fts_idx", &["segid", "term", "pgno"]),
    ("search_kind_vocab", &["term", "doc", "cnt"]),
    ("snapshot_files", &["snapshot_id", "path_id", "source_id", "tape_id"]),
    ("snapshots", &["snapshot_id", "root_tree_id", "scanner_version", "grammar_set_hash_id", "tape_format_version"]),
    ("source_aliases", &["alias_hash_id", "source_id"]),
    ("source_objects", &["source_id", "canonical_hash_id", "raw_length", "codec_id", "created_generation"]),
    ("sqlite_sequence", &["name", "seq"]),
    ("store_identity", &["singleton", "id"]),
    ("store_state", &["singleton", "generation"]),
    ("strings", &["string_id", "domain_id", "content_hash_id", "collision_ordinal"]),
    ("tape_kind_counts", &["tape_id", "kind", "n", "size", "postings"]),
    ("tape_objects", &["tape_id", "source_id", "canonical_hash_id", "grammar_id", "grammar_version", "tape_format_version", "codec_id", "raw_length", "frame", "created_generation"]),
    ("term_coverage", &["tape_id", "kind", "bad", "residue", "missing"]),
    ("term_freq", &["lang", "n"]),
    ("term_kind_freq", &["kind", "n"]),
    ("term_node_freq", &["lang", "kind", "n"]),
    ("term_postings", &["lang", "tape_id", "kind", "n", "postings"]),
    ("text_matcher", &["lang", "upto"]),
    ("text_pending", &["seq", "lang"]),
    ("text_term", &["lang"]),
    ("tree_pages", &["page_key", "page_index", "page_count", "digest", "codec_id", "raw_length", "file_count", "frame"]),
    ("vocabulary_literal", &["lang"]),
    ("vocabulary_pending", &["lang"]),
    ("vocabulary_predicate", &["lang", "ready"]),
    ("vocabulary_term", &["lang"]),
    ("workspace_overlay", &["workspace_id", "path_id", "state_id", "source_id", "tape_id"]),
    ("workspaces", &["workspace_id", "root_path_id", "repository_string_id", "owner_string_id", "snapshot_id", "generation", "lease_until", "state_id"]),
];
