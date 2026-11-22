//! The prefilter's per-tape access paths (docs/final-shape/03-architecture.md §6, §16; 05 Phase 3),
//! adapted from PIAT's anchor store (`matcher/anchors/sql/{store.rs, values.rs,
//! store/kinds.rs}`), which followed the SQL index prototype's `tape_store.py`.
//!
//! A tape's rows are kept facts of the store (t33 steps 6 and 7, `kept`): one partition a tape, its `tape_id`,
//! gone in the transaction that deletes the tape's row. Kinds are atoms (`tape tag · 1024 + kind code`). A tape's
//! set row carries the version the rows were built under; publishing again replaces them. The SQL route reads them
//! through virtual tables of the names the store's tables had (`vtab`).
//!
//! - kinds: per (tape, kind), the count and the preorders as a container segment: the access path for "which nodes
//!   of this tape have kind k", without decoding the tape.
//! - terms: case-exact leaf text per (tape, term, kind). A term is its text's atom (`atoms`), interned in the
//!   publication's transaction, so its text is stored once.
//! - coverage: per (tape, kind), whether every node of the kind is covered by a term (`bad` = 0), and the nodes that
//!   are not (`missing`). Unclean leaves (whitespace, control characters, too long) are kept as terms only while the
//!   store-wide residue of their kind stays under `RESIDUE_CAP`.
//! - the store-wide counts (`kind_size`, `term_kind_freq`, `term_node_freq`, `term_freq`): runs of deltas (`kept`).

pub mod kept;
pub mod vocabulary;
pub(in crate::content_store) mod vtab;

