//! Query-independent tape search index: publication, backlog cursor, and kind presence.
//!
//! Kept facts of the store (t33 step 7): a tape's index is one partition of the search space (its `tape_id`),
//! `si_set(index_version, node_count, complete, has_errors)` and its distinct kind tokens `si_kind(token)` (the
//! store's atoms), put whole at publication and gone in the transaction that deletes the tape (`forget_tapes`).
//! A presence lookup probes the tape's tokens where they lie. `search_index_sets` reads the sets through SQL
//! (`prefilter::vtab`). The kind stream's positions were never read but by the predecessor's positional index,
//! which keeps its own (`positional_search`).

#[cfg(any(test, feature = "test-hooks"))]
use super::SEARCH_INDEX_FAMILY_WRITES;
use super::kept_facts::{ChainRows, KeptPut, KeptSpace, KeptWhat, chain_rows, sorted_put, write_space};
use super::{
    ContentStore, HashKey, MAX_PUBLICATION_BATCH, SearchKindLookup, StoreError, StoreResult,
    StoredKindPresence, TapeId, TapeSearchIndex, UnindexedTape,
};
use rerail_blocks::kept::{FactKey, Kept, KeptRead};
use rusqlite::{Connection, params};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
#[cfg(any(test, feature = "test-hooks"))]
use std::sync::atomic::Ordering;

const SET: &str = "si_set";
const KIND: &str = "si_kind";

/// The search index's space.
pub fn space() -> [u8; 32] {
    Sha256::digest(b"rerail.search-index/v1").into()
}

fn ident(rel: &str) -> [u8; 32] {
    Sha256::digest(format!("rerail.search-index/v1\0{rel}").as_bytes()).into()
}

fn entry<'a, R: KeptRead + ?Sized>(view: &'a R, rel: &str, tape: i64) -> Option<Kept<'a>> {
    let ident = ident(rel);
    view.get(&FactKey { rel, ident: &ident, part: tape }).filter(|e| *e.key.ident == ident)
}

/// A tape's index set: (index_version, node_count, complete, has_errors).
pub fn set<R: KeptRead + ?Sized>(view: &R, tape: i64) -> Option<[i64; 4]> {
    let e = entry(view, SET, tape).filter(|e| e.rows.len() == 1)?;
    Some(std::array::from_fn(|c| e.rows.get(0, c)))
}

/// Every tape with an index set, by id.
pub fn tapes<R: KeptRead + ?Sized>(view: &R) -> impl Iterator<Item = i64> + '_ {
    let set = ident(SET);
    view.of(SET).filter(move |e| *e.key.ident == set).map(|e| e.key.part)
}

/// Whether tape `tape`'s kinds hold any of `tokens` (the store's atoms).
fn holds_any<R: KeptRead + ?Sized>(view: &R, tape: i64, tokens: &[i64]) -> bool {
    entry(view, KIND, tape).is_some_and(|e| tokens.iter().any(|&t| e.rows.probe(&[0], &[t]).next().is_some()))
}

fn set_put(tape: i64, row: [i64; 4]) -> StoreResult<KeptPut> {
    sorted_put(SET, ident(SET), tape, b"iiii", 1, |push| {
        row.into_iter().for_each(&mut *push);
        Ok::<(), StoreError>(())
    })
}

fn gone(tape: i64) -> impl Iterator<Item = KeptPut> {
    [SET, KIND].into_iter().map(move |rel| KeptPut { rel: rel.to_owned(), ident: ident(rel), part: tape, digest: 0, reads: Vec::new(), types: Vec::new(), what: KeptWhat::Gone })
}

fn open(store: &std::path::Path, t: &Connection) -> StoreResult<Option<KeptSpace>> {
    let rows = chain_rows(t, &space())?;
    if rows.is_empty() { Ok(None) } else { KeptSpace::open(store, &space(), &rows).map(Some) }
}

