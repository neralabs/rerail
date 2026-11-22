//! The content store's base (rerail-store-base): its keys, ids, errors and payloads, the constants its tables
//! read, and the connection's cached reads (`Cached`, `flush_of`). The store's base modules read them (the atoms,
//! the segments, the rows, the journal), and so does everything above: the observer and git (rerail-observe), and
//! the store itself (rerail-store, `mod.rs`), which re-exports them where they were (`crate::content_store::*`).
use super::SegmentBytes;
use rusqlite::Connection;
use rerail_blocks::block::Flush;
use sha2::{Digest as _, Sha256};
use std::collections::HashMap;
use std::fmt;
#[cfg(any(test, feature = "test-hooks"))]
use std::sync::atomic::{AtomicUsize, Ordering};

pub const HASH_SHA256: i64 = 1;
pub const HASH_GIT_SHA1: i64 = 2;
pub const HASH_GIT_SHA256: i64 = 3;
/// The facts engine's content key (blake3), stored only as a source alias.
pub const HASH_BLAKE3: i64 = 4;
pub const CODEC_ZSTD: i64 = 1;
pub const STRING_PATH: i64 = 1;
pub const STRING_REPOSITORY: i64 = 2;
pub const STRING_OWNER: i64 = 3;
pub const STRING_PATH_COMPONENT: i64 = 4;
pub const STRING_CLAUSE_KEY: i64 = 5;

pub const DEFAULT_COMPRESSION_LEVEL: i32 = 3;
pub const MAX_PUBLICATION_BATCH: usize = 64;
/// Largest `collect` batch: the per-statement row limit of one collection transaction.
pub const MAX_COLLECTION_BATCH: usize = 256;
pub const DEFAULT_READ_CACHE_BYTES: usize = 32 * 1024 * 1024;

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    /// Seconds this thread's store clock runs ahead of the system clock, so tests can let time
    /// pass inside an operation.
    pub static CLOCK_OFFSET: std::cell::Cell<i64> = const { std::cell::Cell::new(0) };
}

/// The store's clock: whole seconds since the Unix epoch. Leases are measured against it.
pub fn unix_seconds() -> StoreResult<i64> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| StoreError::Invalid("system clock is before the Unix epoch".to_owned()))?
        .as_secs();
    let seconds = i64::try_from(seconds)
        .map_err(|_| StoreError::Invalid("system clock exceeds content-store range".to_owned()))?;
    #[cfg(any(test, feature = "test-hooks"))]
    let seconds = seconds + CLOCK_OFFSET.get();
    Ok(seconds)
}

#[cfg(any(test, feature = "test-hooks"))]
pub static SEARCH_INDEX_FAMILY_WRITES: AtomicUsize = AtomicUsize::new(0);

#[cfg(any(test, feature = "test-hooks"))]
pub fn reset_search_index_family_writes() {
    SEARCH_INDEX_FAMILY_WRITES.store(0, Ordering::Relaxed);
}


#[cfg(any(test, feature = "test-hooks"))]
pub fn search_index_family_writes() -> usize {
    SEARCH_INDEX_FAMILY_WRITES.load(Ordering::Relaxed)
}

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    static WRITE_LOCK_HELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static WRITE_LOCK_SECTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static HASHES_UNDER_WRITE_LOCK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Marks this thread as holding a publication write transaction until dropped.
#[cfg(any(test, feature = "test-hooks"))]
pub struct WriteLockSection;

#[cfg(any(test, feature = "test-hooks"))]
impl WriteLockSection {
    pub fn enter() -> Self {
        WRITE_LOCK_HELD.with(|held| held.set(true));
        WRITE_LOCK_SECTIONS.with(|sections| sections.set(sections.get() + 1));
        Self
    }
}

#[cfg(any(test, feature = "test-hooks"))]
impl Drop for WriteLockSection {
    fn drop(&mut self) {
        WRITE_LOCK_HELD.with(|held| held.set(false));
    }
}

/// Returns `(write sections entered, SHA-256 digests computed inside them)` on this thread
/// since the last call, and resets both.
#[cfg(any(test, feature = "test-hooks"))]
pub fn take_write_lock_hash_counts() -> (usize, usize) {
    (
        WRITE_LOCK_SECTIONS.with(|sections| sections.replace(0)),
        HASHES_UNDER_WRITE_LOCK.with(|hashes| hashes.replace(0)),
    )
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, serde::Serialize, serde::Deserialize)]
pub struct HashKey {
    pub algorithm: i64,
    pub lanes: [i64; 4],
}

