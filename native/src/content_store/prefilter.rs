//! The prefilter's per-tape access paths (docs/final-shape/03-architecture.md §6, §16; 05 Phase 3),
//! adapted from Pulling-it-all-together's anchor store (`matcher/anchors/sql/{store.rs, values.rs,
//! store/kinds.rs}`), which followed ast-index-spike's `tape_store.py`.
//!
//! P kept a private database with its own `file` table holding a second copy of each tape. Here
//! the rows are keyed by the content store's `tape_objects.tape_id` and cascade from it, so a
//! collected tape takes its rows with it and two workspaces holding the same content share them.
//! Kinds are atoms (`tape tag · 1024 + kind code`). A set row (`prefilter_sets`) carries the
//! version the rows were built under; publishing again replaces them.
//!
//! - `tape_kind_counts`: per (tape, kind), the count and the preorders as a container segment: the
//!   access path for "which nodes of this tape have kind k", without decoding the tape.
//! - `term_postings`: case-exact leaf text per (lang, term, tape, kind), and `term_freq` the number
//!   of (tape, kind) rows carrying each term. A term is its text's atom (`atoms`), interned in the
//!   publication's transaction, so its text is stored once.
//! - `term_coverage`: per (tape, kind), whether every node of the kind is covered by a term
//!   (`bad` = 0), and the nodes that are not (`missing`). Unclean leaves (whitespace, control
//!   characters, too long) are kept as terms only while the store-wide residue of their kind stays
//!   under `RESIDUE_CAP`.

use super::bounded_insert::insert;
use super::postings::{append_delta, pack_edges};
use super::{ContentStore, MAX_PUBLICATION_BATCH, StoreError, StoreResult, TapeId};
use rusqlite::{Transaction, params, types::Value};
use std::collections::{BTreeMap, BTreeSet};

/// The image a tape's rows were built by. 2: ERROR nodes are kind code 0 in both postings.
/// 3: term postings carry their node count, which the planner statistics sum. 4: kind sizes and
/// edges (`kind_size`, `edge_freq`). 5: terms are atoms. 6: kind postings are container segments
/// (`postings::segment_positions`, decision 0045).
pub(crate) const PREFILTER_VERSION: i64 = 8;
/// Unclean leaves per kind, store-wide, kept as terms.
pub(crate) const RESIDUE_CAP: usize = 4096;
/// A node of the kind has a named descendant: its text is not a leaf term.
pub(crate) const INTERIOR: u8 = 1;
/// A leaf of the kind is not a clean term.
pub(crate) const UNCLEAN: u8 = 2;
/// The kind's unclean leaves exceed the residue bounds.
pub(crate) const TOO_BIG: u8 = 4;

/// One tape's prefilter rows, built from the tape alone (`code_rule_dsl::prefilter::image`), so
/// publication holds the write lock only to write.
#[derive(Clone, Debug)]
pub(crate) struct TapePrefilter {
    pub tape_id: TapeId,
    pub lang: u32,
    /// The tape's node count: candidate SQL enumerates every node of a tape from it.
    pub nodes: u32,
    /// (kind atom, count, sum of descendants, delta-varint postings), sorted by atom.
    pub kinds: Vec<(u32, u32, u64, Vec<u8>)>,
    /// Direct edges: (parent kind atom, child kind atom, field, count), sorted.
    pub edges: Vec<(u32, u32, u16, u32)>,
    /// Clean leaf terms: (kind atom, term) → preorders, ascending.
    pub postings: BTreeMap<(u32, String), Vec<u32>>,
    /// Per kind atom: the coverage flags and the number of unclean leaves.
    pub cover: BTreeMap<u32, (u8, usize)>,
    /// Per kind atom: the nodes no term covers, ascending.
    pub missing: BTreeMap<u32, Vec<u32>>,
    /// Per kind atom: unclean leaves, which become terms unless the store-wide cap refuses them.
    pub residue: BTreeMap<u32, Vec<(u32, String)>>,
}

