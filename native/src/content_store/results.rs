//! A workspace's per-file clause results and its completed directories (t33 step 8), kept as entries of the store's
//! per-file memo (decision 0240 §4, `held::memo_pack`): each entry's bytes as they are in a segment, read there in
//! place, and one row finding it. Kept facts' blocks hold words, and a result is text (witnesses, captures, fixes), so
//! the memo's form holds it; the five tables that did (`candidate_catalogs`, `candidate_checks`,
//! `context_check_results`, `completed_directory_reuse`, `completed_workspace_revisions`) are gone.
//!
//! - A file's results under one context and one result catalog (the rules, the engine build, the grammars) are one
//!   entry of `RESULTS`: every clause's result for it, so an entry is written whole. Its digest is the file's content,
//!   and collection drops it once the store keeps no source of that content (`collect_workspace_results`), as the
//!   result rows cascaded from their source.
//! - A completed directory (its tree, prefix and package markers, under a catalog) is one entry of `DIRECTORIES`,
//!   dropped once no result of its catalog is kept.
//!
//! An entry names its catalog, content and context in its image, and a read checks them: its part is a digest of
//! them, so a read never takes another's entry.

use super::held::{MemoBlob, MemoPut};
use super::{CandidateCatalogIdentity, CandidateClause, ContentStore, DirectoryReuse, HashKey, ObjectBytes, StoreError, StoreResult, StoredLocalResult};
use rusqlite::{Connection, params};
use sha2::{Digest as _, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// The results' entries' space.
fn results_space() -> [u8; 32] {
    Sha256::digest(b"rerail.workspace-results/v1").into()
}

/// The completed directories' entries' space.
fn directories_space() -> [u8; 32] {
    Sha256::digest(b"rerail.workspace-directories/v1").into()
}

const RESULTS_MAGIC: &[u8; 4] = b"RWR1";
const DIRECTORY_MAGIC: &[u8; 4] = b"RWD1";

/// The part of the entry of `keys`: a digest of them.
fn part(keys: &[&[u8]]) -> i64 {
    let mut h = Sha256::new();
    for k in keys {
        h.update((k.len() as u64).to_le_bytes());
        h.update(k);
    }
    i64::from_le_bytes(h.finalize()[..8].try_into().expect("8 bytes")) & i64::MAX
}

/// An entry's catalog, as its row's document key: what collection asks of a directory's catalog.
fn catalog_tag(catalog: &CandidateCatalogIdentity) -> i64 {
    i64::from_le_bytes(Sha256::digest(catalog.key().key_bytes())[..8].try_into().expect("8 bytes"))
}

/// One file's results under one context, as a workspace writes them: each clause's result bytes.
pub struct WorkspaceResults {
    /// The file's content (its SHA-256 key).
    pub source: HashKey,
    pub context: HashKey,
    pub results: Vec<(CandidateClause, Vec<u8>)>,
}

fn results_part(catalog: &CandidateCatalogIdentity, source: HashKey, context: HashKey) -> i64 {
    part(&[RESULTS_MAGIC, &catalog.key().key_bytes(), &source.key_bytes(), &context.key_bytes()])
}

fn directory_part(catalog: &CandidateCatalogIdentity, directory: &DirectoryReuse) -> i64 {
    part(&[DIRECTORY_MAGIC, &catalog.key().key_bytes(), &directory.tree.key_bytes(), &directory.package_markers.key_bytes(), directory.prefix.as_bytes()])
}

fn put_len(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
}

/// The entry's image: its magic, catalog, content and context, then each result's rule, clause and bytes.
fn results_image(catalog: &CandidateCatalogIdentity, file: &WorkspaceResults) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 120 + file.results.iter().map(|(c, b)| 48 + c.clause.len() + b.len()).sum::<usize>());
    out.extend_from_slice(RESULTS_MAGIC);
    for key in [catalog.key(), file.source, file.context] {
        out.extend_from_slice(&key.key_bytes());
    }
    out.extend_from_slice(&(file.results.len() as u32).to_le_bytes());
    for (candidate, bytes) in &file.results {
        out.extend_from_slice(&candidate.rule.key_bytes());
        put_len(&mut out, candidate.clause.as_bytes());
        put_len(&mut out, bytes);
    }
    out
}

