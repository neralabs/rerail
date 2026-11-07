//! The facts engine's memo (docs/facts/LOOP.md § Parity on changed plans only; PHASE-3.md § P3c):
//! one clause's per-file results under `(content hash, plan digest, fact schema version[, path])`.
//!
//! - The content is a stored source (`source_objects`), found by its blake3 alias
//!   (`HashKey::blake3`), so every memo row dies with its source (`ON DELETE CASCADE`): the store's
//!   reclamation, an older binary's included, can never leave a row naming another content.
//! - A clause whose result is a function of the content alone (the engine decides, statically, per
//!   clause) is stored under the empty path; its findings carry no path, and identical content at
//!   two paths is evaluated once. A clause that reads the path (scopes aside, which select clauses
//!   before the memo is read) is stored under its path.
//! - A clause run with something to report (findings, failures, repository rows, budget rows) is a
//!   `facts_results` row: the engine's encoded entry, opaque here. A clause run with nothing to
//!   report is one bit of a completion word of its plan set (`facts_completions`, bit `i` = the
//!   set's clause `i`), with its counts in the same row's counts frame, so a content's empty
//!   clauses cost one row per (set, path) rather than one per clause.
//! - A plan set is the ordered plan digests of a compiled policy set (`facts_plan_sets`); a lookup
//!   resolves completion bits of every set the content was evaluated under to plan digests, so a
//!   set that differs in a few plans still reuses every other clause's result.
//!
//! Older binaries never read these tables; every reference cascades from `source_objects`, and no
//! other shared table (hashes, strings, paths) is referenced, so their collection cannot orphan or
//! misread a row: the tables are additive and the floor stays (schema.rs).

use super::rows::{generation, intern_string};
use super::{
    CODEC_ZSTD, ContentStore, HashKey, STRING_OWNER, SourceId, StoreError, StoreResult, compress,
};
use rusqlite::{OptionalExtension, Transaction, params};
use sha2::{Digest as _, Sha256};
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

const OBJECT_SOURCE: i64 = 2;
/// The path content-pure rows are stored under (no stored path is empty: `validate_relative_path`).
const CONTENT_PURE: &str = "";

/// A compiled policy set's clause plan digests, in clause order; `digest` names the set.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FactsPlanSet {
    pub digest: [u8; 32],
    pub plans: Arc<[[u8; 32]]>,
}

impl FactsPlanSet {
    pub fn new(plans: Vec<[u8; 32]>) -> Self {
        let mut hash = Sha256::new();
        hash.update(b"rerail.facts-plan-set/v1\0");
        hash.update((plans.len() as u64).to_le_bytes());
        for plan in &plans {
            hash.update(plan);
        }
        Self {
            digest: hash.finalize().into(),
            plans: plans.into(),
        }
    }
}

/// Where a memo row lives: the content's source, the fact schema version and the path the
/// path-dependent rows are keyed by.
#[derive(Clone, Copy, Debug)]
pub struct FactsMemoKey<'a> {
    pub source: SourceId,
    pub fact_schema: u32,
    pub path: &'a str,
}

/// One clause run with something to report: its plan, whether it read the path, its entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FactsResult {
    pub plan: [u8; 32],
    pub path_dependent: bool,
    pub entry: Vec<u8>,
}

/// One clause run with nothing to report: its ordinal in the plan set it ran under, whether it
/// read the path, and its counts (opaque).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FactsCompletion {
    pub ordinal: u32,
    pub path_dependent: bool,
    pub counts: Vec<u8>,
}

/// What the memo holds for one (content, fact schema, path): every clause result by plan digest,
/// and every plan known to have completed with nothing to report, with its counts.
#[derive(Clone, Debug, Default)]
pub struct FactsMemo {
    pub results: HashMap<[u8; 32], FactsResult>,
    /// plan digest → (path dependent, counts)
    pub completed: HashMap<[u8; 32], (bool, Vec<u8>)>,
}

impl FactsMemo {
    pub fn is_empty(&self) -> bool {
        self.results.is_empty() && self.completed.is_empty()
    }