impl ContentStore {
    /// Publishes each tape's prefilter rows, replacing any it has.
    pub(crate) fn put_prefilters_bulk(&self, sets: &[TapePrefilter]) -> StoreResult<()> {
        if sets.len() > MAX_PUBLICATION_BATCH {
            return Err(StoreError::Invalid("prefilter publication exceeds 64 tapes".to_owned()));
        }
        self.require_write_capacity()?;
        self.immediate_transaction(|transaction| {
            let mut counts = Counts::default();
            for set in sets {
                put(transaction, set, &mut counts, &self.atoms)?;
            }
            counts.flush(transaction, &self.atoms)
        })?;
        self.hold_prefilters(sets);
        Ok(())
    }

    /// The kind images of `sets`, tape-local, held in the working set for the wave that reads
    /// them (written through by `put_prefilters_bulk`; ahead of it by a write-back queue).
    pub(crate) fn hold_prefilters(&self, sets: &[TapePrefilter]) {
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

    /// Removes a tape's prefilter rows (they also go when the tape is collected).
    pub(crate) fn remove_prefilter(&self, tape: TapeId) -> StoreResult<()> {
        self.working_set().forget_kinds(tape.row());
        self.immediate_transaction(|transaction| {
            transaction.execute("DELETE FROM prefilter_sets WHERE tape_id=?1", [tape.row()])?;
            Ok(())
        })
    }

    /// The version of a tape's prefilter rows, if it has them.
    pub(crate) fn prefilter_version(&self, tape: TapeId) -> StoreResult<Option<i64>> {
        use rusqlite::OptionalExtension;
        let connection = self.connection.lock().expect("content store mutex poisoned");
        Ok(connection
            .prepare_cached("SELECT version FROM prefilter_sets WHERE tape_id=?1")?
            .query_row([tape.row()], |row| row.get(0))
            .optional()?)
    }

    /// The preorders of `tape`'s nodes of kind atom `kind`, decoded in SQL (`nera_segment`).
    pub(crate) fn kind_positions(&self, tape: TapeId, kind: u32) -> StoreResult<Vec<u32>> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let mut statement = connection.prepare_cached(
            "SELECT p.pre FROM tape_kind_counts c, nera_segment(c.postings) p \
             WHERE c.tape_id=?1 AND c.kind=?2",
        )?;
        let rows = statement
            .query_map(params![tape.row(), kind], |row| row.get(0))?
            .collect::<Result<Vec<u32>, _>>()?;
        Ok(rows)
    }