/// Tapes `tapes`' indexes gone, in transaction `t` of the store at `store`, which deletes their rows (their ids may
/// be given again). The chain's rows once written, for `ContentStore::note_kept` after the commit.
pub(in crate::content_store) fn forget_tapes(store: &std::path::Path, t: &Connection, tapes: &[i64]) -> StoreResult<Option<ChainRows>> {
    if tapes.is_empty() {
        return Ok(None);
    }
    let Some(old) = open(store, t)? else { return Ok(None) };
    let view = old.view();
    let puts: Vec<KeptPut> = tapes.iter().filter(|&&tape| set(&view, tape).is_some()).flat_map(|&tape| gone(tape)).collect();
    drop(view);
    if puts.is_empty() {
        return Ok(None);
    }
    write_space(store, t, &space(), puts, None)
}

/// The maintenance backlog's candidates: stored tapes of one grammar version and tape format after a keyset cursor,
/// by id; those without a search index are kept (`tapes_without_search_index_after`). Tapes of another tape format
/// are never in it: this build cannot decode them, and no lookup of this build reads them (`tape_objects` lookups
/// match the format too).
pub(super) const TAPES_WITHOUT_SEARCH_INDEX_SQL: &str = "SELECT t.tape_id FROM tape_objects t \
     WHERE t.grammar_version = ?1 AND t.tape_format_version = ?2 AND t.tape_id > ?3 \
     ORDER BY t.tape_id LIMIT ?4";

/// A backlog tape's source and tape frames, to build its index from.
const TAPE_FRAMES_SQL: &str = "SELECT t.grammar_id, \
     s.raw_length, sh.algorithm_id, sh.lane0, sh.lane1, sh.lane2, sh.lane3, \
     t.raw_length, th.algorithm_id, th.lane0, th.lane1, th.lane2, th.lane3, \
     s.seg, s.seg_at, s.seg_len, t.seg, t.seg_at, t.seg_len \
     FROM tape_objects t \
     JOIN hashes th ON th.hash_id = t.canonical_hash_id \
     JOIN source_objects s ON s.source_id = t.source_id \
     JOIN hashes sh ON sh.hash_id = s.canonical_hash_id \
     WHERE t.tape_id = ?1";

/// A tape's index to publish: its version, its node count, whether it has syntax errors and its kind stream.
pub(in crate::content_store) struct Document<'a> {
    pub tape: i64,
    pub version: i64,
    pub nodes: u32,
    pub errors: bool,
    pub kinds: &'a str,
}