use crate::content_store::connection::Cached as _;
use super::bounded_insert::insert;
use super::kept_facts::{ChainRows, KeptSpace, chain_rows};
use super::postings::append_delta;
use super::{ContentStore, MAX_PUBLICATION_BATCH, StoreError, StoreResult, TapeId};
use rusqlite::{Connection, types::Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// The image a tape's rows were built by. 2: ERROR nodes are kind code 0 in both postings.
/// 3: term postings carry their node count, which the planner statistics sum. 4: kind sizes and
/// edges (`kind_size`, `edge_freq`). 5: terms are atoms. 6: kind postings are container segments
/// (`postings::segment_positions`, decision 0045). 9: kept facts (`kept`), with no edges.
pub const PREFILTER_VERSION: i64 = 9;
/// Unclean leaves per kind, store-wide, kept as terms.
pub const RESIDUE_CAP: usize = 4096;
/// A node of the kind has a named descendant: its text is not a leaf term.
pub const INTERIOR: u8 = 1;
/// A leaf of the kind is not a clean term.
pub const UNCLEAN: u8 = 2;
/// The kind's unclean leaves exceed the residue bounds.
pub const TOO_BIG: u8 = 4;

/// One tape's prefilter rows, built from the tape alone (`code_rule_dsl::prefilter::image`), so
/// publication holds the write lock only to write.
#[derive(Clone, Debug)]
pub struct TapePrefilter {
    pub tape_id: TapeId,
    pub lang: u32,
    /// The tape's node count: candidate SQL enumerates every node of a tape from it.
    pub nodes: u32,
    /// (kind atom, count, sum of descendants, delta-varint postings), sorted by atom.
    pub kinds: Vec<(u32, u32, u64, Vec<u8>)>,
    /// Clean leaf terms: (kind atom, term) → preorders, ascending.
    pub postings: BTreeMap<(u32, String), Vec<u32>>,
    /// Per kind atom: the coverage flags and the number of unclean leaves.
    pub cover: BTreeMap<u32, (u8, usize)>,
    /// Per kind atom: the nodes no term covers, ascending.
    pub missing: BTreeMap<u32, Vec<u32>>,
    /// Per kind atom: unclean leaves, which become terms unless the store-wide cap refuses them.
    pub residue: BTreeMap<u32, Vec<(u32, String)>>,
}

/// Terms whose publication fails in a store (`ContentStore::reject_prefilter_term_for_test`): a failed publication's
/// rollback, tested.
#[cfg(any(test, feature = "test-hooks"))]
static REJECT_TERM: std::sync::Mutex<Vec<(std::path::PathBuf, String)>> = std::sync::Mutex::new(Vec::new());

impl ContentStore {
    /// Makes every later publication of prefilter rows with term `term` into this store fail, inside its transaction.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn reject_prefilter_term_for_test(&self, term: &str) {
        REJECT_TERM.lock().expect("reject hook").push((self.database_path.clone(), term.to_owned()));
    }

    /// Publishes each tape's prefilter rows, replacing any it has (a tape given twice: its last), and counts
    /// them into the store-wide counts, in one transaction.
    pub fn put_prefilters_bulk(&self, sets: &[TapePrefilter]) -> StoreResult<()> {
        if sets.len() > MAX_PUBLICATION_BATCH {
            return Err(StoreError::Invalid("prefilter publication exceeds 64 tapes".to_owned()));
        }
        self.require_write_capacity()?;
        let mut last: BTreeMap<i64, &TapePrefilter> = BTreeMap::new();
        for set in sets {
            last.insert(set.tape_id.row(), set);
        }
        let path = self.database_path.clone();
        // Its terms are the store's ids: written after a fork too (decision 0170).
        let written = super::atoms::mapped(|| self.immediate_transaction(|t| {
            let space = kept::space();
            let old = open(&path, t, &space)?;
            let view = old.as_ref().map(KeptSpace::view);
            let sums = view.as_ref().map(|v| kept::sums(v));
            // Each term's atom, from the store's arena (decision 0049), as the store numbers it: new ones are
            // pending, and the transaction's commit appends them before these rows (`ContentStore::commit`).
            let texts: BTreeSet<&str> = last
                .values()
                .flat_map(|s| s.postings.keys().map(|(_, t)| t.as_str()).chain(s.residue.values().flatten().map(|(_, t)| t.as_str())))
                .collect();
            let texts: Vec<&str> = texts.into_iter().collect();
            let mut ids: Vec<i64> = self.atoms.intern_all(&texts)?.into_iter().map(i64::from).collect();
            self.atoms.to_store(t, super::ProcessIds::of_process(&mut ids))?;
            let atoms: HashMap<&str, i64> = texts.iter().copied().zip(ids).collect();
            #[cfg(any(test, feature = "test-hooks"))]
            if REJECT_TERM.lock().expect("reject hook").iter().any(|(store, term)| *store == path && atoms.contains_key(term.as_str())) {
                return Err(StoreError::Invalid("publication failed: a rejected term (test hook)".to_owned()));
            }
            let mut change = kept::Change::new();
            for (&tape, set) in &last {
                change.replace(view.as_ref(), tape);
                let prior = |kind: i64| sums.as_ref().map_or(0, |s| s.kind(kind).residue) + change.delta.residue(kind);
                let rows = tape_rows(set, &atoms, prior)?;
                change.put(tape, &rows)?;
            }
            drop(sums);
            drop(view);
            change.write(&path, t, old.as_ref())
        }))?;
        self.note_kept(&kept::space(), written);
        self.hold_prefilters(sets);
        Ok(())
    }

    /// The kind images of `sets`, tape-local, held in the working set for the wave that reads
    /// them (written through by `put_prefilters_bulk`; ahead of it by a write-back queue).
    pub fn hold_prefilters(&self, sets: &[TapePrefilter]) {
        for set in sets {
            self.working_set().put_kinds(
                set.tape_id.row(),
                super::working_set::KindImage {
                    lang: set.lang,
                    nodes: set.nodes,
                    kinds: set.kinds.iter().map(|(kind, _, _, postings)| (*kind, postings.clone())).collect(),
                },
            );
        }
    }

    /// Removes a tape's prefilter rows, counted out (they also go when the tape is collected).
    pub fn remove_prefilter(&self, tape: TapeId) -> StoreResult<()> {
        self.working_set().forget_kinds(tape.row());
        let path = self.database_path.clone();
        let written = self.immediate_transaction(|t| forget_tapes(&path, t, &[tape.row()]))?;
        self.note_kept(&kept::space(), written);
        Ok(())
    }

    /// The version of a tape's prefilter rows, if it has them.
    pub fn prefilter_version(&self, tape: TapeId) -> StoreResult<Option<i64>> {
        let Some(space) = self.kept_space(&kept::space())? else { return Ok(None) };
        Ok(kept::tape(&space.view(), tape.row()).map(|t| t.version))
    }

    /// The preorders of `tape`'s nodes of kind atom `kind`, decoded from their container segment.
    pub fn kind_positions(&self, tape: TapeId, kind: u32) -> StoreResult<Vec<u32>> {
        let Some(space) = self.kept_space(&kept::space())? else { return Ok(Vec::new()) };
        let view = space.view();
        let Some(t) = kept::tape(&view, tape.row()) else { return Ok(Vec::new()) };
        let Some(k) = t.kind(i64::from(kind)) else { return Ok(Vec::new()) };
        super::postings::segment_positions(&t.bytes(k.at, k.len)).map_err(|e| StoreError::Corrupt(format!("a kind's postings: {e}")))
    }

    /// Runs the prefilter's candidate SQL (`code_rule_dsl::prefilter::execute`) on the store's
    /// connection in one transaction. Its sets are temporary tables; only registering regex
    /// predicates writes the store (the `vocabulary_*` tables), so a statement that does takes the
    /// write lock up front and any other reads a snapshot without waiting for a writer.
    pub fn candidate_transaction<T>(
        &self,
        writes: bool,
        mut operation: impl FnMut(&rusqlite::Connection) -> rusqlite::Result<T>,
    ) -> StoreResult<T> {
        // The candidate statement's page cache, for its span only: the store's connections keep
        // 1 MB each (`configure_connection`; decision 0024 §6).
        const CANDIDATE_CACHE_KB: i64 = 32 * 1024;
        let cache = |kb: i64| -> StoreResult<()> {
            self.connection.lock().expect("content store mutex poisoned").pragma_update(None, "cache_size", -kb)?;
            Ok(())
        };
        cache(CANDIDATE_CACHE_KB)?;
        // Its statements read and write terms as the store numbers them (the `atoms` table answers in the store's ids,
        // decision 0170), never this process's: written after a fork too.
        let value = super::atoms::mapped(|| {
            if writes {
                self.require_write_capacity()?;
                return self.immediate_transaction(|transaction| Ok(operation(transaction)?));
            }
            let mut connection = self.connection.lock().expect("content store mutex poisoned");
            let transaction = super::connection::begin(&mut connection, rusqlite::TransactionBehavior::Deferred)?;
            let value = operation(&transaction)?;
            self.commit(transaction)?;
            Ok(value)
        });
        cache(1_024)?;
        value
    }

    /// Sets the progress handler of this thread's store connection, which `candidate_transaction` runs on
    /// (`None`: removes it). The anchor lab counts a candidate statement's VM steps with it.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn candidate_progress<F>(&self, steps: i32, handler: Option<F>) -> StoreResult<()>
    where
        F: FnMut() -> bool + Send + 'static,
    {
        self.connection.lock().expect("content store mutex poisoned").progress_handler(steps, handler)?;
        Ok(())
    }

    /// Copies the prefilter rows of `tapes`, the store-wide counts and the regex vocabulary into a
    /// new store at `path`, for the anchor lab (`code_rule_dsl::prefilter::lab`). Tapes keep their
    /// identity and grammar but not their bytes, which candidate SQL never reads.
    pub fn capture_prefilter(&self, path: &std::path::Path, tapes: &[i64]) -> StoreResult<()> {
        drop(ContentStore::open(path)?);
        // The capture copies the atoms' segments: the pending ones are written first.
        if self.atoms.pending() {
            self.immediate_transaction(|_| Ok(()))?;
        }
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let target = path.to_str().ok_or_else(|| StoreError::Invalid("capture path is not UTF-8".to_owned()))?;
        connection.execute_cached("ATTACH DATABASE ?1 AS capture", [target])?;
        // The copies keep their keys, not their parents (sources, hashes), so the capture is
        // written with foreign keys off; that pragma is per connection and outside transactions.
        connection.execute_batch("PRAGMA foreign_keys = OFF")?;
        let copied = (|| -> rusqlite::Result<()> {
            connection.execute_batch(
                "BEGIN; CREATE TEMP TABLE capture_tapes(tape_id INTEGER PRIMARY KEY);",
            )?;
            insert(&connection, "INSERT OR IGNORE INTO temp.capture_tapes(tape_id)", "", tapes.iter().map(|&tape| [Value::Integer(tape)]))?;
            // `RERAIL_CAPTURE_TAPES=1` keeps the tapes' frames, for the SQL fallback's lab
            // (`RERAIL_LAB_FALLBACK`), whose relations read them through `nera_tree`.
            if std::env::var_os("RERAIL_CAPTURE_TAPES").is_some() {
                connection.execute_batch(
                    "INSERT INTO capture.tape_objects SELECT t.* FROM temp.capture_tapes c JOIN main.tape_objects t USING(tape_id); \
                     INSERT INTO capture.object_segments SELECT * FROM main.object_segments;",
                )?;
                // The segments the tapes are in (decision 0295 step 5), beside the capture as beside the store.
                let (from, to) = (super::object_segments::object_dir(&self.database_path), super::object_segments::object_dir(path));
                if let Ok(entries) = std::fs::read_dir(&from) {
                    let io = |e: std::io::Error| rusqlite::Error::ToSqlConversionFailure(Box::new(e));
                    std::fs::create_dir_all(&to).map_err(io)?;
                    for e in entries.flatten() {
                        std::fs::copy(e.path(), to.join(e.file_name())).map_err(io)?;
                    }
                }
            } else {
                connection.execute_batch(
                    "INSERT INTO capture.tape_objects(tape_id, source_id, canonical_hash_id, grammar_id, grammar_version, \
                       tape_format_version, codec_id, raw_length, frame, created_generation, seg, seg_at, seg_len, programs) \
                     SELECT t.tape_id, t.source_id, t.canonical_hash_id, t.grammar_id, \
                       t.grammar_version, t.tape_format_version, t.codec_id, 0, x'', t.created_generation, -1, 0, 0, t.programs \
                       FROM temp.capture_tapes c JOIN main.tape_objects t USING(tape_id);",
                )?;
            }
            // The prefilter's and the vocabulary's kept facts whole (every tape's rows, the counts, the keys' terms):
            // their chains' rows, and their files beside the capture as beside the store.
            let spaces = [kept::space(), vocabulary::space()];
            for space in &spaces {
                connection.execute("INSERT INTO capture.facts_chain SELECT * FROM main.facts_chain WHERE space=?1", [&space[..]])?;
            }
            connection.execute_batch(
                "INSERT INTO capture.atom_blocks SELECT * FROM main.atom_blocks;
                 DROP TABLE temp.capture_tapes;
                 COMMIT;",
            )?;
            let io = |e: std::io::Error| rusqlite::Error::ToSqlConversionFailure(Box::new(e));
            for space in &spaces {
                let (from, to) = (super::kept_facts::space_dir(&self.database_path, space), super::kept_facts::space_dir(path, space));
                if let Ok(entries) = std::fs::read_dir(&from) {
                    std::fs::create_dir_all(&to).map_err(io)?;
                    for e in entries.flatten().filter(|e| !e.file_name().to_string_lossy().ends_with(".new")) {
                        std::fs::copy(e.path(), to.join(e.file_name())).map_err(io)?;
                    }
                }
            }
            // The atoms' blocks (decision 0401), beside the capture as beside the store: what the rows copied name.
            super::atoms::copy_blocks(&self.database_path, path).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            Ok(())
        })();
        if copied.is_err() && !connection.is_autocommit() {
            connection.execute_batch("ROLLBACK")?;
        }
        connection.execute_batch("PRAGMA foreign_keys = ON")?;
        connection.execute_batch("DETACH DATABASE capture")?;
        Ok(copied?)
    }

    /// Deletes a tape, as collection does (its prefilter rows counted out and gone with it, and its search index), for tests of the
    /// collection.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn delete_tape_for_test(&self, tape: TapeId) -> StoreResult<()> {
        self.working_set().clear();
        let path = self.database_path.clone();
        let (written, search) = self.immediate_transaction(|transaction| {
            transaction.execute("DELETE FROM tape_objects WHERE tape_id=?1", [tape.row()])?;
            Ok((forget_tapes(&path, transaction, &[tape.row()])?, super::search_index::forget_tapes(&path, transaction, &[tape.row()])?))
        })?;
        self.note_kept(&kept::space(), written);
        self.note_kept(&super::search_index::space(), search);
        Ok(())
    }

    /// Rows of a read-only query over the store, for tests of the prefilter's SQL.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn query_rows<T>(
        &self,
        sql: &str,
        values: impl rusqlite::Params,
        row: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    ) -> StoreResult<Vec<T>> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let mut statement = connection.prepare(sql)?;
        let rows = statement.query_map(values, row)?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

