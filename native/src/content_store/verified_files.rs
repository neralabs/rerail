//! Clean worktree files whose bytes a restore hashed equal to their committed blob after their
//! change time moved, remembered with the volume, file id and change time they had then: a later
//! restore takes such a file without a read while it keeps all three
//! (`git_workspace::VerifiedFiles`).

use super::rows::hash_params;
use super::{ContentStore, HashKey, MAX_COLLECTION_BATCH, StoreError, StoreResult};
use rusqlite::{Row, params};

/// How long a remembered file stays without being hashed again, in seconds: after it, a file
/// proven by its row alone is read once more, and the rows of a worktree no restore reaches go.
const VERIFIED_FILE_SECONDS: i64 = 30 * 24 * 60 * 60;

const INSERT_VERIFIED: &str = "INSERT INTO verified_worktree_files\
     (worktree,path,volume,file_id,change_time,blob,verified_at) VALUES (?1,?2,?3,?4,?5,?6,?7) \
     ON CONFLICT(worktree,path) DO UPDATE SET volume=excluded.volume,file_id=excluded.file_id,\
     change_time=excluded.change_time,blob=excluded.blob,verified_at=excluded.verified_at";

const FORGET_VERIFIED: &str = "DELETE FROM verified_worktree_files WHERE (worktree,path) IN (\
     SELECT worktree,path FROM verified_worktree_files WHERE verified_at < ?1 \
     ORDER BY verified_at LIMIT ?2)";

/// A worktree file, by its repository path, that held `blob`'s bytes while it had this volume,
/// file id and change time.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedWorktreeFile {
    pub path: String,
    pub volume: u64,
    pub file_id: [u8; 16],
    /// The file's change time, in the file system's own units.
    pub changed: i64,
    pub blob: HashKey,
}

impl ContentStore {
    /// The files of the worktree whose root is `worktree` that restores remembered.
    pub fn verified_worktree_files(
        &self,
        worktree: &str,
    ) -> StoreResult<Vec<VerifiedWorktreeFile>> {
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let mut statement = connection.prepare_cached(
            "SELECT path,volume,file_id,change_time,blob FROM verified_worktree_files \
             WHERE worktree=?1",
        )?;
        let files = statement.query_map([worktree], verified_file)?;
        files.collect::<Result<_, _>>().map_err(StoreError::from)
    }

    /// Remembers `files` of the worktree whose root is `worktree`, verified at `now` (Unix
    /// seconds), and forgets rows older than `VERIFIED_FILE_SECONDS`, in one write that never
    /// waits for another writer: `Busy` while one holds the store, and then nothing is kept, which
    /// costs the next restore a read of each file.
    pub fn remember_verified_worktree_files(
        &self,
        worktree: &str,
        files: &[VerifiedWorktreeFile],
        now: i64,
    ) -> StoreResult<()> {
        self.immediate_transaction_no_wait(|transaction| {
            let mut insert = transaction.prepare_cached(INSERT_VERIFIED)?;
            for file in files {
                insert.execute(params![
                    worktree,
                    file.path,
                    file.volume.cast_signed(),
                    file.file_id,
                    file.changed,
                    blob_bytes(file.blob),
                    now
                ])?;
            }
            transaction.execute(
                FORGET_VERIFIED,
                params![now - VERIFIED_FILE_SECONDS, MAX_COLLECTION_BATCH as i64],
            )?;
            Ok(())
        })
    }
}

/// A row of `verified_worktree_files` as the file it remembers.
fn verified_file(row: &Row<'_>) -> rusqlite::Result<VerifiedWorktreeFile> {
    Ok(VerifiedWorktreeFile {
        path: row.get(0)?,
        volume: row.get::<_, i64>(1)?.cast_unsigned(),
        file_id: row.get(2)?,
        changed: row.get(3)?,
        blob: blob_key(row.get(4)?),
    })
}

/// A blob id as 40 bytes: its algorithm and its four lanes, each eight bytes big-endian.
fn blob_bytes(key: HashKey) -> [u8; 40] {
    let mut bytes = [0_u8; 40];
    for (chunk, value) in bytes.chunks_exact_mut(8).zip(hash_params(key)) {
        chunk.copy_from_slice(&value.to_be_bytes());
    }
    bytes
}

/// The blob id `blob_bytes` wrote.
fn blob_key(bytes: [u8; 40]) -> HashKey {
    let mut values = [0_i64; 5];
    for (value, chunk) in values.iter_mut().zip(bytes.chunks_exact(8)) {
        *value = i64::from_be_bytes(chunk.try_into().expect("eight-byte hash lane"));
    }
    HashKey {
        algorithm: values[0],
        lanes: [values[1], values[2], values[3], values[4]],
    }
}
