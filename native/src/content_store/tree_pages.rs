//! Per-directory result pages (C16; docs/final-shape/03-architecture.md §15), adapted from Tempo's
//! tree result segments (ner-1562-scale-test-the-re-railer@8fe9ffed4, `tree_result_segments`):
//! a directory's results are stored as up to 64-file pages, compressed, and restored whole
//! wherever the same subtree appears (Tempo E33–E34: complete-root reuse in 610 ms, then 34 ms
//! median; 47 workspaces 48.7 s → 13.6 s).
//!
//! Tempo keyed pages by (tree hash, prefix, candidate catalog) and published them with a
//! workspace completion. Here they are keyed by what the directory's result depends on, (tree
//! hash, prefix, context chain, plan), so any scope holding the subtree finds them, and their
//! digest is the output digest of the scope's page derivation (`facts::pages`): a page set that is
//! recomputed identical cuts off. Pages no name binds are collected (reclamation.rs).

use super::{CODEC_ZSTD, ContentStore, HashKey, StoreError, StoreResult, compress};
use rusqlite::OptionalExtension;
use sha2::{Digest as _, Sha256};
use std::io::Cursor;

/// Files per page (Tempo's segment bound).
pub const TREE_PAGE_FILES: usize = 64;

/// What one directory's result depends on.
#[derive(Clone, Copy, Debug)]
pub struct TreePageKey<'a> {
    pub tree: HashKey,
    pub prefix: &'a str,
    /// The context chain (package markers and other ancestry the result reads), as one digest.
    pub context: [u8; 32],
    /// The plan identity of what produced the pages.
    pub plan: [u8; 32],
}

impl TreePageKey<'_> {
    pub fn key(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"rerail.tree-page-key/v1\0");
        hash.update(self.tree.algorithm.to_le_bytes());
        for lane in self.tree.lanes {
            hash.update(lane.to_le_bytes());
        }
        hash.update((self.prefix.len() as u64).to_le_bytes());
        hash.update(self.prefix.as_bytes());
        hash.update(self.context);
        hash.update(self.plan);
        hash.finalize().into()
    }
}

/// One page: the encoded results of up to `TREE_PAGE_FILES` files.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreePage {
    pub file_count: usize,
    pub bytes: Vec<u8>,
}

/// The digest of a directory's pages: what its page derivation outputs.
pub fn tree_pages_digest(pages: &[TreePage]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"rerail.tree-pages/v1\0");
    hash.update((pages.len() as u64).to_le_bytes());
    for page in pages {
        hash.update((page.file_count as u64).to_le_bytes());
        hash.update((page.bytes.len() as u64).to_le_bytes());
        hash.update(&page.bytes);
    }
    hash.finalize().into()
}