/// The prefilter's space as transaction `t` of the store at `store` finds it.
fn open(store: &std::path::Path, t: &Connection, space: &[u8; 32]) -> StoreResult<Option<KeptSpace>> {
    let rows = chain_rows(t, space)?;
    if rows.is_empty() { Ok(None) } else { KeptSpace::open(store, space, &rows).map(Some) }
}

/// Tapes `tapes`' prefilter rows counted out and gone, in transaction `t` of the store at `store`, which deletes
/// their rows (their ids may be given again). The chain's rows once written, for `ContentStore::note_kept` after
/// the commit.
pub(in crate::content_store) fn forget_tapes(store: &std::path::Path, t: &Connection, tapes: &[i64]) -> StoreResult<Option<ChainRows>> {
    if tapes.is_empty() {
        return Ok(None);
    }
    let space = kept::space();
    let Some(old) = open(store, t, &space)? else { return Ok(None) };
    let view = old.view();
    let mut change = kept::Change::new();
    for &tape in tapes {
        change.remove(Some(&view), tape);
    }
    change.write(store, t, Some(&old))
}

/// `set`'s rows as kept: its terms the store's atoms (`atoms`), and a kind's unclean leaves terms only while
/// `prior(kind)`, the store-wide residue the counts and the publication so far hold, and the tape's stay under
/// `RESIDUE_CAP` (a kind past it keeps them on the coverage path instead).
fn tape_rows(set: &TapePrefilter, atoms: &HashMap<&str, i64>, prior: impl Fn(i64) -> i64) -> StoreResult<kept::TapeRows> {
    let atom = |term: &str| atoms.get(term).copied().ok_or_else(|| StoreError::Invalid("a posting's term has no atom".to_owned()));
    let mut cover = set.cover.clone();
    let mut accepted = BTreeSet::new();
    for &kind in set.residue.keys() {
        let coverage = cover.get_mut(&kind).ok_or_else(|| StoreError::Invalid("residue without coverage".to_owned()))?;
        let before = prior(i64::from(kind));
        if usize::try_from(before).unwrap_or(usize::MAX).saturating_add(coverage.1) > RESIDUE_CAP {
            coverage.0 |= TOO_BIG;
        }
        if coverage.0 & (INTERIOR | TOO_BIG) == 0 {
            accepted.insert(kind);
        }
    }
    let mut postings: BTreeMap<(u32, &str), Vec<u32>> = set.postings.iter().map(|((kind, term), pres)| ((*kind, term.as_str()), pres.clone())).collect();
    for kind in &accepted {
        for (pre, term) in &set.residue[kind] {
            postings.entry((*kind, term.as_str())).or_default().push(*pre);
        }
    }
    let mut terms = Vec::with_capacity(postings.len());
    for ((kind, term), mut pres) in postings {
        pres.sort_unstable();
        terms.push((atom(term)?, i64::from(kind), pres.len() as i64, pack(&pres)));
    }
    terms.sort_unstable_by_key(|t| (t.0, t.1));
    let covers = cover
        .iter()
        .map(|(&kind, &(bad, residue))| {
            let missing = if accepted.contains(&kind) { &[][..] } else { set.missing.get(&kind).map_or(&[][..], Vec::as_slice) };
            (i64::from(kind), i64::from(bad), residue as i64, pack(missing))
        })
        .collect();
    Ok(kept::TapeRows {
        version: PREFILTER_VERSION,
        nodes: i64::from(set.nodes),
        lang: i64::from(set.lang),
        kinds: set.kinds.iter().map(|(kind, n, size, positions)| (i64::from(*kind), i64::from(*n), *size as i64, positions.clone())).collect(),
        terms,
        covers,
    })
}

/// Ascending preorders as a delta-varint segment.
pub fn pack(pres: &[u32]) -> Vec<u8> {
    let mut packed = Vec::new();
    let mut previous = 0;
    for &pre in pres {
        append_delta(&mut packed, pre - previous);
        previous = pre;
    }
    packed
}
