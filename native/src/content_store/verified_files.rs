//! Clean worktree files whose bytes a restore hashed equal to their committed blob after their
//! change time moved, remembered with the volume, file id and change time they had then: a later
//! restore takes such a file without a read while it keeps all three
//! (`git_workspace::VerifiedFiles`).
//!
//! They are the rows of a relation, kept in the store's frames as every relation's rows are and
//! written by the store's writer (`queue_frames`), with no table of their own (decisions 0103 and
//! 0104). One space holds them. Each worktree's files are one part of `verified_worktree_file`,
//! and part 0 of `verified_worktree` names each such part with when it was last written, so the
//! files of a worktree no restore writes for 30 days are forgotten at a later write.

use super::frames::{Put, Rel, Rows};
use super::{ContentStore, HashKey, MAX_COLLECTION_BATCH, StoreResult};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

/// How long a remembered file stays without being hashed again, in seconds: after it, a file
/// proven by its row alone is read once more, and the rows of a worktree no restore reaches go.
const VERIFIED_FILE_SECONDS: i64 = 30 * 24 * 60 * 60;

/// The relations' names, each with the columns its identity names: a change of the columns is
/// another relation, whose rows are hashed again.
const FILES: (&str, &str) = (
    "verified_worktree_file",
    "path key, volume, file id (2), change time, blob algorithm, blob lanes (4), verified at",
);
const WORKTREES: (&str, &str) = ("verified_worktree", "part, written at");

/// The columns of a `verified_worktree_file` row, every one an integer.
const FILE_WIDTH: usize = 11;

/// An integer column, in the engine's codes for a frame's value types.
const INTEGER: u8 = b'i';

/// A worktree file, by the key of its repository path (`worktree_path_key`), that held `blob`'s
/// bytes while it had this volume, file id and change time.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedWorktreeFile {
    pub path: i64,
    pub volume: u64,
    pub file_id: [u8; 16],
    /// The file's change time, in the file system's own units.
    pub changed: i64,
    pub blob: HashKey,
}

/// The key of a repository path: its SHA-256's first eight bytes. Two paths with one key share a
/// row, which costs a read and never proves a file: a row holds only for the file whose volume,
/// file id and change time it names.
pub fn worktree_path_key(path: &str) -> i64 {
    first_eight(Sha256::digest(path.as_bytes()).into())
}

impl ContentStore {
    /// The files of the worktree whose root is `worktree` that restores remembered.
    pub fn verified_worktree_files(
        &self,
        worktree: &str,
    ) -> StoreResult<Vec<VerifiedWorktreeFile>> {
        let rows = self.verified_rows(worktree_part(worktree))?;
        Ok(rows.into_values().map(|(file, _)| file).collect())
    }

    /// Remembers `files` of the worktree whose root is `worktree`, verified at `now` (Unix
    /// seconds), each replacing the row of its path. The worktree's rows older than
    /// `VERIFIED_FILE_SECONDS` are forgotten, and so are the rows of worktrees not written for
    /// that long. Handed to the store's writer: the restore does not wait for the write.
    pub fn remember_verified_worktree_files(
        &self,
        worktree: &str,
        files: &[VerifiedWorktreeFile],
        now: i64,
    ) -> StoreResult<()> {
        let part = worktree_part(worktree);
        let since = now - VERIFIED_FILE_SECONDS;
        let mut kept = self.verified_rows(part)?;
        kept.retain(|_, (_, verified)| *verified >= since);
        kept.extend(files.iter().map(|file| (file.path, (file.clone(), now))));
        let mut written = self.written_worktrees()?;
        written.insert(part, now);
        let old: Vec<i64> = written
            .iter()
            .filter(|(_, at)| **at < since)
            .map(|(old, _)| *old)
            .take(MAX_COLLECTION_BATCH)
            .collect();
        written.retain(|other, _| !old.contains(other));
        let files_rows = kept.values().flat_map(|(file, at)| file_row(file, *at));
        let worktree_rows = written.iter().flat_map(|(part, at)| [*part, *at]);
        let puts = vec![
            put(FILES, part, files_rows.collect(), FILE_WIDTH),
            put(WORKTREES, 0, worktree_rows.collect(), 2),
        ];
        let gone = old.into_iter().map(|old| (rel(FILES), old)).collect();
        self.queue_frames(space(), puts, gone, None);
        Ok(())
    }