impl ContentStore {
    /// Stores a directory's pages under `key`; returns their digest. Storing pages again under a
    /// key is a no-op, and other bytes under a stored key are refused.
    pub fn put_tree_pages(&self, key: &TreePageKey<'_>, pages: &[TreePage]) -> StoreResult<[u8; 32]> {
        if pages.is_empty() || pages.iter().any(|page| page.file_count > TREE_PAGE_FILES) {
            return Err(StoreError::Invalid("a directory has one or more pages of at most 64 files".to_owned()));
        }
        let (page_key, digest) = (key.key(), tree_pages_digest(pages));
        let page_count = pages.len() as i64;
        let mut plans = Vec::new();
        let mut index = 0_i64;
        for chunk in pages.chunks(256) {
            let mut sql = String::from(
                "INSERT INTO tree_pages(page_key,page_index,page_count,digest,codec_id,raw_length,file_count,frame) VALUES ",
            );
            let mut values = Vec::with_capacity(chunk.len() * 8);
            for (position, page) in chunk.iter().enumerate() {
                if position > 0 {
                    sql.push(',');
                }
                sql.push_str("(?,?,?,?,?,?,?,?)");
                values.push(rusqlite::types::Value::Blob(page_key.to_vec()));
                values.push(rusqlite::types::Value::Integer(index));
                values.push(rusqlite::types::Value::Integer(page_count));
                values.push(rusqlite::types::Value::Blob(digest.to_vec()));
                values.push(rusqlite::types::Value::Integer(CODEC_ZSTD));
                values.push(rusqlite::types::Value::Integer(page.bytes.len() as i64));
                values.push(rusqlite::types::Value::Integer(page.file_count as i64));
                values.push(rusqlite::types::Value::Blob(compress(&page.bytes, self.compression_level)?));
                index += 1;
            }
            // A stored page is "updated" to itself only when every column agrees, so the changed
            // count is the chunk's length exactly when the stored pages are these pages.
            sql.push_str(
                " ON CONFLICT(page_key,page_index) DO UPDATE SET raw_length=excluded.raw_length \
                 WHERE tree_pages.page_count=excluded.page_count AND tree_pages.digest=excluded.digest \
                 AND tree_pages.raw_length=excluded.raw_length AND tree_pages.file_count=excluded.file_count",
            );
            plans.push((sql, values, chunk.len()));
        }
        self.immediate_transaction(|transaction| {
            for (sql, values, expected) in &plans {
                let changed = transaction.execute(sql.as_str(), rusqlite::params_from_iter(values.iter()))?;
                if changed != *expected {
                    return Err(StoreError::Corrupt("tree page identity has different bytes".to_owned()));
                }
            }
            Ok(())
        })?;
        Ok(digest)
    }

    /// The pages stored under `key` and their digest, if every page is stored.
    pub fn tree_pages(&self, key: &TreePageKey<'_>) -> StoreResult<Option<([u8; 32], Vec<TreePage>)>> {
        let rows = {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            connection
                .prepare_cached(
                    "SELECT page_index,page_count,digest,codec_id,raw_length,file_count,frame FROM tree_pages \
                     WHERE page_key=?1 ORDER BY page_index",
                )?
                .query_map([&key.key()[..]], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, Vec<u8>>(6)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        let Some(first) = rows.first() else {
            return Ok(None);
        };
        let (count, digest) = (first.1, first.2.clone());
        // A partly collected page set is a miss.
        if rows.len() as i64 != count {
            return Ok(None);
        }
        let mut pages = Vec::with_capacity(rows.len());
        for (expected, (index, page_count, page_digest, codec, raw_length, file_count, frame)) in rows.into_iter().enumerate() {
            if index != expected as i64 || page_count != count || page_digest != digest || codec != CODEC_ZSTD {
                return Err(StoreError::Corrupt("tree pages disagree".to_owned()));
            }
            let bytes = zstd::stream::decode_all(Cursor::new(frame))?;
            if bytes.len() != usize::try_from(raw_length).unwrap_or(usize::MAX) {
                return Err(StoreError::Corrupt("tree page length mismatch".to_owned()));
            }
            let file_count = usize::try_from(file_count)
                .map_err(|_| StoreError::Corrupt("tree page file count is invalid".to_owned()))?;
            pages.push(TreePage { file_count, bytes });
        }
        let digest: [u8; 32] = digest
            .try_into()
            .map_err(|_| StoreError::Corrupt("tree page digest is not 32 bytes".to_owned()))?;
        if tree_pages_digest(&pages) != digest {
            return Err(StoreError::Corrupt("tree pages do not match their digest".to_owned()));
        }
        Ok(Some((digest, pages)))
    }

    /// Whether `key` has pages stored (without decoding them).
    pub fn has_tree_pages(&self, key: &TreePageKey<'_>) -> StoreResult<bool> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        Ok(connection
            .prepare_cached("SELECT 1 FROM tree_pages WHERE page_key=?1 LIMIT 1")?
            .query_row([&key.key()[..]], |_| Ok(()))
            .optional()?
            .is_some())
    }
}

#[cfg(test)]
mod tests;
