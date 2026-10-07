//! Row primitives shared by every table: hash lanes, interned hashes/strings/paths, validated
//! relative paths, and the store generation.

use super::{HashKey, STRING_PATH, StoreError, StoreResult};
use rusqlite::{OptionalExtension, Transaction, params};

pub(super) fn generation(transaction: &Transaction<'_>) -> StoreResult<i64> {
    transaction
        .query_row(
            "SELECT generation FROM store_state WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .map_err(StoreError::from)
}

pub(super) fn hash_params(key: HashKey) -> [i64; 5] {
    [
        key.algorithm,
        key.lanes[0],
        key.lanes[1],
        key.lanes[2],
        key.lanes[3],
    ]
}

pub(super) fn intern_hash(transaction: &Transaction<'_>, key: HashKey) -> StoreResult<i64> {
    let existing = transaction
        .prepare_cached(
            "SELECT hash_id FROM hashes INDEXED BY hashes_by_prefix \
             WHERE algorithm_id=?1 AND lane0=?2 AND lane1=?3 AND lane2=?4 AND lane3=?5",
        )?
        .query_row(hash_params(key), |row| row.get(0))
        .optional()?;
    if let Some(hash_id) = existing {
        return Ok(hash_id);
    }
    transaction
        .prepare_cached(
            "INSERT INTO hashes(algorithm_id,lane0,lane1,lane2,lane3) VALUES (?1,?2,?3,?4,?5)",
        )?
        .execute(hash_params(key))?;
    Ok(transaction.last_insert_rowid())
}

pub(super) fn intern_string(
    transaction: &Transaction<'_>,
    domain: i64,
    value: &str,
) -> StoreResult<i64> {
    let hash_id = intern_hash(transaction, HashKey::sha256(value.as_bytes()))?;
    let mut statement = transaction.prepare_cached(
        "SELECT string_id,collision_ordinal,value FROM strings WHERE domain_id=?1 AND content_hash_id=?2 ORDER BY collision_ordinal",
    )?;
    let mut rows = statement.query(params![domain, hash_id])?;
    let mut ordinal = 0_i64;
    while let Some(row) = rows.next()? {
        let candidate: String = row.get(2)?;
        if candidate == value {
            return row.get(0).map_err(StoreError::from);
        }
        ordinal = ordinal.max(row.get::<_, i64>(1)?.saturating_add(1));
    }
    drop(rows);
    drop(statement);
    transaction
        .prepare_cached(
            "INSERT INTO strings(domain_id,content_hash_id,collision_ordinal,value) VALUES (?1,?2,?3,?4)",
        )?
        .execute(params![domain, hash_id, ordinal, value])?;
    Ok(transaction.last_insert_rowid())
}

pub(super) fn intern_path(transaction: &Transaction<'_>, path: &str) -> StoreResult<i64> {
    let string_id = intern_string(transaction, STRING_PATH, path)?;
    transaction
        .prepare_cached(
            "INSERT INTO paths(string_id) VALUES (?1) ON CONFLICT(string_id) DO NOTHING",
        )?
        .execute([string_id])?;
    transaction
        .prepare_cached("SELECT path_id FROM paths WHERE string_id=?1")?
        .query_row([string_id], |row| row.get(0))
        .map_err(StoreError::from)
}

pub(super) fn validate_relative_path(path: &str) -> StoreResult<()> {
    if path.is_empty()
        || path.starts_with('/')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(StoreError::Invalid(
            "path must be a normalized repository-relative path".to_owned(),
        ));
    }
    Ok(())
}