    /// The worktree's rows in part `part`, by path key, each with when it was verified.
    fn verified_rows(&self, part: i64) -> StoreResult<BTreeMap<i64, (VerifiedWorktreeFile, i64)>> {
        let rows = self.stored_rows(FILES, part)?;
        Ok(rows
            .chunks_exact(FILE_WIDTH)
            .map(|row| (row[0], (verified_file(row), row[10])))
            .collect())
    }

    /// Each worktree's part, with when it was last written.
    fn written_worktrees(&self) -> StoreResult<BTreeMap<i64, i64>> {
        let rows = self.stored_rows(WORKTREES, 0)?;
        Ok(rows.chunks_exact(2).map(|row| (row[0], row[1])).collect())
    }

    /// The rows of `relation`'s part `part`, flat, as queued or stored: none when it has no
    /// frame there.
    fn stored_rows(&self, relation: (&str, &str), part: i64) -> StoreResult<Vec<i64>> {
        let frame = self.admit_parts_stored(&space(), &rel(relation), &[part])?;
        let Some(frame) = frame.get(&part) else {
            return Ok(Vec::new());
        };
        Ok(frame.rows()?.rows)
    }
}

/// The space of the store's verified worktree files.
fn space() -> [u8; 32] {
    Sha256::digest(b"rerail.space/v1\0verified worktree files\0").into()
}

/// A relation of this space, its identity the digest of its name and columns.
fn rel((name, columns): (&str, &str)) -> Rel {
    let mut identity = Sha256::new();
    identity.update(b"rerail.relation/v1\0");
    identity.update(name.as_bytes());
    identity.update(b"\0");
    identity.update(columns.as_bytes());
    Rel {
        name: name.to_owned(),
        ident: identity.finalize().into(),
    }
}

/// The part of `verified_worktree_file` that holds the files of the worktree whose root is
/// `worktree`: positive, since part 0 is a whole relation's and a negative part a delta's.
fn worktree_part(worktree: &str) -> i64 {
    let mut digest = Sha256::new();
    digest.update(b"worktree\0");
    digest.update(worktree.as_bytes());
    (first_eight(digest.finalize().into()) & i64::MAX).max(1)
}

/// A SHA-256's first eight bytes, as a big-endian integer.
fn first_eight(digest: [u8; 32]) -> i64 {
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    i64::from_be_bytes(bytes)
}

/// A frame of `rows` (`width` integers each) for `relation`'s part `part`, its digest that of
/// its rows.
fn put(relation: (&str, &str), part: i64, rows: Vec<i64>, width: usize) -> Put {
    let digest = rows
        .iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |digest, value| {
            (digest ^ value.cast_unsigned()).wrapping_mul(0x0100_0000_01b3)
        });
    Put {
        rel: rel(relation),
        part,
        digest,
        reads: Vec::new(),
        rows: Rows {
            types: vec![INTEGER; width],
            n: rows.len() / width,
            rows,
        },
    }
}

/// A file as its row: its path key, volume, file id in two halves, change time, blob's algorithm
/// and four lanes, and when it was verified.
fn file_row(file: &VerifiedWorktreeFile, verified: i64) -> [i64; FILE_WIDTH] {
    let (low, high) = file.file_id.split_at(8);
    let half = |bytes: &[u8]| i64::from_be_bytes(bytes.try_into().expect("eight bytes"));
    let lanes = file.blob.lanes;
    [
        file.path,
        file.volume.cast_signed(),
        half(low),
        half(high),
        file.changed,
        file.blob.algorithm,
        lanes[0],
        lanes[1],
        lanes[2],
        lanes[3],
        verified,
    ]
}

/// The file a row of `verified_worktree_file` remembers.
fn verified_file(row: &[i64]) -> VerifiedWorktreeFile {
    let mut file_id = [0_u8; 16];
    file_id[..8].copy_from_slice(&row[2].to_be_bytes());
    file_id[8..].copy_from_slice(&row[3].to_be_bytes());
    VerifiedWorktreeFile {
        path: row[0],
        volume: row[1].cast_unsigned(),
        file_id,
        changed: row[4],
        blob: HashKey {
            algorithm: row[5],
            lanes: [row[6], row[7], row[8], row[9]],
        },
    }
}
