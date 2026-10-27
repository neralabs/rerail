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
//! - `tape_kind_counts`: per (tape, kind), the count and the delta-varint preorder postings: the
//!   access path for "which nodes of this tape have kind k", without decoding the tape.
//! - `term_postings`: case-exact leaf text per (lang, term, tape, kind), and `term_freq` the number
//!   of (tape, kind) rows carrying each term.
//! - `term_coverage`: per (tape, kind), whether every node of the kind is covered by a term
//!   (`bad` = 0), and the nodes that are not (`missing`). Unclean leaves (whitespace, control
//!   characters, too long) are kept as terms only while the store-wide residue of their kind stays
//!   under `RESIDUE_CAP`.

use super::bounded_insert::insert;
use super::postings::append_delta;
use super::{ContentStore, MAX_PUBLICATION_BATCH, StoreError, StoreResult, TapeId};
use rusqlite::{Transaction, params, types::Value};
use std::collections::{BTreeMap, BTreeSet};

/// The image a tape's rows were built by. 2: ERROR nodes are kind code 0 in both postings.
/// 3: term postings carry their node count, which the planner statistics sum.
pub(crate) const PREFILTER_VERSION: i64 = 3;
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
    /// (kind atom, count, delta-varint postings), sorted by atom.
    pub kinds: Vec<(u32, u32, Vec<u8>)>,
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
            for set in sets {
                put(transaction, set)?;
            }
            Ok(())
        })
    }

    /// Removes a tape's prefilter rows (they also go when the tape is collected).
    pub(crate) fn remove_prefilter(&self, tape: TapeId) -> StoreResult<()> {
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

    /// The preorders of `tape`'s nodes of kind atom `kind`, decoded in SQL (`nera_postings`).
    pub(crate) fn kind_positions(&self, tape: TapeId, kind: u32) -> StoreResult<Vec<u32>> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let mut statement = connection.prepare_cached(
            "SELECT p.pre FROM tape_kind_counts c, nera_postings(c.postings) p \
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
        if writes {
            self.require_write_capacity()?;
            return self.immediate_transaction(|transaction| Ok(operation(transaction)?));
        }
        let mut connection = self.connection.lock().expect("content store mutex poisoned");
        let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)?;
        let value = operation(&transaction)?;
        transaction.commit()?;
        Ok(value)
    }

    /// Deletes a tape, as collection does, for tests of the cascade.
    #[cfg(test)]
    pub(crate) fn delete_tape_for_test(&self, tape: TapeId) -> StoreResult<()> {
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

fn put(transaction: &Transaction<'_>, set: &TapePrefilter) -> StoreResult<()> {
    let tape = set.tape_id.row();
    // Publishing again replaces: the cascade and triggers count the old rows down.
    transaction.execute("DELETE FROM prefilter_sets WHERE tape_id=?1", [tape])?;
    transaction.execute(
        "INSERT INTO prefilter_sets(tape_id,version,nodes) VALUES(?1,?2,?3)",
        params![tape, PREFILTER_VERSION, set.nodes],
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
        "INSERT INTO tape_kind_counts(tape_id,kind,n,postings)",
        "",
        set.kinds.iter().map(|(kind, n, positions)| {
            [tape.into(), i64::from(*kind).into(), i64::from(*n).into(), Value::Blob(positions.clone())]
        }),
    )?;
    insert(
        transaction,
        "INSERT INTO kind_freq(kind,n)",
        "ON CONFLICT(kind) DO UPDATE SET n=n+excluded.n",
        set.kinds.iter().map(|(kind, n, _)| [i64::from(*kind).into(), i64::from(*n).into()]),
    )?;
    // In key order (term, then kind), so consecutive rows land on neighbouring leaves.
    let mut rows = postings.iter_mut().collect::<Vec<_>>();
    rows.sort_unstable_by_key(|((kind, term), _)| (*term, *kind));
    insert(
        transaction,
        "INSERT INTO term_postings(lang,term,tape_id,kind,n,postings)",
        "",
        rows.into_iter().map(|((kind, term), pres)| {
            pres.sort_unstable();
            [lang.into(), (*term).to_owned().into(), tape.into(), i64::from(*kind).into(), (pres.len() as i64).into(), Value::Blob(pack(pres))]
        }),
    )?;
    insert(
        transaction,
        "INSERT INTO term_node_freq(lang,term,kind,n)",
        "ON CONFLICT(lang,term,kind) DO UPDATE SET n=n+excluded.n",
        postings.iter().map(|((kind, term), pres)| {
            [lang.into(), (*term).to_owned().into(), i64::from(*kind).into(), (pres.len() as i64).into()]
        }),
    )?;
    let mut covered = BTreeMap::<u32, i64>::new();
    for ((kind, _), pres) in &postings {
        *covered.entry(*kind).or_default() += pres.len() as i64;
    }
    insert(
        transaction,
        "INSERT INTO term_kind_freq(kind,n)",
        "ON CONFLICT(kind) DO UPDATE SET n=n+excluded.n",
        covered.into_iter().map(|(kind, n)| [i64::from(kind).into(), n.into()]),
    )?;
    // One row per term, counting the tape's kinds that carry it, in place of a per-row trigger.
    let mut terms = BTreeMap::<&str, i64>::new();
    for (_, term) in postings.keys() {
        *terms.entry(term).or_default() += 1;
    }
    insert(
        transaction,
        "INSERT INTO term_freq(lang,term,n)",
        "ON CONFLICT(lang,term) DO UPDATE SET n=n+excluded.n",
        terms.into_iter().map(|(term, n)| [lang.into(), term.to_owned().into(), n.into()]),
    )?;
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