    /// The digest of what `set`'s clauses produced, in clause order, when the memo knows every
    /// one (None: a miss). It names the output, not the content: two contents whose clauses
    /// report the same things share it, so a re-run that reproduces it cuts off (C5, C14).
    pub fn run_digest(&self, set: &FactsPlanSet) -> Option<[u8; 32]> {
        let mut hash = Sha256::new();
        hash.update(b"rerail.facts-run/v1\0");
        for plan in set.plans.iter() {
            let (tag, path_dependent, bytes) = match (self.results.get(plan), self.completed.get(plan)) {
                (Some(result), _) => (1_u8, result.path_dependent, &result.entry),
                (None, Some((path_dependent, counts))) => (2, *path_dependent, counts),
                (None, None) => return None,
            };
            hash.update([tag, u8::from(path_dependent)]);
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        }
        Some(hash.finalize().into())
    }
}

impl HashKey {
    /// A blake3 content hash (the facts engine's content key), stored as a source alias.
    pub fn blake3(bytes: [u8; 32]) -> Self {
        Self::from_padded(super::HASH_BLAKE3, bytes)
    }
}

impl ContentStore {
    /// The stored source whose blake3 content hash is `content`.
    pub fn facts_source(&self, content: [u8; 32]) -> StoreResult<Option<SourceId>> {
        self.source_by_alias(HashKey::blake3(content))
    }

    /// Stores `bytes` (whose blake3 hash is `content`) with its blake3 alias and `aliases` (a Git
    /// blob id); returns its source.
    pub fn put_facts_source(
        &self,
        bytes: &[u8],
        content: [u8; 32],
        aliases: &[HashKey],
    ) -> StoreResult<SourceId> {
        let mut all = Vec::with_capacity(aliases.len() + 1);
        all.push(HashKey::blake3(content));
        all.extend_from_slice(aliases);
        self.put_source(bytes, &all)
    }