/// A reader of an entry's image, from its start.
struct Image<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Image<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let got = self.bytes.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(got)
    }
    fn len(&mut self) -> Option<usize> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?) as usize)
    }
    fn key(&mut self) -> Option<HashKey> {
        HashKey::from_key_bytes(self.take(40)?.try_into().ok()?)
    }
}

/// The results an entry holds, each read in place where the entry lies; `None` when it is not the entry of `catalog`,
/// `source` and `context` (or does not read).
fn results_of(blob: &MemoBlob, catalog: &CandidateCatalogIdentity, source: HashKey, context: HashKey) -> Option<Vec<StoredLocalResult>> {
    let mut image = Image { bytes: blob.bytes.bytes(), at: 0 };
    if image.take(4)? != RESULTS_MAGIC || image.key()? != catalog.key() || image.key()? != source || image.key()? != context {
        return None;
    }
    let n = image.len()?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let rule = image.key()?;
        let clause_len = image.len()?;
        let clause = std::str::from_utf8(image.take(clause_len)?).ok()?.to_owned();
        let len = image.len()?;
        let at = image.at;
        image.take(len)?;
        out.push(StoredLocalResult { candidate: CandidateClause { rule, clause }, bytes: ObjectBytes::Mapped(blob.bytes.range(at, len)?) });
    }
    Some(out)
}

fn directory_image(catalog: &CandidateCatalogIdentity, directory: &DirectoryReuse) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 120 + 4 + directory.prefix.len());
    out.extend_from_slice(DIRECTORY_MAGIC);
    for key in [catalog.key(), directory.tree, directory.package_markers] {
        out.extend_from_slice(&key.key_bytes());
    }
    put_len(&mut out, directory.prefix.as_bytes());
    out
}

impl ContentStore {
    /// Keeps `files`' results under `catalog`, each file's under each of its contexts one entry, in place of what was
    /// kept for them. Busy when the store stays busy past its write deadline: nothing is kept, and the files' results
    /// stay with their workspace.
    pub fn put_workspace_results(&self, catalog: CandidateCatalogIdentity, files: &[WorkspaceResults]) -> StoreResult<()> {
        let tag = catalog_tag(&catalog);
        let puts = files
            .iter()
            .map(|file| {
                let digest = file.source.sha256_bytes().ok_or_else(|| StoreError::Invalid("a workspace result's content is not a SHA-256 key".to_owned()))?;
                let part = results_part(&catalog, file.source, file.context);
                Ok(MemoPut { part, path: format!("{part:016x}"), digest, key: tag, bytes: results_image(&catalog, file), over: false })
            })
            .collect::<StoreResult<Vec<_>>>()?;
        self.put_memo_entries(&results_space(), puts, 1)
    }

    /// The results kept under `catalog` for each (content, context) of `keys` that has some.
    #[allow(clippy::type_complexity)]
    pub fn workspace_results(&self, keys: &[(HashKey, HashKey)], catalog: CandidateCatalogIdentity) -> StoreResult<HashMap<(HashKey, HashKey), Arc<Vec<StoredLocalResult>>>> {
        if keys.is_empty() {
            return Ok(HashMap::new());
        }
        let parts: Vec<i64> = keys.iter().map(|&(source, context)| results_part(&catalog, source, context)).collect();
        let found = self.memo_entries(&results_space(), &parts, true)?;
        Ok(keys
            .iter()
            .zip(&parts)
            .filter_map(|(&(source, context), part)| Some(((source, context), Arc::new(results_of(found.get(part)?, &catalog, source, context)?))))
            .collect())
    }