impl HashKey {
    pub fn is_git_sha256(self) -> bool {
        self.algorithm == HASH_GIT_SHA256
    }

    pub fn sha256_bytes(self) -> Option<[u8; 32]> {
        (self.algorithm == HASH_SHA256).then(|| {
            let mut digest = [0_u8; 32];
            for (chunk, lane) in digest.as_chunks_mut::<8>().0.iter_mut().zip(self.lanes) {
                *chunk = lane.to_be_bytes();
            }
            digest
        })
    }

    pub fn sha256(bytes: &[u8]) -> Self {
        #[cfg(any(test, feature = "test-hooks"))]
        if WRITE_LOCK_HELD.with(std::cell::Cell::get) {
            HASHES_UNDER_WRITE_LOCK.with(|hashes| hashes.set(hashes.get() + 1));
        }
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        Self::from_sha256_digest(digest)
    }

    /// The identity of a tape of the source `source`: its bytes and its source's hash. A
    /// named-only tape holds no source text, so two sources of one shape and one set of token
    /// lengths store the same bytes; each is its own tape, as it carries its own source.
    pub fn tape_identity(source: Self, tape: &[u8]) -> Self {
        #[cfg(any(test, feature = "test-hooks"))]
        if WRITE_LOCK_HELD.with(std::cell::Cell::get) {
            HASHES_UNDER_WRITE_LOCK.with(|hashes| hashes.set(hashes.get() + 1));
        }
        let mut hash = Sha256::new();
        hash.update(b"rerail.tape-of/v1\0");
        hash.update(source.algorithm.to_le_bytes());
        for lane in source.lanes {
            hash.update(lane.to_le_bytes());
        }
        hash.update(tape);
        Self::from_sha256_digest(hash.finalize().into())
    }

    pub fn from_sha256_digest(digest: [u8; 32]) -> Self {
        Self::from_padded(HASH_SHA256, digest)
    }

    pub fn git_sha1(bytes: [u8; 20]) -> Self {
        let mut padded = [0_u8; 32];
        padded[..20].copy_from_slice(&bytes);
        Self::from_padded(HASH_GIT_SHA1, padded)
    }

    /// A Git SHA-1 id's twenty bytes.
    pub fn git_sha1_bytes(self) -> Option<[u8; 20]> {
        (self.algorithm == HASH_GIT_SHA1).then(|| {
            let mut bytes = [0_u8; 32];
            for (chunk, lane) in bytes.as_chunks_mut::<8>().0.iter_mut().zip(self.lanes) {
                *chunk = lane.to_be_bytes();
            }
            bytes[..20].try_into().expect("twenty bytes")
        })
    }

    /// The key as bytes: its algorithm, then its lanes (an alias table's key).
    pub fn key_bytes(self) -> [u8; 40] {
        let mut out = [0_u8; 40];
        out[..8].copy_from_slice(&self.algorithm.to_be_bytes());
        for (chunk, lane) in out[8..].as_chunks_mut::<8>().0.iter_mut().zip(self.lanes) {
            *chunk = lane.to_be_bytes();
        }
        out
    }

    pub fn git_sha256(bytes: [u8; 32]) -> Self {
        Self::from_padded(HASH_GIT_SHA256, bytes)
    }