    /// Every memo row of `key`'s content, content-pure and at `key.path`, for `key.fact_schema`.
    pub fn facts_memo(&self, key: FactsMemoKey<'_>) -> StoreResult<FactsMemo> {
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let schema = i64::from(key.fact_schema);
        let mut memo = FactsMemo::default();
        let mut results = connection.prepare_cached(
            "SELECT path,plan_digest,codec_id,raw_length,frame FROM facts_results \
             WHERE source_id=?1 AND fact_schema=?2 AND path IN (?3,?4)",
        )?;
        let rows = results.query_map(params![key.source.0, schema, CONTENT_PURE, key.path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Vec<u8>>(4)?,
            ))
        })?;
        for row in rows {
            let (path, plan, codec, raw_length, frame) = row?;
            let plan = digest(&plan)?;
            let entry = decode(codec, raw_length, frame)?;
            memo.results.insert(
                plan,
                FactsResult {
                    plan,
                    path_dependent: path != CONTENT_PURE,
                    entry,
                },
            );
        }
        let mut completions = connection.prepare_cached(
            "SELECT path,set_digest,words,codec_id,raw_length,counts FROM facts_completions \
             WHERE source_id=?1 AND fact_schema=?2 AND path IN (?3,?4)",
        )?;
        let rows = completions
            .query_map(params![key.source.0, schema, CONTENT_PURE, key.path], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for (path, set, words, codec, raw_length, counts) in rows {
            let set = digest(&set)?;
            let plans = self.plan_set(&connection, set)?;
            let mut counts = decode_counts(&decode(codec, raw_length, counts)?)?;
            for (ordinal, plan) in plans.iter().enumerate() {
                if word_bit(&words, ordinal) && !memo.results.contains_key(plan) {
                    let c = counts.remove(&(ordinal as u32)).ok_or_else(|| {
                        StoreError::Corrupt("facts completion without counts".to_owned())
                    })?;
                    memo.completed.insert(*plan, (path != CONTENT_PURE, c));
                }
            }
        }
        Ok(memo)
    }

    /// Stores one file evaluation's clause runs under `set` (registered first): `results` as rows,
    /// `completions` as bits of the set's completion words (added to the stored words, with their
    /// counts) at the content-pure path or at `key.path`. Returns the run's digest
    /// (`FactsMemo::run_digest`) when it covered every clause of `set`.
    pub fn put_facts_memo(
        &self,
        key: FactsMemoKey<'_>,
        set: &FactsPlanSet,
        results: &[FactsResult],
        completions: &[FactsCompletion],
    ) -> StoreResult<Option<[u8; 32]>> {
        super::rows::validate_relative_path(key.path)?;
        let mut frames = Vec::with_capacity(results.len());
        for result in results {
            frames.push(compress(&result.entry, self.compression_level)?);
        }
        let mut by_path: [(Vec<u64>, HashMap<u32, Vec<u8>>); 2] = Default::default();
        for completion in completions {
            if completion.ordinal as usize >= set.plans.len() {
                return Err(StoreError::Invalid(
                    "facts completion outside its plan set".to_owned(),
                ));
            }
            let (words, counts) = &mut by_path[usize::from(completion.path_dependent)];
            let (word, bit) = (completion.ordinal as usize / 64, completion.ordinal % 64);
            if words.len() <= word {
                words.resize(word + 1, 0);
            }
            words[word] |= 1 << bit;
            counts.insert(completion.ordinal, completion.counts.clone());
        }
        let run = FactsMemo {
            results: results.iter().map(|result| (result.plan, result.clone())).collect(),
            completed: completions
                .iter()
                .map(|c| (set.plans[c.ordinal as usize], (c.path_dependent, c.counts.clone())))
                .collect(),
        }
        .run_digest(set);
        let schema = i64::from(key.fact_schema);
        let level = self.compression_level;
        self.immediate_transaction(|transaction| {
            let mut insert = transaction.prepare_cached(
                "INSERT INTO facts_results(source_id,fact_schema,path,plan_digest,codec_id,raw_length,frame) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(source_id,fact_schema,path,plan_digest) DO UPDATE \
                 SET codec_id=excluded.codec_id,raw_length=excluded.raw_length,frame=excluded.frame",
            )?;
            for (result, frame) in results.iter().zip(&frames) {
                let path = if result.path_dependent { key.path } else { CONTENT_PURE };
                insert.execute(params![
                    key.source.0,
                    schema,
                    path,
                    &result.plan[..],
                    CODEC_ZSTD,
                    result.entry.len() as i64,
                    frame
                ])?;
            }
            let mut set_written = false;
            for (dependent, (words, counts)) in by_path.iter().enumerate() {
                if words.is_empty() {
                    continue;
                }
                // The set's ordinals, in the transaction that first names it (collection deletes
                // a set no completion row names).
                if !set_written {
                    write_plan_set(transaction, set)?;
                    set_written = true;
                }
                let path = if dependent == 1 { key.path } else { CONTENT_PURE };
                merge_completion(transaction, key.source, schema, path, set, words, counts, level)?;
            }
            Ok(run)
        })
    }

    /// Pins `sources` for `owner` until `lease_until`, in one transaction (the daemon's held
    /// contents: their memo rows survive collection while a worktree holds them).
    pub fn pin_facts_sources(
        &self,
        sources: &[SourceId],
        owner: &str,
        lease_until: i64,
    ) -> StoreResult<()> {
        if owner.is_empty() || lease_until <= 0 {
            return Err(StoreError::Invalid("object pin is invalid".to_owned()));
        }
        self.immediate_transaction(|transaction| {
            let owner_id = intern_string(transaction, STRING_OWNER, owner)?;
            let mut pin = transaction.prepare_cached(
                "INSERT INTO object_pins(object_kind_id,object_id,owner_string_id,lease_until) VALUES (?1,?2,?3,?4) \
                 ON CONFLICT(object_kind_id,object_id,owner_string_id) DO UPDATE SET lease_until=excluded.lease_until",
            )?;
            for source in sources {
                pin.execute(params![OBJECT_SOURCE, source.0, owner_id, lease_until])?;
            }
            Ok(())
        })
    }

    /// Releases `owner`'s pins on `sources` (contents no worktree holds any more); collection then
    /// reclaims them with their memo rows once nothing else references them.
    pub fn unpin_facts_sources(&self, sources: &[SourceId], owner: &str) -> StoreResult<()> {
        self.immediate_transaction(|transaction| {
            let mut unpin = transaction.prepare_cached(
                "DELETE FROM object_pins WHERE object_kind_id=?1 AND object_id=?2 AND owner_string_id IN \
                 (SELECT string_id FROM strings WHERE domain_id=?3 AND value=?4)",
            )?;
            for source in sources {
                unpin.execute(params![OBJECT_SOURCE, source.0, STRING_OWNER, owner])?;
            }
            Ok(())
        })
    }

    /// The store's current generation (collection advances it): a source created before it and
    /// held by nothing is collectable.
    pub fn facts_generation(&self) -> StoreResult<i64> {
        self.immediate_transaction(|transaction| generation(transaction))
    }

    fn plan_set(
        &self,
        connection: &rusqlite::Connection,
        set: [u8; 32],
    ) -> StoreResult<Arc<[[u8; 32]]>> {
        if let Some(plans) = self.facts_plan_sets.lock().get(&set) {
            return Ok(plans.clone());
        }
        let plans = connection
            .prepare_cached(
                "SELECT ordinal,plan_digest FROM facts_plan_sets WHERE set_digest=?1 ORDER BY ordinal",
            )?
            .query_map([&set[..]], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut out = Vec::with_capacity(plans.len());
        for (expected, (ordinal, plan)) in plans.into_iter().enumerate() {
            if ordinal != expected as i64 {
                return Err(StoreError::Corrupt("facts plan set has a gap".to_owned()));
            }
            out.push(digest(&plan)?);
        }
        let plans: Arc<[[u8; 32]]> = out.into();
        // A set is immutable once written; one not (or no longer) stored is not cached.
        if !plans.is_empty() {
            self.facts_plan_sets.lock().insert(set, plans.clone());
        }
        Ok(plans)
    }
}