    /// Records `directories` as evaluated whole under `catalog`: a later scan restores their clean files' results.
    pub fn complete_directories(&self, catalog: CandidateCatalogIdentity, directories: &[DirectoryReuse]) -> StoreResult<()> {
        let tag = catalog_tag(&catalog);
        let digest: [u8; 32] = Sha256::digest(catalog.key().key_bytes()).into();
        let puts = directories
            .iter()
            .map(|directory| {
                let part = directory_part(&catalog, directory);
                MemoPut { part, path: format!("{part:016x}"), digest, key: tag, bytes: directory_image(&catalog, directory), over: false }
            })
            .collect();
        self.put_memo_entries(&directories_space(), puts, 1)
    }

    /// The prefixes of `directories` recorded as evaluated whole under `catalog` (`complete_directories`).
    pub fn reusable_directory_prefixes(&self, catalog: CandidateCatalogIdentity, directories: &[DirectoryReuse]) -> StoreResult<HashSet<String>> {
        if directories.is_empty() {
            return Ok(HashSet::new());
        }
        let parts: Vec<i64> = directories.iter().map(|d| directory_part(&catalog, d)).collect();
        let found = self.memo_entries(&directories_space(), &parts, true)?;
        Ok(directories
            .iter()
            .zip(&parts)
            .filter(|(d, part)| found.get(part).is_some_and(|blob| blob.bytes.bytes() == directory_image(&catalog, d).as_slice()))
            .map(|(d, _)| d.prefix.clone())
            .collect())
    }

    /// Every kept result, as (content, clause, bytes), of every catalog and context, in that order.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn all_workspace_results(&self) -> StoreResult<Vec<(HashKey, String, Vec<u8>)>> {
        let parts: Vec<i64> = {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            let mut st = connection.prepare("SELECT part FROM memo_entries WHERE space = ?1")?;
            st.query_map([&results_space()[..]], |r| r.get(0))?.collect::<Result<_, _>>()?
        };
        let mut out = Vec::new();
        for blob in self.memo_entries(&results_space(), &parts, true)?.values() {
            let mut image = Image { bytes: blob.bytes.bytes(), at: 0 };
            let read = (|| {
                image.take(4)?;
                let (catalog, source, context) = (image.key()?, image.key()?, image.key()?);
                let _ = (catalog, context);
                let mut got = Vec::new();
                for _ in 0..image.len()? {
                    image.key()?;
                    let n = image.len()?;
                    let clause = String::from_utf8(image.take(n)?.to_vec()).ok()?;
                    let n = image.len()?;
                    got.push((source, clause, image.take(n)?.to_vec()));
                }
                Some(got)
            })();
            out.extend(read.ok_or_else(|| StoreError::Invalid("a workspace results entry does not read".to_owned()))?);
        }
        out.sort();
        Ok(out)
    }

    /// How many completed directories the store keeps, of every catalog.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn completed_directory_count(&self) -> StoreResult<usize> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let n: i64 = connection.query_row("SELECT count(*) FROM memo_entries WHERE space = ?1", [&directories_space()[..]], |r| r.get(0))?;
        Ok(n as usize)
    }
}

/// Collects, in `t` (a write transaction), the workspace results whose content the store no longer keeps, and the
/// completed directories of a catalog with no result kept. How many.
pub(super) fn collect_workspace_results(t: &Connection) -> StoreResult<usize> {
    let results = results_space();
    let digests: Vec<Vec<u8>> = t.prepare_cached("SELECT DISTINCT digest FROM memo_entries WHERE space = ?1")?.query_map([&results[..]], |r| r.get(0))?.collect::<Result<_, _>>()?;
    let mut n = 0;
    for digest in digests {
        let kept = <[u8; 32]>::try_from(digest.as_slice()).is_ok_and(|d| super::objects::source_id_by_hash(t, HashKey::from_sha256_digest(d)).ok().flatten().is_some());
        if !kept {
            n += t.prepare_cached("DELETE FROM memo_entries WHERE space = ?1 AND digest = ?2")?.execute(params![&results[..], digest])?;
        }
    }
    let directories = directories_space();
    n += t
        .prepare_cached("DELETE FROM memo_entries WHERE space = ?1 AND doc NOT IN (SELECT DISTINCT doc FROM memo_entries WHERE space = ?2)")?
        .execute(params![&directories[..], &results[..]])?;
    Ok(n)
}