    pub fn from_padded(algorithm: i64, bytes: [u8; 32]) -> Self {
        let mut lanes = [0_i64; 4];
        for (index, chunk) in bytes.as_chunks::<8>().0.iter().enumerate() {
            lanes[index] = i64::from_be_bytes(*chunk);
        }
        Self { algorithm, lanes }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitTreeEntry {
    pub name: String,
    pub mode: u16,
    pub target: HashKey,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitTree {
    pub id: HashKey,
    pub prefix: String,
    pub entries: Vec<GitTreeEntry>,
}

#[derive(Debug)]
pub enum StoreError {
    Database(rusqlite::Error),
    Io(std::io::Error),
    Invalid(String),
    Corrupt(String),
    /// Another writer holds the store. A write's attempts are retried until the write deadline
    /// (`connection::retry_busy`; one deadline for all the writes of an `OperationDeadline`), so
    /// a write returns it only once that passed. The store is a cache, so a caller may proceed
    /// without it.
    Busy,
    /// A row a publication had resolved vanished before the publication used it, under a
    /// concurrent collection. Never a reason to proceed without the store.
    Vanished(&'static str),
    /// A deletion or stop of an analysis scope that a running analysis holds (`scope_lease`, decision 0547): retry
    /// once that run has finished.
    InUse(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(error) => write!(formatter, "content store database error: {error}"),
            Self::Io(error) => write!(formatter, "content store I/O error: {error}"),
            Self::Invalid(message) => write!(formatter, "invalid content store input: {message}"),
            Self::Corrupt(message) => write!(formatter, "corrupt content store: {message}"),
            Self::Busy => formatter.write_str("content store writer is busy"),
            Self::InUse(message) => write!(formatter, "analysis scope in use: {message}"),
            Self::Vanished(row) => write!(
                formatter,
                "content store {row} vanished under a concurrent collection"
            ),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        if matches!(
            error.sqlite_error_code(),
            Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
        ) {
            Self::Busy
        } else {
            Self::Database(error)
        }
    }
}

impl From<std::io::Error> for StoreError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

pub type StoreResult<T> = std::result::Result<T, StoreError>;

/// Serialized as its number (a run's record names a source this store holds by it, decision 0306).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct SourceId(pub i64);

impl SourceId {
    /// The id as a number (a key's bytes, a test's check).
    pub fn raw(self) -> i64 {
        self.0
    }
    /// An id a record of this store kept (`raw`; a kept statistics blob, decision 0096 §2). A source collected since reads as absent: a lookup finds nothing.
    pub fn from_raw(id: i64) -> Self {
        SourceId(id)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct TapeId(pub i64);

impl TapeId {
    /// The row id, for SQL over the store's tables (the prefilter's lowering).
    pub fn row(self) -> i64 {
        self.0
    }
}


#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TapeSearchIndex {
    pub tape_id: TapeId,
    pub index_version: i64,
    pub node_count: u32,
    pub has_errors: bool,
    pub kind_tokens: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchKindLookup {
    pub tape_id: TapeId,
    pub index_version: i64,
    pub kind_tokens: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoredKindPresence {
    Unknown,
    Present,
    Absent,
    SyntaxErrors,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnapshotId(pub i64);
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct WorkspaceId(pub i64);


#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CandidateCatalogIdentity {
    code_rule_catalog: HashKey,
    grammar_set: HashKey,
    scanner_version: i64,
    tape_format_version: i64,
}

impl CandidateCatalogIdentity {
    pub fn new(
        code_rule_catalog: HashKey,
        grammar_set: HashKey,
        scanner_version: i64,
        tape_format_version: i64,
    ) -> StoreResult<Self> {
        if scanner_version < 0 || tape_format_version < 0 {
            return Err(StoreError::Invalid(
                "candidate catalog versions must be nonnegative".to_owned(),
            ));
        }
        Ok(Self {
            code_rule_catalog,
            grammar_set,
            scanner_version,
            tape_format_version,
        })
    }

    pub fn key(self) -> HashKey {
        let mut bytes = Vec::with_capacity(96);
        for key in [self.code_rule_catalog, self.grammar_set] {
            bytes.extend_from_slice(&key.algorithm.to_be_bytes());
            for lane in key.lanes {
                bytes.extend_from_slice(&lane.to_be_bytes());
            }
        }
        bytes.extend_from_slice(&self.scanner_version.to_be_bytes());
        bytes.extend_from_slice(&self.tape_format_version.to_be_bytes());
        HashKey::sha256(&bytes)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CandidateClause {
    pub rule: HashKey,
    pub clause: String,
}

#[derive(Clone, Copy, Debug)]
pub struct TapePayload<'a> {
    pub grammar_id: i64,
    pub grammar_version: i64,
    pub tape_format_version: i64,
    pub bytes: &'a [u8],
    /// The documents derived from this tape's (`file_documents`), in ordinal order from 1: each
    /// is stored as a source and a tape of its own, with this tape's grammar version and format,
    /// in the transaction that stores this tape. Empty for a single-language file.
    pub documents: &'a [DocumentPayload<'a>],
}

/// A document derived from a file's primary one, published with the primary's tape.
#[derive(Clone, Copy, Debug)]
pub struct DocumentPayload<'a> {
    /// The document it was derived from: 0 the primary, `i` the `i`-th derived one.
    pub parent_ordinal: i64,
    /// What the document is to the file (`tape::DocumentRole::code`); never 0, the primary's.
    pub role_id: i64,
    /// The embedding that built it (`tape::via_code`).
    pub via_id: i64,
    /// Its text, stored as an ordinary source.
    pub source: &'a [u8],
    /// Its tape's grammar id: unique to derived documents (`workspace::document_grammar_id`).
    pub grammar_id: i64,
    pub tape: &'a [u8],
    pub source_map: &'a [u8],
    pub host_errors: &'a [u8],
    /// The parent's bytes it stands at, where they are not its text's (`tape::DerivedImage::at`).
    pub at: Option<(u32, u32)>,
}

/// A document derived from a file's, queued for the store's writer with the file's tape (decision 0431 step 6): a
/// `DocumentPayload` that owns its bytes until written.
#[derive(Clone, Debug)]
pub struct QueuedDocument {
    pub parent_ordinal: i64,
    pub role_id: i64,
    pub via_id: i64,
    pub source: std::sync::Arc<String>,
    pub grammar_id: i64,
    pub tape: Vec<u8>,
    pub source_map: Vec<u8>,
    pub host_errors: Vec<u8>,
    pub at: Option<(u32, u32)>,
}

impl QueuedDocument {
    /// Its payload, borrowing its bytes.
    pub fn payload(&self) -> DocumentPayload<'_> {
        DocumentPayload {
            parent_ordinal: self.parent_ordinal,
            role_id: self.role_id,
            via_id: self.via_id,
            source: self.source.as_bytes(),
            grammar_id: self.grammar_id,
            tape: &self.tape,
            source_map: &self.source_map,
            host_errors: &self.host_errors,
            at: self.at,
        }
    }
}

/// A `file_documents` row: a document derived from a stored primary tape. `source` and
/// `grammar_id` are its tape row's, `None` where that row is missing: the document is then not
/// stored, and the file is not either.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredDocument {
    pub ordinal: i64,
    pub parent_ordinal: i64,
    pub role_id: i64,
    pub via_id: i64,
    pub tape: TapeId,
    pub source: Option<SourceId>,
    pub grammar_id: Option<i64>,
    pub source_map: Vec<u8>,
    pub host_errors: Vec<u8>,
    pub at: Option<(u32, u32)>,
}



#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoredFile {
    pub source: SourceId,
    pub tape: Option<TapeId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicationResult {
    pub files: Vec<StoredFile>,
    pub retries: usize,
    pub retry_wait_nanoseconds: u64,
    pub transaction_nanoseconds: u64,
    pub object_transaction_nanoseconds: u64,
    pub relationship_transaction_nanoseconds: u64,
    pub read_data_statements: usize,
    pub data_statements: usize,
    pub max_data_statements_per_pass: usize,
    pub transaction_statements: usize,
    pub reconciliation_passes: usize,
    pub reused_sources: usize,
    pub reused_tapes: usize,
    pub written_sources: usize,
    pub written_tapes: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct SnapshotFilePublication<'a> {
    pub path: &'a str,
    pub source: &'a [u8],
    pub aliases: &'a [HashKey],
    pub tape: Option<TapePayload<'a>>,
    pub catalog: CandidateCatalogIdentity,
    pub candidates: &'a [CandidateClause],
}

#[derive(Clone, Debug)]
pub struct LocalResultPublication {
    pub candidate: CandidateClause,
    pub context: HashKey,
    /// The result's bytes, stored as they are, in a segment (decision 0450 step 2).
    pub bytes: Vec<u8>,
}


#[derive(Clone, Copy, Debug)]
pub struct WorkspaceCompletion {
    pub workspace_id: WorkspaceId,
    pub revision: i64,
    pub completed_at: i64,
}

#[derive(Clone, Debug)]
pub struct DirectoryReuse {
    pub prefix: String,
    pub tree: HashKey,
    pub package_markers: HashKey,
}

#[derive(Clone, Debug)]
pub struct StoredSourceObjects {
    pub source: SourceId,
    pub tapes: HashMap<i64, TapeId>,
}

#[derive(Clone, Debug)]
pub struct StoredPathObjects {
    pub source_hash: HashKey,
    pub objects: StoredSourceObjects,
}

#[derive(Clone, Copy, Debug)]
pub struct OverlayFilePublication<'a> {
    pub path: &'a str,
    pub state_id: i64,
    pub source: &'a [u8],
    pub aliases: &'a [HashKey],
    pub tape: Option<TapePayload<'a>>,
    pub catalog: CandidateCatalogIdentity,
    pub candidates: &'a [CandidateClause],
    pub local_results: &'a [LocalResultPublication],
    pub candidates_current: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CollectionReport {
    pub expired_pins: usize,
    pub retired_workspaces: usize,
    pub snapshot_files: usize,
    pub snapshots: usize,
    pub candidate_rows: usize,
    pub candidate_checks: usize,
    pub candidate_catalogs: usize,
    pub tapes: usize,
    pub sources: usize,
}

/// A stored source's or tape's bytes, read in place in their segment (`read_cache`).
pub struct CachedBytes {
    pub bytes: SegmentBytes,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapacityState {
    Normal,
    Collect,
    RejectWrites,
    OverBudget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoreCapacity {
    pub bytes: u64,
    pub state: CapacityState,
}


/// The flush mode of `connection`'s store, as its `synchronous` says (`configure_connection`): what code that holds
/// only a connection (the atoms, the kept facts) writes its blocks back by. A connection the store did not configure
/// (SQLite's default, `FULL`) reads as `Synced`.
pub fn flush_of(connection: &Connection) -> Flush {
    match connection.query_row("PRAGMA synchronous", [], |row| row.get::<_, i64>(0)) {
        Ok(0) => Flush::None,
        Ok(1) => Flush::Os,
        _ => Flush::Synced,
    }
}

/// `Connection::query_row` and `Connection::execute` through the connection's statement cache (decision 0240): the
/// plain forms parse their SQL at every call, and the store's writer made several such calls a tape (the tape's
/// generation, its source's hash), parsing SQL for each of 27,000 files.
pub trait Cached {
    fn query_row_cached<
        T,
        P: rusqlite::Params,
        F: FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    >(
        &self,
        sql: &str,
        params: P,
        f: F,
    ) -> rusqlite::Result<T>;
    fn execute_cached<P: rusqlite::Params>(&self, sql: &str, params: P) -> rusqlite::Result<usize>;
}

impl Cached for Connection {
    fn query_row_cached<
        T,
        P: rusqlite::Params,
        F: FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    >(
        &self,
        sql: &str,
        params: P,
        f: F,
    ) -> rusqlite::Result<T> {
        self.prepare_cached(sql)?.query_row(params, f)
    }

    fn execute_cached<P: rusqlite::Params>(&self, sql: &str, params: P) -> rusqlite::Result<usize> {
        self.prepare_cached(sql)?.execute(params)
    }
}

/// A file's stat when its source was read: what the stat index keeps of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FileStat {
    pub size: i64,
    pub mtime_ns: i64,
    pub inode: i64,
}

impl FileStat {
    /// The stat of `metadata`.
    #[cfg(unix)]
    pub fn of(metadata: &std::fs::Metadata) -> FileStat {
        use std::os::unix::fs::MetadataExt as _;
        FileStat { size: metadata.len() as i64, mtime_ns: metadata.mtime().saturating_mul(1_000_000_000).saturating_add(metadata.mtime_nsec()), inode: metadata.ino() as i64 }
    }

    /// The stat of `metadata` where there are no inode numbers (Windows, wasm32): size and modification time.
    #[cfg(not(unix))]
    pub fn of(metadata: &std::fs::Metadata) -> FileStat {
        let mtime_ns = metadata.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as i64);
        FileStat { size: metadata.len() as i64, mtime_ns, inode: 0 }
    }
}

/// How long before its recording an entry's mtime must be for the entry to be trusted (git's racy rule, with margin
/// for file systems whose mtime is coarse).
pub const RACY_NS: i64 = 2_000_000_000;

impl HashKey {
    /// A blake3 content hash (the facts engine's content key), stored as a source alias.
    pub fn blake3(bytes: [u8; 32]) -> Self {
        Self::from_padded(super::HASH_BLAKE3, bytes)
    }
}

impl TapeId {
    pub const fn for_benchmark(value: i64) -> Self {
        Self(value)
    }
}
