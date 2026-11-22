//! A fork's map between this process's atoms and the store's (decision 0170).
//!
//! Ids are given out without the lease (decision 0160), from this process's end. Another process may store atoms past
//! that end while some of this one's are pending: the two numbered different strings alike. Then this process forks.
//! Below the fork both agree. Past it, each keeps its own ids in memory (they index dense arrays, and every row it
//! holds names them), and the store keeps its own. Between them is this map:
//!
//! - **Strings first.** In a write transaction (SQLite's write lock held), the pending atoms are found in the store's
//!   blocks (decision 0401); a string it has takes its id, and the rest are appended to its open block. The map is local id to
//!   store id. A writer's batch does this before its jobs make their rows (`begin_write`), so they are made with the
//!   store's ids, not made and then made again.
//! - **Rows written** through a typed boundary (a frame's symbol columns, the memo's) are mapped local to store
//!   (`to_store`); **rows read** store to local (`to_local`). Another process's atom is read where it lies and
//!   interned here when a row read names it (`hold`): only the ids met.
//! - **Nothing else** holding a `Sym` is written or read after the fork: a write job that does not declare its rows
//!   mapped is dropped, as a cache's miss (`write_back::Syms`).
//!
//! An append made here is the map's only while it is in the store: a rolled-back savepoint or transaction takes it
//! away, and the next write sees that (`validate`) and forgets what it mapped.
use super::{Atoms, Interner, State, Sym, database_end, hash_str, stored};
use crate::content_store::{StoreError, StoreResult};
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::atomic::Ordering;

const NONE: Sym = Sym::MAX;

/// Symbol values as this process numbers them: what its rows, entries and arenas hold. Mapped to the store's by
/// `Atoms::to_store` alone (decision 0170). The brand is the caller's claim, made where the values are: a value read
/// from the store is a `StoreIds`, and passing it here instead is written as such, and reads as wrong.
pub struct ProcessIds<'a>(pub(super) &'a mut [i64]);

/// Symbol values as the store numbers them: what a frame, a row read or a write holds. Mapped to this process's by
/// `Atoms::to_local` alone (decision 0170).
pub struct StoreIds<'a>(pub(super) &'a mut [i64]);

impl<'a> ProcessIds<'a> {
    /// `values`, this process's ids: made by its own table, not read from the store.
    pub fn of_process(values: &'a mut [i64]) -> Self {
        Self(values)
    }

    /// The values, still this process's ids.
    pub fn values(&self) -> &[i64] {
        self.0
    }
}

impl<'a> StoreIds<'a> {
    /// `values`, the store's ids: read from it, not yet mapped.
    pub fn as_stored(values: &'a mut [i64]) -> Self {
        Self(values)
    }

    /// The values, still the store's ids.
    pub fn values(&self) -> &[i64] {
        self.0
    }
}

pub(super) struct Map {
    /// Ids below are the same here and in the store.
    pub(super) fork: Sym,
    /// The store's id of local atom `fork + i`, or `NONE` (pending).
    to_store: Vec<Sym>,
    /// The local id of each store id past `fork` met so far.
    to_local: HashMap<Sym, Sym>,
    /// How many of `to_store` are set.
    mapped: usize,
    /// Appends made here and not yet seen committed: their first id and the local atoms they hold.
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
        if store < self.fork {
            return Some(store);
        }
        self.to_local.get(&store).copied()
    }

    /// Store atom `store` reads as local atom `local`.
    fn link(&mut self, store: Sym, local: Sym) {
        self.to_local.insert(store, local);
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
            self.to_local.remove(s);
            *s = NONE;
            self.mapped -= 1;
        }
    }

    /// Local atoms past the fork with no store id yet, or one in an append not yet seen committed (its transaction may
    /// still roll back).
    pub(super) fn pending(&self, len: usize) -> usize {
        len - self.fork as usize - self.mapped + self.open.iter().map(|(_, l)| l.len()).sum::<usize>()
    }
}

/// Holds store atoms `ids` past the fork not met yet: each string read where it lies in the store's blocks (read to
/// the store's end on `connection`) and interned here; one this process holds already maps to its atom. Below the fork
/// every stored atom reads as itself.
fn hold(connection: &Connection, atoms: &mut Interner, map: &mut Map, ids: &[Sym]) -> StoreResult<()> {
    let mut ids: Vec<Sym> = ids.iter().copied().filter(|&id| id >= map.fork && map.local_of(id).is_none()).collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Ok(());
    }
    atoms.view_store(&stored::rows(connection)?)?;
    for id in ids {
        let Some(s) = atoms.store_str(id).map(str::to_owned) else { continue };
        let local = atoms.intern(&s);
        if local >= map.fork && map.store_of(local).is_none() {
            map.set(local, id);
        } else {
            // Held under another id already (mapped): reads of `id` still find it.
            map.link(id, local);
        }
    }
    Ok(())
}