/// Records `set`'s ordinals unless the store holds them.
fn write_plan_set(transaction: &Transaction<'_>, set: &FactsPlanSet) -> StoreResult<()> {
    let held = transaction
        .prepare_cached("SELECT 1 FROM facts_plan_sets WHERE set_digest=?1 LIMIT 1")?
        .query_row([&set.digest[..]], |_| Ok(()))
        .optional()?
        .is_some();
    if held {
        return Ok(());
    }
    let mut insert = transaction.prepare_cached(
        "INSERT INTO facts_plan_sets(set_digest,ordinal,plan_digest) VALUES (?1,?2,?3)",
    )?;
    for (ordinal, plan) in set.plans.iter().enumerate() {
        insert.execute(params![&set.digest[..], ordinal as i64, &plan[..]])?;
    }
    Ok(())
}

/// Adds `words` and `counts` to the stored completion row of (source, schema, path, set).
#[allow(clippy::too_many_arguments)]
fn merge_completion(
    transaction: &Transaction<'_>,
    source: SourceId,
    schema: i64,
    path: &str,
    set: &FactsPlanSet,
    words: &[u64],
    counts: &HashMap<u32, Vec<u8>>,
    level: i32,
) -> StoreResult<()> {
    let stored = transaction
        .prepare_cached(
            "SELECT words,codec_id,raw_length,counts FROM facts_completions \
             WHERE source_id=?1 AND fact_schema=?2 AND path=?3 AND set_digest=?4",
        )?
        .query_row(params![source.0, schema, path, &set.digest[..]], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Vec<u8>>(3)?,
            ))
        })
        .optional()?;
    let (mut merged_words, mut merged_counts) = match stored {
        Some((w, codec, raw_length, frame)) => (
            w.chunks_exact(8)
                .map(|c| u64::from_le_bytes(c.try_into().expect("eight bytes")))
                .collect::<Vec<_>>(),
            decode_counts(&decode(codec, raw_length, frame)?)?,
        ),
        None => (Vec::new(), HashMap::new()),
    };
    if merged_words.len() < words.len() {
        merged_words.resize(words.len(), 0);
    }
    for (stored, new) in merged_words.iter_mut().zip(words) {
        *stored |= new;
    }
    for (ordinal, c) in counts {
        merged_counts.insert(*ordinal, c.clone());
    }
    let word_bytes: Vec<u8> = merged_words.iter().flat_map(|w| w.to_le_bytes()).collect();
    let raw = encode_counts(&merged_counts);
    let frame = compress(&raw, level)?;
    transaction
        .prepare_cached(
            "INSERT INTO facts_completions(source_id,fact_schema,path,set_digest,words,codec_id,raw_length,counts) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(source_id,fact_schema,path,set_digest) DO UPDATE \
             SET words=excluded.words,codec_id=excluded.codec_id,raw_length=excluded.raw_length,counts=excluded.counts",
        )?
        .execute(params![
            source.0,
            schema,
            path,
            &set.digest[..],
            word_bytes,
            CODEC_ZSTD,
            raw.len() as i64,
            frame
        ])?;
    Ok(())
}