    /// Runs the prefilter's candidate SQL (`code_rule_dsl::prefilter::execute`) on the store's
    /// connection in one transaction. Its sets are temporary tables; only registering regex
    /// predicates writes the store (the `vocabulary_*` tables), so a statement that does takes the
    /// write lock up front and any other reads a snapshot without waiting for a writer.
    pub(crate) fn candidate_transaction<T>(
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
        let value = (|| {
            if writes {
                self.require_write_capacity()?;
                return self.immediate_transaction(|transaction| Ok(operation(transaction)?));
            }
            let mut connection = self.connection.lock().expect("content store mutex poisoned");
            let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)?;
            let value = operation(&transaction)?;
            self.commit(transaction)?;
            Ok(value)
        })();
        cache(1_024)?;
        value
    }

    /// Copies the prefilter rows of `tapes`, the store-wide counts and the regex vocabulary into a
    /// new store at `path`, for the anchor lab (`code_rule_dsl::prefilter::lab`). Tapes keep their
    /// identity and grammar but not their bytes, which candidate SQL never reads.
    pub(crate) fn capture_prefilter(&self, path: &std::path::Path, tapes: &[i64]) -> StoreResult<()> {
        drop(ContentStore::open(path)?);
        // The capture copies the atoms' segments: the pending ones are written first.
        if self.atoms.pending() {
            self.immediate_transaction(|_| Ok(()))?;
        }
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let target = path.to_str().ok_or_else(|| StoreError::Invalid("capture path is not UTF-8".to_owned()))?;
        connection.execute("ATTACH DATABASE ?1 AS capture", [target])?;
        // The copies keep their keys, not their parents (sources, hashes), so the capture is
        // written with foreign keys off; that pragma is per connection and outside transactions.
        connection.execute_batch("PRAGMA foreign_keys = OFF")?;
        let copied = (|| -> rusqlite::Result<()> {
            connection.execute_batch(
                "BEGIN; CREATE TEMP TABLE capture_tapes(tape_id INTEGER PRIMARY KEY);",
            )?;
            insert(&connection, "INSERT OR IGNORE INTO temp.capture_tapes(tape_id)", "", tapes.iter().map(|&tape| [Value::Integer(tape)]))?;
            // `term_freq` before the vocabulary, so its insert trigger finds no predicate to mark.
            // `RERAIL_CAPTURE_TAPES=1` keeps the tapes' frames, for the SQL fallback's lab
            // (`RERAIL_LAB_FALLBACK`), whose relations read them through `nera_tree`.
            if std::env::var_os("RERAIL_CAPTURE_TAPES").is_some() {
                connection.execute_batch(
                    "INSERT INTO capture.tape_objects SELECT t.* FROM temp.capture_tapes c JOIN main.tape_objects t USING(tape_id);",
                )?;
            } else {
                connection.execute_batch(
                    "INSERT INTO capture.tape_objects SELECT t.tape_id, t.source_id, t.canonical_hash_id, t.grammar_id, \
                       t.grammar_version, t.tape_format_version, t.codec_id, 0, x'', t.created_generation \
                       FROM temp.capture_tapes c JOIN main.tape_objects t USING(tape_id);",
                )?;
            }
            connection.execute_batch(
                "
                 INSERT INTO capture.prefilter_sets SELECT s.* FROM temp.capture_tapes c JOIN main.prefilter_sets s USING(tape_id);
                 INSERT INTO capture.tape_kind_counts SELECT k.* FROM temp.capture_tapes c JOIN main.tape_kind_counts k USING(tape_id);
                 INSERT INTO capture.term_postings SELECT p.* FROM temp.capture_tapes c JOIN main.term_postings p USING(tape_id);
                 INSERT INTO capture.term_coverage SELECT v.* FROM temp.capture_tapes c JOIN main.term_coverage v USING(tape_id);
                 INSERT OR IGNORE INTO capture.atom_segment SELECT * FROM main.atom_segment;
                 INSERT INTO capture.kind_size SELECT * FROM main.kind_size;
                 INSERT INTO capture.edge_freq SELECT * FROM main.edge_freq;
                 INSERT INTO capture.term_freq SELECT * FROM main.term_freq;
                 INSERT INTO capture.term_node_freq SELECT * FROM main.term_node_freq;
                 INSERT INTO capture.term_kind_freq SELECT * FROM main.term_kind_freq;
                 DELETE FROM capture.vocabulary_pending;
                 INSERT INTO capture.vocabulary_predicate SELECT * FROM main.vocabulary_predicate;
                 INSERT INTO capture.vocabulary_term SELECT * FROM main.vocabulary_term;
                 INSERT INTO capture.vocabulary_literal SELECT * FROM main.vocabulary_literal;
                 INSERT INTO capture.vocabulary_pending SELECT * FROM main.vocabulary_pending;
                 DROP TABLE temp.capture_tapes;
                 COMMIT;",
            )
        })();
        if copied.is_err() && !connection.is_autocommit() {
            connection.execute_batch("ROLLBACK")?;
        }
        connection.execute_batch("PRAGMA foreign_keys = ON")?;
        connection.execute_batch("DETACH DATABASE capture")?;
        Ok(copied?)
    }

    /// Deletes a tape, as collection does, for tests of the cascade.
    #[cfg(test)]
    pub(crate) fn delete_tape_for_test(&self, tape: TapeId) -> StoreResult<()> {
        self.working_set().clear();
        self.immediate_transaction(|transaction| {
            transaction.execute("DELETE FROM tape_objects WHERE tape_id=?1", [tape.row()])?;
            Ok(())
        })
    }

    /// Rows of a read-only query over the store, for tests of the prefilter's SQL.
    #[cfg(test)]
    pub(crate) fn query_rows<T>(
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

/// A publication's store-wide counts and postings, summed over its tapes and written once a key
/// (06 S7: one upsert per key per tape was ~150 of the push's ~300 ms of publication).
#[derive(Default)]
struct Counts {
    kind_size: BTreeMap<u32, (i64, i64)>,
    edge_freq: BTreeMap<(i64, i64, i64), i64>,
    postings: Vec<(i64, String, i64, i64, i64, Vec<u8>)>,
    term_node_freq: BTreeMap<(i64, String, i64), i64>,
    term_kind_freq: BTreeMap<u32, i64>,
    term_freq: BTreeMap<(i64, String), i64>,
}

impl Counts {
    fn is_empty(&self) -> bool {
        self.kind_size.is_empty() && self.edge_freq.is_empty() && self.postings.is_empty()
    }

    /// Writes and clears the sums; postings in key order, so consecutive rows share leaves.
    fn flush(&mut self, transaction: &Transaction<'_>, store: &super::atoms::Atoms) -> StoreResult<()> {
        let counts = std::mem::take(self);
        // Each term's atom, from the store's arena (decision 0049): new ones are pending, and the
        // transaction's commit appends them before these rows (`ContentStore::commit`).
        let texts = counts.term_freq.keys().map(|(_, term)| term.as_str()).collect::<Vec<_>>();
        let ids = store.intern_all(&texts)?;
        let atoms: BTreeMap<&str, i64> = texts.iter().copied().zip(ids.into_iter().map(i64::from)).collect();
        let atom = |term: &str| atoms.get(term).copied().ok_or_else(|| StoreError::Invalid("a posting's term has no atom".to_owned()));
        insert(
            transaction,
            "INSERT INTO kind_size(kind,n,size)",
            "ON CONFLICT(kind) DO UPDATE SET n=n+excluded.n,size=size+excluded.size",
            counts.kind_size.into_iter().map(|(kind, (n, size))| [i64::from(kind).into(), n.into(), size.into()]),
        )?;
        insert(
            transaction,
            "INSERT INTO edge_freq(pkind,ckind,field,n)",
            "ON CONFLICT(pkind,ckind,field) DO UPDATE SET n=n+excluded.n",
            counts.edge_freq.into_iter().map(|((parent, child, field), n)| [parent.into(), child.into(), field.into(), n.into()]),
        )?;
        let mut postings = counts.postings;
        postings.sort_unstable_by(|a, b| (a.0, &a.1, a.2, a.3).cmp(&(b.0, &b.1, b.2, b.3)));
        insert(
            transaction,
            "INSERT INTO term_postings(lang,term,tape_id,kind,n,postings)",
            "",
            postings
                .into_iter()
                .map(|(lang, term, tape, kind, n, packed)| Ok([lang.into(), atom(&term)?.into(), tape.into(), kind.into(), n.into(), Value::Blob(packed)]))
                .collect::<StoreResult<Vec<_>>>()?,
        )?;
        insert(
            transaction,
            "INSERT INTO term_node_freq(lang,term,kind,n)",
            "ON CONFLICT(lang,term,kind) DO UPDATE SET n=n+excluded.n",
            counts
                .term_node_freq
                .into_iter()
                .map(|((lang, term, kind), n)| Ok([lang.into(), atom(&term)?.into(), kind.into(), n.into()]))
                .collect::<StoreResult<Vec<_>>>()?,
        )?;
        insert(
            transaction,
            "INSERT INTO term_kind_freq(kind,n)",
            "ON CONFLICT(kind) DO UPDATE SET n=n+excluded.n",
            counts.term_kind_freq.into_iter().map(|(kind, n)| [i64::from(kind).into(), n.into()]),
        )?;
        insert(
            transaction,
            "INSERT INTO term_freq(lang,term,n)",
            "ON CONFLICT(lang,term) DO UPDATE SET n=n+excluded.n",
            counts
                .term_freq
                .iter()
                .map(|((lang, term), n)| Ok([(*lang).into(), atom(term)?.into(), (*n).into()]))
                .collect::<StoreResult<Vec<_>>>()?,
        )?;
        Ok(())
    }
}

fn put(transaction: &Transaction<'_>, set: &TapePrefilter, counts: &mut Counts, atoms: &super::atoms::Atoms) -> StoreResult<()> {
    let tape = set.tape_id.row();
    // Publishing again replaces: the cascade and triggers count the old rows down, so the sums
    // so far are written first, as they would have been one tape at a time.
    if !counts.is_empty() {
        let published: bool = transaction
            .prepare_cached("SELECT EXISTS(SELECT 1 FROM prefilter_sets WHERE tape_id=?1)")?
            .query_row([tape], |row| row.get(0))?;
        if published {
            counts.flush(transaction, atoms)?;
        }
    }
    transaction.execute("DELETE FROM prefilter_sets WHERE tape_id=?1", [tape])?;
    transaction.execute(
        "INSERT INTO prefilter_sets(tape_id,version,nodes,edges) VALUES(?1,?2,?3,?4)",
        params![tape, PREFILTER_VERSION, set.nodes, pack_edges(&set.edges)],
    )?;
    // The residue cap applies to the store, not the tape: a kind whose unclean leaves, with the
    // ones already published, exceed it keeps them on the coverage path instead.
    let mut cover = set.cover.clone();
    let mut accepted = BTreeSet::new();
    {
        let mut prior = transaction.prepare_cached(
            "SELECT coalesce(sum(residue),0) FROM term_coverage WHERE kind=?1 AND residue>0",
        )?;
        for &kind in set.residue.keys() {
            let coverage = cover.get_mut(&kind).ok_or_else(|| StoreError::Invalid("residue without coverage".to_owned()))?;
            let before: i64 = prior.query_row([kind], |row| row.get(0))?;
            if usize::try_from(before).unwrap_or(usize::MAX).saturating_add(coverage.1) > RESIDUE_CAP {
                coverage.0 |= TOO_BIG;
            }
            if coverage.0 & (INTERIOR | TOO_BIG) == 0 {
                accepted.insert(kind);
            }
        }
    }
    let mut postings = set.postings.iter().map(|((kind, term), pres)| ((*kind, term.as_str()), pres.clone())).collect::<BTreeMap<_, _>>();
    for kind in &accepted {
        for (pre, term) in &set.residue[kind] {
            postings.entry((*kind, term.as_str())).or_default().push(*pre);
        }
    }
    let lang = i64::from(set.lang);
    insert(
        transaction,
        "INSERT INTO tape_kind_counts(tape_id,kind,n,size,postings)",
        "",
        set.kinds.iter().map(|(kind, n, size, positions)| {
            [tape.into(), i64::from(*kind).into(), i64::from(*n).into(), (*size as i64).into(), Value::Blob(positions.clone())]
        }),
    )?;
    for (kind, n, size, _) in &set.kinds {
        let entry = counts.kind_size.entry(*kind).or_default();
        entry.0 += i64::from(*n);
        entry.1 += *size as i64;
    }
    for &(parent, child, field, n) in &set.edges {
        *counts.edge_freq.entry((i64::from(parent), i64::from(child), i64::from(field))).or_default() += i64::from(n);
    }
    for ((kind, term), pres) in &mut postings {
        pres.sort_unstable();
        counts.postings.push((lang, (*term).to_owned(), tape, i64::from(*kind), pres.len() as i64, pack(pres)));
        *counts.term_node_freq.entry((lang, (*term).to_owned(), i64::from(*kind))).or_default() += pres.len() as i64;
        *counts.term_kind_freq.entry(*kind).or_default() += pres.len() as i64;
    }
    // One row per term, counting the tape's kinds that carry it, in place of a per-row trigger.
    for (_, term) in postings.keys() {
        *counts.term_freq.entry((lang, (*term).to_owned())).or_default() += 1;
    }
    insert(
        transaction,
        "INSERT INTO term_coverage(tape_id,kind,bad,residue,missing)",
        "",
        cover.iter().map(|(&kind, &(bad, residue))| {
            let missing = if accepted.contains(&kind) { &[][..] } else { set.missing.get(&kind).map_or(&[][..], Vec::as_slice) };
            [
                tape.into(),
                i64::from(kind).into(),
                i64::from(bad).into(),
                (residue as i64).into(),
                Value::Blob(pack(missing)),
            ]
        }),
    )?;
    Ok(())
}

/// Ascending preorders as a delta-varint segment.
pub(crate) fn pack(pres: &[u32]) -> Vec<u8> {
    let mut packed = Vec::new();
    let mut previous = 0;
    for &pre in pres {
        append_delta(&mut packed, pre - previous);
        previous = pre;
    }
    packed
}
