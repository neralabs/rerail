//! Clean worktree files whose bytes a restore hashed equal to their committed blob after their
//! change time moved, remembered with the volume, file id and change time they had then: a later
//! restore takes such a file without a read while it keeps all three
//! (`git_workspace::VerifiedFiles`).
//!
//! They are the rows of two relations in one space of the store's frames, written by the store's
//! writer (`queue_frames`), with no table of their own. Whether a row still proves its file is
//! decided in Rust, by `git_workspace::VerifiedFiles`. Each worktree has its own part of each
//! relation: its files in `verified_worktree_file`, and when a restore last wrote them in
//! `verified_worktree`. A save writes only its own worktree's parts, so two saves of two
//! worktrees, from one process or two, keep each other's, and it forgets the parts of worktrees
//! no restore wrote for 30 days, found among the parts the store holds.

use super::frames::{Admitted, Put, Rel, Rows};
use super::{ContentStore, HashKey, MAX_COLLECTION_BATCH, StoreResult};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::sync::Arc;

/// How long a remembered file proves its file without a read, in seconds: an older row proves
/// nothing, so the file is read once more and its row written again, and the parts of a worktree
/// no restore writes for this long go at another worktree's save.
const VERIFIED_FILE_SECONDS: i64 = 30 * 24 * 60 * 60;

/// A relation of this space: its name and its columns, every one an integer. Its identity is the
/// digest of both, and its rows are as wide as its columns, so a change of the columns is another
/// relation, whose rows are hashed again.
struct Relation {
    name: &'static str,
    columns: &'static [&'static str],
}

const FILES: Relation = Relation {
    name: "verified_worktree_file",
    columns: &[
        "path key",
        "volume",
        "file id bytes 0 to 7",
        "file id bytes 8 to 15",
        "change time",
        "blob algorithm",
        "blob lane 0",
        "blob lane 1",
        "blob lane 2",
        "blob lane 3",
        "verified at",
    ],
};
const WORKTREES: Relation = Relation {
    name: "verified_worktree",
    columns: &["written at"],
};

/// The columns of a `verified_worktree_file` row.
const FILE_WIDTH: usize = FILES.columns.len();

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
    /// The files of the worktree whose root is `worktree` that restores remembered within
    /// `VERIFIED_FILE_SECONDS` before `now` (Unix seconds).
    pub fn verified_worktree_files(
        &self,
        worktree: &str,
        now: i64,
    ) -> StoreResult<Vec<VerifiedWorktreeFile>> {
        let rows = self.verified_rows(worktree_part(worktree), now - VERIFIED_FILE_SECONDS)?;
        Ok(rows.into_values().map(|(file, _)| file).collect())
    }

    /// Remembers `files` of the worktree whose root is `worktree`, verified at `now` (Unix
    /// seconds), each replacing the row of its path. The worktree's rows older than
    /// `VERIFIED_FILE_SECONDS` are forgotten, and so are the parts of at most
    /// `MAX_COLLECTION_BATCH` worktrees not written for that long. Handed to the store's writer:
    /// the restore does not wait for the write.
    pub fn remember_verified_worktree_files(
        &self,
        worktree: &str,
        files: &[VerifiedWorktreeFile],
        now: i64,
    ) -> StoreResult<()> {
        let part = worktree_part(worktree);
        let since = now - VERIFIED_FILE_SECONDS;
        let mut kept = self.verified_rows(part, since)?;
        kept.extend(files.iter().map(|file| (file.path, (file.clone(), now))));
        let files_rows = kept.values().flat_map(|(file, at)| file_row(file, *at));
        let puts = vec![
            put(&FILES, part, files_rows.collect()),
            put(&WORKTREES, part, vec![now]),
        ];
        let old = self.old_worktrees(part, since)?;
        let gone = old
            .into_iter()
            .flat_map(|old| [(rel(&FILES), old), (rel(&WORKTREES), old)])
            .collect();
        self.queue_frames(space(), puts, gone, None);
        Ok(())
    }

    /// The rows of part `part` verified at `since` or later, by path key, each with when it was
    /// verified.
    fn verified_rows(
        &self,
        part: i64,
        since: i64,
    ) -> StoreResult<BTreeMap<i64, (VerifiedWorktreeFile, i64)>> {
        let rows = self.stored_rows(&FILES, part)?;
        Ok(rows
            .chunks_exact(FILE_WIDTH)
            .filter(|row| row[FILE_WIDTH - 1] >= since)
            .map(|row| (row[0], (verified_file(row), row[FILE_WIDTH - 1])))
            .collect())
    }

    /// The parts of at most `MAX_COLLECTION_BATCH` worktrees other than `part`'s that the store
    /// holds and no restore wrote at `since` or later. A part whose write time cannot be read
    /// (its frame's bytes are gone) is among them: no restore can read its files either.
    fn old_worktrees(&self, part: i64, since: i64) -> StoreResult<Vec<i64>> {
        let worktrees = rel(&WORKTREES);
        let parts = self.stored_frame_parts(&space(), &worktrees)?;
        let written = self.admit_parts_stored(&space(), &worktrees, &parts)?;
        let old = |other: &&i64| **other != part && written_at(written.get(other)) < since;
        Ok(parts
            .iter()
            .filter(old)
            .take(MAX_COLLECTION_BATCH)
            .copied()
            .collect())
    }

    /// The rows of `relation`'s part `part`, flat, as queued or stored: none when it has no
    /// frame there or its bytes cannot be read.
    fn stored_rows(&self, relation: &Relation, part: i64) -> StoreResult<Vec<i64>> {
        let frame = self.admit_parts_stored(&space(), &rel(relation), &[part])?;
        let Some(frame) = frame.get(&part) else {
            return Ok(Vec::new());
        };
        Ok(frame.rows()?.rows)
    }
}

/// When a restore last wrote a worktree's files, as its `verified_worktree` frame holds it: the
/// earliest time there is where it has no frame whose rows can be read.
fn written_at(frame: Option<&Arc<Admitted>>) -> i64 {
    let rows = frame.and_then(|frame| frame.rows().ok());
    let first = rows.and_then(|rows| rows.rows.first().copied());
    first.unwrap_or(i64::MIN)
}

/// The space of the store's verified worktree files.
fn space() -> [u8; 32] {
    Sha256::digest(b"rerail.space/v1\0verified worktree files\0").into()
}

/// A relation of this space, its identity the digest of its name and columns.
fn rel(relation: &Relation) -> Rel {
    let mut identity = Sha256::new();
    identity.update(b"rerail.relation/v1\0");
    identity.update(relation.name.as_bytes());
    for column in relation.columns {
        identity.update(b"\0");
        identity.update(column.as_bytes());
    }
    Rel {
        name: relation.name.to_owned(),
        ident: identity.finalize().into(),
    }
}

/// The part of each relation that holds the worktree whose root is `worktree`: positive, since
/// part 0 is a whole relation's and a negative part a delta's.
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

/// A frame of `rows` for `relation`'s part `part`, as wide as its columns, its digest that of its
/// rows.
fn put(relation: &Relation, part: i64, rows: Vec<i64>) -> Put {
    let width = relation.columns.len();
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