/// The store's atoms of `values` past the fork, held here: found in its blocks' tables.
fn find(connection: &Connection, atoms: &mut Interner, map: &mut Map, values: &[&str]) -> StoreResult<()> {
    if values.is_empty() {
        return Ok(());
    }
    atoms.view_store(&stored::rows(connection)?)?;
    let ids: Vec<Sym> = values.iter().filter_map(|v| atoms.store_find(v, hash_str(v))).filter(|&id| id >= map.fork).collect();
    hold(connection, atoms, map, &ids)
}

/// Forgets what appends rolled back took away (a failed savepoint or transaction): their atoms are pending again. An
/// append is there when the store's atoms at its ids are its strings.
fn validate(connection: &Connection, atoms: &mut Interner, map: &mut Map) -> StoreResult<()> {
    if map.open.is_empty() {
        return Ok(());
    }
    atoms.view_store(&stored::rows(connection)?)?;
    let mut kept = Vec::new();
    for (first, locals) in std::mem::take(&mut map.open) {
        let there = locals.iter().enumerate().all(|(k, &l)| atoms.store_str(first + k as Sym).is_some() && atoms.store_str(first + k as Sym) == atoms.str(l));
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
/// takes its id; the rest are appended to its open block. Returns how many were appended.
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
    // Another process may have stored the string past the fork: found in the blocks, read to the store's end.
    let rows = stored::rows(connection)?;
    atoms.view_store(&rows)?;
    let mut new = Vec::new();
    for l in wanted {
        let s = atoms.str(l).expect("a local atom");
        match atoms.store_find(s, hash_str(s)) {
            // The store's, and read as `l`.
            Some(id) => map.set(l, id),
            None => new.push(l),
        }
    }
    if new.is_empty() {
        return Ok(0);
    }
    // Appended at the store's end, in the transaction.
    let first = stored::end_of(&rows);
    let run: Vec<(String, u64)> = new.iter().map(|&l| atoms.str(l).expect("a local atom")).map(|s| (s.to_owned(), hash_str(s))).collect();
    atoms.stored.append(connection, first, run.iter().map(|(s, h)| (s.as_str(), *h)))?;
    for (k, &l) in new.iter().enumerate() {
        map.set(l, first + k as Sym);
    }
    map.open.push((first, new.clone()));
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

    /// Below which id this process reads every stored atom by the store's numbering: `None` once forked (its ids past
    /// the fork are its own) or with no store. A row made under the store's ids below it names atoms read here.
    pub fn held_below(&self) -> Option<usize> {
        if self.path.is_none() || self.forked() {
            return None;
        }
        let state = self.state.read();
        (state.map.is_none()).then_some(state.atoms.base())
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
            if !(end > base && end != self.wrote.load(Ordering::Acquire)) {
                return Ok(());
            }
            // Another process's atoms and none pending here: this process's end moves past them now, while this
            // transaction holds the write lock, so an id given out while it runs is not one of theirs.
            if len == base {
                return self.advance_holding_in(connection, &mut state, base, end);
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
        validate(connection, atoms, map)?;
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
        let State { atoms, map, .. } = &mut *state;
        if let Some(map) = map.as_mut() {
            validate(connection, atoms, map)?;
        }
        Ok(0)
    }

    /// Forked: after a commit, every append made in it is the store's.
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
    pub fn to_store<'v>(&self, connection: &Connection, ids: ProcessIds<'v>) -> StoreResult<StoreIds<'v>> {
        let values = ids.0;
        if !self.forked() {
            return Ok(StoreIds(values));
        }
        if !writing(connection) {
            return Err(StoreError::Invalid("symbols are mapped to the store's in a write transaction".into()));
        }
        let mut state = self.state.write();
        let State { atoms, map, .. } = &mut *state;
        let map = map.as_mut().expect("forked");
        let fork = i64::from(map.fork);
        // An append of an earlier transaction that rolled back is forgotten before its ids are used.
        if !map.open.is_empty() {
            validate(connection, atoms, map)?;
        }
        let wanted: Vec<Sym> = values.iter().filter(|&&v| v >= fork).map(|&v| v as Sym).filter(|&l| map.store_of(l).is_none()).collect();
        if !wanted.is_empty() {
            resolve(connection, atoms, map, &wanted)?;
        }
        for v in values.iter_mut().filter(|v| **v >= fork) {
            *v = i64::from(map.store_of(*v as Sym).ok_or_else(|| StoreError::Invalid(format!("atom {v} has no store id")))?);
        }
        Ok(StoreIds(values))
    }

    /// This process's ids of `values` (symbols as the store holds them): unchanged unless forked; another process's
    /// atom is read and held here first.
    pub fn to_local<'v>(&self, ids: StoreIds<'v>) -> StoreResult<ProcessIds<'v>> {
        let values = ids.0;
        if !self.forked() {
            // Not forked: an id past this process's end is another process's, stored since. Learning the store's end
            // first moves this one's there, or, with atoms pending here, forks (decision 0170): either way the id is
            // not read as one of this process's own.
            let base = self.state.read().atoms.base() as i64;
            if !values.iter().any(|&v| v >= base) {
                return Ok(ProcessIds(values));
            }
            self.read_end()?;
            if !self.forked() {
                return Ok(ProcessIds(values));
            }
        }
        let missing: Vec<Sym> = {
            let state = self.state.read();
            let map = state.map.as_ref().expect("forked");
            let fork = map.fork;
            // Past the fork, those not met; below it, every one reads as itself.
            values.iter().map(|&v| v as Sym).filter(|&v| v >= fork && map.local_of(v).is_none()).collect()
        };
        if !missing.is_empty() {
            self.hold_forked(&missing)?;
        }
        let state = self.state.read();
        let map = state.map.as_ref().expect("forked");
        let fork = i64::from(map.fork);
        for v in values.iter_mut().filter(|v| **v >= fork) {
            *v = i64::from(map.local_of(*v as Sym).ok_or_else(|| StoreError::Corrupt(format!("store atom {v} is past the store's end")))?);
        }
        Ok(ProcessIds(values))
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

    /// Forked: holds store atoms `ids` past the fork (their strings, read where they lie).
    pub(super) fn hold_forked(&self, ids: &[Sym]) -> StoreResult<()> {
        self.with_reader(|connection| {
            let mut state = self.state.write();
            let State { atoms, map, .. } = &mut *state;
            hold(connection, atoms, map.as_mut().expect("forked"), ids)
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

impl Atoms {
    /// Mapped: each of store atoms `ids` the store holds, with this process's atom of its string (held here first, in
    /// one read).
    pub(super) fn stored_pairs(&self, ids: &[Sym]) -> StoreResult<Vec<(Sym, Sym)>> {
        self.hold_forked(ids)?;
        let state = self.state.read();
        let map = state.map.as_ref().expect("mapped");
        Ok(ids.iter().filter_map(|&id| map.local_of(id).filter(|&l| state.atoms.str(l).is_some()).map(|l| (id, l))).collect())
    }

    /// Mapped: every store atom, with this process's atom of its string (those past the fork held here).
    pub(super) fn stored_pairs_all(&self) -> StoreResult<Vec<(Sym, Sym)>> {
        let end = self.with_reader(|connection| Ok(database_end(connection)?))?.unwrap_or(0);
        self.stored_pairs(&(0..end as Sym).collect::<Vec<_>>())
    }
}

/// Every stored column that holds atoms, and how it crosses the store's boundary when this process's numbering differs
/// from the store's (decision 0170). With `NOT_ATOMS`, it classifies every integer, untyped and blob column of the
/// store's schema: a test (`every_stored_column_is_classified`) fails on a column in neither, so a new column of atoms
/// is mapped, or said to be none, before it is kept. Only that test reads the two tables.
#[cfg(test)]
pub(crate) const ATOM_COLUMNS: &[(&str, &str, &str)] = &[
    ("atom_blocks", "first", "the store's atoms themselves: their numbering is the store's"),
    ("atom_blocks", "upto", "the store's atoms themselves"),
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
    ("source_objects", "frame", "content-addressed sources: their makers keep no atom (tapes, program texts, tree counts of integer constants, the planning choices' own string table)"),
    ("stat_blob", "blob", "the kept statistics' exact counts of `sym` columns: mapped as written and read (keptstats.rs `Keeper::map_symbols`)"),
];

/// The integer, untyped and blob columns of the store's schema that hold no atom (ids of the store's own rows,
/// counts, digests, encoded bytes of integers or text). `fact_value_stats.val`: the slice's scope (text) and clause
/// numbers (`matched(c: number, …)`).
#[cfg(test)]
pub(crate) const NOT_ATOMS: &[(&str, &[&str])] = &[
    ("atom_blocks", &["seq", "tag"]),
    ("candidate_catalogs", &["catalog_id", "identity_hash_id"]),
    ("candidate_checks", &["check_id", "catalog_id", "policy_hash_id", "check_string_id"]),
    ("completed_directory_reuse", &["tree_hash_id", "package_hash_id", "catalog_hash_id"]),
    ("completed_workspace_revisions", &["workspace_id", "revision", "catalog_hash_id", "completed_at"]),
    ("context_check_results", &["source_id", "context_hash_id", "check_id", "seg", "seg_at", "seg_len"]),
    ("corpus_decided", &["singleton", "run", "time", "swept"]),
    ("corpus_live", &["source_id", "grammar_version", "tape_format"]),
    ("corpus_manifest_plans", &["manifest_id", "plan_digest"]),
    ("corpus_manifests", &["manifest_id", "root_id", "last_run", "last_time", "grammar_version", "tape_format", "plan_digest", "digest", "sources", "lane", "policy"]),
    ("corpus_plan_relations", &["plan_digest"]),
    ("corpus_plans", &["plan_digest", "live"]),
    ("corpus_roots", &["root_id"]),
    ("corpus_state", &["singleton", "run", "history_bytes", "due", "archive_due"]),
    ("derivation_inputs", &["ordinal", "digest"]),
    ("derivations", &["producer", "status", "cost"]),
    ("edge_freq", &["pkind", "ckind", "field", "n"]),
    ("fact_stats", &["rows", "ndv", "sq", "sqd"]),
    ("fact_value_stats", &["val", "n", "docs", "sqd"]),
    ("facts_chain", &["space", "seq", "tag"]),
    ("file_documents", &["tape_id", "ordinal", "parent_ordinal", "role_id", "via_id", "document_tape_id", "source_map", "host_errors", "at_start", "at_end"]),
    // The per-file memo's entries (decision 0240 §4): their symbols are strings in the pack (memo/image.rs).
    ("memo_entries", &["space", "part", "digest", "doc", "pack", "at", "len", "raw", "over_pack", "over_at", "over_len", "over_raw", "framed"]),
    ("memo_packs", &["pack"]),
    // A guest's identity (a digest) and its key (decision 0391).
    ("guest_keys", &["identity", "key"]),
    ("git_tree_entries", &["tree_id", "name_string_id", "mode_id", "object_kind_id", "target_hash_id"]),
    ("git_trees", &["tree_id", "tree_hash_id", "complete", "entry_count"]),
    ("hashes", &["hash_id", "algorithm_id", "lane0", "lane1", "lane2", "lane3"]),
    ("kind_size", &["kind", "n", "size"]),
    ("name_bindings", &["digest", "revision"]),
    ("object_archive", &["kind", "id", "frame"]),
    ("object_pins", &["object_kind_id", "object_id", "owner_string_id", "lease_until"]),
    ("paths", &["path_id", "string_id"]),
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
    ("source_objects", &["source_id", "canonical_hash_id", "raw_length", "codec_id", "created_generation", "seg", "seg_at", "seg_len"]),
    ("object_segments", &["seg"]),
    ("sqlite_sequence", &["name", "seq"]),
    ("store_identity", &["singleton", "id"]),
    ("store_sessions", &["kind", "key", "latest", "last_used", "ended", "aged_bytes"]),
    ("analysis_scope_collect", &["snapshot_id"]),
    ("analysis_scope_generation", &["generation"]),
    ("analysis_directory_acquisition", &["max_control_generation"]),
    ("analysis_scope_manifest", &["manifest_id", "acquired_at", "acquisition", "recorded_run"]),
    ("analysis_run_workspace_snapshot", &["acquisition", "snapshot_id"]),
    ("analysis_scope_run", &["acquisition", "acquired_at"]),
    ("analysis_scope_release", &["before", "legacy_acquisition"]),
    ("analysis_scope_snapshot", &["snapshot_id", "acquired_at"]),
    ("analysis_scope_workspace_snapshot", &["snapshot_id", "acquired_at", "acquisition"]),
    ("store_state", &["singleton", "generation", "analysis_acquisitions"]),
    ("strings", &["string_id", "domain_id", "content_hash_id", "collision_ordinal"]),
    ("tape_kind_counts", &["tape_id", "kind", "n", "size", "postings"]),
    ("tape_objects", &["tape_id", "source_id", "canonical_hash_id", "grammar_id", "grammar_version", "tape_format_version", "codec_id", "raw_length", "frame", "created_generation", "seg", "seg_at", "seg_len", "programs"]),
    ("term_coverage", &["tape_id", "kind", "bad", "residue", "missing"]),
    ("term_freq", &["lang", "n"]),
    ("term_kind_freq", &["kind", "n"]),
    ("term_node_freq", &["lang", "kind", "n"]),
    ("term_postings", &["lang", "tape_id", "kind", "n", "postings"]),
    ("text_matcher", &["lang", "upto"]),
    ("text_pending", &["seq", "lang"]),
    ("text_term", &["lang"]),
    ("vocabulary_literal", &["lang"]),
    ("vocabulary_pending", &["lang"]),
    ("vocabulary_predicate", &["lang", "ready"]),
    ("vocabulary_term", &["lang"]),
    ("workspace_overlay", &["workspace_id", "path_id", "state_id", "source_id", "tape_id"]),
    ("workspaces", &["workspace_id", "root_path_id", "repository_string_id", "owner_string_id", "snapshot_id", "generation", "lease_until", "state_id"]),
];