impl ContentStore {
    /// The puts that publish `documents` (a tape given twice: its last), in transaction `t`: their kind tokens
    /// interned and mapped to the store's ids, under `atoms::mapped`.
    pub(in crate::content_store) fn search_index_puts(&self, t: &Connection, documents: &[Document<'_>]) -> StoreResult<Vec<KeptPut>> {
        let texts: Vec<&str> = documents.iter().flat_map(|d| d.kinds.split_ascii_whitespace()).collect::<BTreeSet<_>>().into_iter().collect();
        let mut ids: Vec<i64> = self.atoms.intern_all(&texts)?.into_iter().map(i64::from).collect();
        self.atoms.to_store(t, super::ProcessIds::of_process(&mut ids))?;
        let atom: std::collections::HashMap<&str, i64> = texts.iter().copied().zip(ids).collect();
        let mut last = std::collections::BTreeMap::new();
        for d in documents {
            last.insert(d.tape, d);
        }
        let mut puts = Vec::new();
        for (&tape, d) in &last {
            let kinds: BTreeSet<i64> = d.kinds.split_ascii_whitespace().map(|t| atom[t]).collect();
            puts.push(set_put(tape, [d.version, i64::from(d.nodes), 1, i64::from(d.errors)])?);
            puts.push(sorted_put(KIND, ident(KIND), tape, b"s", kinds.len(), |push| {
                kinds.iter().for_each(|&k| push(k));
                Ok::<(), StoreError>(())
            })?);
            #[cfg(any(test, feature = "test-hooks"))]
            SEARCH_INDEX_FAMILY_WRITES.fetch_add(1, Ordering::Relaxed);
        }
        Ok(puts)
    }

    /// Take `tapes`, which this build can never decode into a search index, out of the maintenance
    /// backlog: each gets an incomplete search-index set, which presence lookups answer as
    /// `Unknown`, so the tape is never narrowed. A tape that already has a set keeps it.
    pub fn put_unavailable_search_indexes(
        &self,
        tapes: &[TapeId],
        index_version: i64,
    ) -> StoreResult<()> {
        if tapes.len() > MAX_PUBLICATION_BATCH {
            return Err(StoreError::Invalid(
                "search-index publication exceeds 64 tapes".to_owned(),
            ));
        }
        self.require_write_capacity()?;
        let path = self.database_path.clone();
        let written = self.immediate_transaction(|t| {
            let old = open(&path, t)?;
            let view = old.as_ref().map(KeptSpace::view);
            let mut puts = Vec::new();
            for tape in tapes.iter().map(|tape| tape.0).collect::<BTreeSet<_>>() {
                if view.as_ref().is_some_and(|v| set(v, tape).is_some()) {
                    continue;
                }
                // Nothing reads the count of an incomplete set.
                puts.push(set_put(tape, [index_version, 1, 0, 0])?);
            }
            drop(view);
            if puts.is_empty() { Ok(None) } else { write_space(&path, t, &space(), puts, None) }
        })?;
        self.note_kept(&space(), written);
        Ok(())
    }

    /// Publish one query-independent index per immutable tape: its kinds, from the kind stream (one token per
    /// preorder node). All validation finishes before the short write transaction starts.
    pub fn put_search_indexes_bulk(&self, indexes: &[TapeSearchIndex]) -> StoreResult<()> {
        if indexes.len() > MAX_PUBLICATION_BATCH {
            return Err(StoreError::Invalid(
                "search-index publication exceeds 64 tapes".to_owned(),
            ));
        }
        for index in indexes {
            let valid_tokens = |stream: &str| {
                stream
                    .split_ascii_whitespace()
                    .all(|token| token.bytes().all(|byte| byte.is_ascii_alphanumeric()))
            };
            if index.index_version < 0
                || index.node_count == 0
                || index.kind_tokens.split_ascii_whitespace().count() != index.node_count as usize
                || !valid_tokens(&index.kind_tokens)
            {
                return Err(StoreError::Invalid(
                    "invalid positional FTS search document".to_owned(),
                ));
            }
        }
        self.require_write_capacity()?;
        let path = self.database_path.clone();
        // Its tokens are the store's ids: written after a fork too (decision 0170).
        let written = super::atoms::mapped(|| self.immediate_transaction(|t| {
            let documents: Vec<Document<'_>> = indexes.iter().map(|i| Document { tape: i.tape_id.0, version: i.index_version, nodes: i.node_count, errors: i.has_errors, kinds: &i.kind_tokens }).collect();
            let puts = self.search_index_puts(t, &documents)?;
            write_space(&path, t, &space(), puts, None)
        }))?;
        self.note_kept(&space(), written);
        Ok(())
    }

    /// Up to `limit` stored tapes of `grammar_version` and `tape_format_version` after `after`
    /// without a search index: tape id, grammar id, source text and decoded tape image. The caller
    /// retains the keyset cursor across maintenance wakes, so each batch seeks past already-indexed
    /// tapes.
    pub fn tapes_without_search_index_after(
        &self,
        grammar_version: i64,
        tape_format_version: i64,
        after: Option<TapeId>,
        limit: usize,
    ) -> StoreResult<Vec<UnindexedTape>> {
        type Frame = (i64, HashKey, [i64; 3]);
        let kept = self.kept_space(&space())?;
        let view = kept.as_ref().map(KeptSpace::view);
        let rows = {
            let connection = self
                .connection
                .lock()
                .expect("content store mutex poisoned");
            // The candidates by id, a page at a time, until `limit` without an index.
            let mut chosen = Vec::new();
            let mut cursor = after.map_or(0, |tape| tape.0);
            let page = limit.max(64);
            let mut candidates = connection.prepare_cached(TAPES_WITHOUT_SEARCH_INDEX_SQL)?;
            while chosen.len() < limit {
                let ids = candidates
                    .query_map(params![grammar_version, tape_format_version, cursor, page as i64], |row| row.get::<_, i64>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                let Some(&end) = ids.last() else { break };
                cursor = end;
                chosen.extend(ids.into_iter().filter(|&tape| view.as_ref().is_none_or(|v| set(v, tape).is_none())));
                chosen.truncate(limit);
            }
            drop(candidates);
            let mut statement = connection.prepare_cached(TAPE_FRAMES_SQL)?;
            let frame = |row: &rusqlite::Row<'_>, at: usize, seg: usize| -> rusqlite::Result<Frame> {
                Ok((
                    row.get(at)?,
                    HashKey {
                        algorithm: row.get(at + 1)?,
                        lanes: [
                            row.get(at + 2)?,
                            row.get(at + 3)?,
                            row.get(at + 4)?,
                            row.get(at + 5)?,
                        ],
                    },
                    [row.get(seg)?, row.get(seg + 1)?, row.get(seg + 2)?],
                ))
            };
            let mut rows = Vec::with_capacity(chosen.len());
            for tape in chosen {
                let mut found = statement.query_map([tape], |row| {
                    Ok((TapeId(tape), row.get::<_, i64>(0)?, frame(row, 1, 13)?, frame(row, 7, 16)?))
                })?;
                if let Some(row) = found.next() {
                    rows.push(row?);
                }
            }
            rows
        };
        drop(view);
        // A source is checked against its hash; its tape against its identity (`HashKey::tape_identity`).
        // In a segment (decision 0295 step 5): read there; one whose segment is gone is not indexed, nor an archived
        // one (decision 0450 step 4): placed back to be indexed in the background, it would undo the archive. An index
        // made before its archive is kept; a run that reads the tape places it back, and it is indexed after.
        let mut maps = super::segments::SegmentMaps::new(super::object_segments::object_dir(&self.database_path));
        let mut decode = |(length, hash, [seg, at, len]): Frame, identity: &dyn Fn(&[u8]) -> HashKey| -> StoreResult<Option<super::ObjectBytes>> {
            let Some(frame) = super::object_segments::row_frame(&mut maps, seg, at, len) else { return Ok(None) };
            let bytes = super::ObjectBytes::Mapped(frame.segment().clone());
            if usize::try_from(length).ok() != Some(bytes.len()) || identity(&bytes) != hash {
                return Err(StoreError::Corrupt(
                    "object length or hash does not match metadata".to_owned(),
                ));
            }
            Ok(Some(bytes))
        };
        let mut out = Vec::with_capacity(rows.len());
        for (tape, grammar, source, image) in rows {
            let source_hash = source.1;
            let (Some(source), Some(image)) = (decode(source, &|bytes| HashKey::sha256(bytes))?, decode(image, &|bytes| HashKey::tape_identity(source_hash, bytes))?) else { continue };
            out.push(UnindexedTape { tape, grammar, source, image });
        }
        Ok(out)
    }

    /// Returns whether each complete, error-free stored tape contains any requested root kind.
    /// Missing, stale, malformed, and syntax-error index rows remain non-negative answers.
    pub fn get_search_kind_presence_bulk(
        &self,
        lookups: &[SearchKindLookup],
    ) -> StoreResult<Vec<StoredKindPresence>> {
        let Some(kept) = self.kept_space(&space())? else { return Ok(vec![StoredKindPresence::Unknown; lookups.len()]) };
        let view = kept.view();
        // Each token's atom as the store numbers it; a token never stored is held by no tape.
        let texts: Vec<&str> = lookups.iter().flat_map(|l| l.kind_tokens.iter().map(String::as_str)).collect::<BTreeSet<_>>().into_iter().collect();
        let local: Vec<Option<super::Sym>> = self.atoms.lookup_all(&texts)?;
        let present: Vec<(&str, super::Sym)> = texts.iter().zip(local).filter_map(|(&t, l)| Some((t, l?))).collect();
        let stored = self.atoms.stored(&present.iter().map(|&(_, l)| l).collect::<Vec<_>>())?;
        let atom: std::collections::HashMap<&str, i64> = present.iter().zip(stored).filter_map(|(&(t, _), s)| Some((t, i64::from(s?)))).collect();
        Ok(lookups
            .iter()
            .map(|lookup| match set(&view, lookup.tape_id.0).filter(|s| s[0] == lookup.index_version) {
                Some([_, _, 1, 1]) => StoredKindPresence::SyntaxErrors,
                Some([_, _, 1, _]) => {
                    let tokens: Vec<i64> = lookup.kind_tokens.iter().filter_map(|t| atom.get(t.as_str()).copied()).collect();
                    if holds_any(&view, lookup.tape_id.0, &tokens) { StoredKindPresence::Present } else { StoredKindPresence::Absent }
                }
                _ => StoredKindPresence::Unknown,
            })
            .collect())
    }
}