fn word_bit(words: &[u8], ordinal: usize) -> bool {
    words
        .get(ordinal / 64 * 8..ordinal / 64 * 8 + 8)
        .is_some_and(|w| u64::from_le_bytes(w.try_into().expect("eight bytes")) >> (ordinal % 64) & 1 == 1)
}

fn digest(bytes: &[u8]) -> StoreResult<[u8; 32]> {
    bytes
        .try_into()
        .map_err(|_| StoreError::Corrupt("facts plan digest is not 32 bytes".to_owned()))
}

fn decode(codec: i64, raw_length: i64, frame: Vec<u8>) -> StoreResult<Vec<u8>> {
    if codec != CODEC_ZSTD {
        return Err(StoreError::Corrupt(format!("unsupported facts memo codec {codec}")));
    }
    let bytes = zstd::stream::decode_all(Cursor::new(frame))?;
    if bytes.len() != usize::try_from(raw_length).unwrap_or(usize::MAX) {
        return Err(StoreError::Corrupt(
            "facts memo length does not match metadata".to_owned(),
        ));
    }
    Ok(bytes)
}

/// Counts by ordinal: `u32` entries, then per entry `u32` ordinal, `u32` length and the bytes
/// (little-endian), in ordinal order.
fn encode_counts(counts: &HashMap<u32, Vec<u8>>) -> Vec<u8> {
    let mut ordinals: Vec<&u32> = counts.keys().collect();
    ordinals.sort_unstable();
    let mut out = Vec::new();
    out.extend_from_slice(&(ordinals.len() as u32).to_le_bytes());
    for ordinal in ordinals {
        let c = &counts[ordinal];
        out.extend_from_slice(&ordinal.to_le_bytes());
        out.extend_from_slice(&(c.len() as u32).to_le_bytes());
        out.extend_from_slice(c);
    }
    out
}

fn decode_counts(bytes: &[u8]) -> StoreResult<HashMap<u32, Vec<u8>>> {
    let corrupt = || StoreError::Corrupt("facts completion counts are malformed".to_owned());
    let mut at = 0usize;
    let u32_at = |at: &mut usize| -> StoreResult<u32> {
        let v = bytes.get(*at..*at + 4).ok_or_else(corrupt)?;
        *at += 4;
        Ok(u32::from_le_bytes(v.try_into().expect("four bytes")))
    };
    let n = u32_at(&mut at)?;
    let mut out = HashMap::with_capacity(n as usize);
    for _ in 0..n {
        let ordinal = u32_at(&mut at)?;
        let len = u32_at(&mut at)? as usize;
        let c = bytes.get(at..at + len).ok_or_else(corrupt)?;
        at += len;
        out.insert(ordinal, c.to_vec());
    }
    if at != bytes.len() {
        return Err(corrupt());
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
