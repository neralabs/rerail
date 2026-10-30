//! Rust-only content-addressed storage for workspace source and syntax tapes.
//!
//! JavaScript does not receive connections, SQL, row identifiers, or hash lanes.
//! The code-rule runtime will call this module through coarse Rust operations.

pub(crate) mod atoms;
// Publication of the prefilter is wired into the scan in a later slice; until then only tests call it.
#[allow(dead_code)]
mod bounded_insert;
mod compression;
mod connection;
mod derivation;
mod facts_memo;
mod objects;
mod postings;
/// G's positional search index (value and text streams), for G's literal anchors (O21).
#[cfg(test)]
pub(crate) mod positional_search;
pub(crate) mod vocabulary;
#[allow(unused_imports)]
pub(crate) use postings::append_delta;
#[allow(dead_code)]
pub(crate) mod prefilter;
pub(crate) use prefilter::TapePrefilter;
mod publication;
mod read_cache;
mod reclamation;
mod relationships;
mod results;
mod rows;
mod schema;
mod search_index;
mod snapshots;
mod tree_diff;
mod tree_pages;
/// G's tree result segments as copied, beside `tree_pages` (O19).
#[cfg(test)]
#[allow(dead_code)]
mod tree_result_segments;
#[cfg(test)]
mod tests;
mod workspaces;

pub(crate) use compression::compress;
pub(crate) use connection::BusyRetries;
pub(crate) use connection::OperationDeadline;
pub use atoms::Sym;
pub use derivation::{ABSENT, Binding, Derivation, DerivationInput, DerivationRecord, StoredDerivation};
pub use facts_memo::{FactsCompletion, FactsMemo, FactsMemoKey, FactsPlanSet, FactsResult};
pub use schema::STORE_FILE_NAME;
pub use tree_diff::{TreeChange, TreeDiff};
pub use tree_pages::{TREE_PAGE_FILES, TreePage, TreePageKey, tree_pages_digest};

#[cfg(test)]
pub(crate) use connection::{
    BUSY_ATTEMPT_HOOK, BUSY_DEADLINE_OVERRIDE, BUSY_TIMEOUT_OVERRIDE, BUSY_WAIT_HOOK,
    operation_waited,
};
#[cfg(test)]
pub(crate) use snapshots::AFTER_GIT_TREE_TRANSACTION;

use connection::{ThreadConnection, retry_busy};
use read_cache::{CachedBacking, DecompressedReadCache};
use results::LocalResultCache;
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest as _, Sha256};
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

const HASH_SHA256: i64 = 1;
const HASH_GIT_SHA1: i64 = 2;
const HASH_GIT_SHA256: i64 = 3;
/// The facts engine's content key (blake3), stored only as a source alias.
const HASH_BLAKE3: i64 = 4;
const CODEC_ZSTD: i64 = 1;
const STRING_PATH: i64 = 1;
const STRING_REPOSITORY: i64 = 2;
const STRING_OWNER: i64 = 3;
const STRING_PATH_COMPONENT: i64 = 4;
const STRING_CLAUSE_KEY: i64 = 5;
/// Fact names (`derivation.rs`): bound names and derivation outputs.

const DEFAULT_COMPRESSION_LEVEL: i32 = 3;
const MAX_PUBLICATION_BATCH: usize = 64;
/// Largest `collect` batch: the per-statement row limit of one collection transaction.
pub const MAX_COLLECTION_BATCH: usize = 256;
const DEFAULT_READ_CACHE_BYTES: usize = 32 * 1024 * 1024;
const DEFAULT_DECOMPRESSED_CACHE_TTL: Duration = Duration::from_secs(30);

#[cfg(test)]
thread_local! {
    /// Seconds this thread's store clock runs ahead of the system clock, so tests can let time
    /// pass inside an operation.
    pub(crate) static CLOCK_OFFSET: std::cell::Cell<i64> = const { std::cell::Cell::new(0) };
}

/// The store's clock: whole seconds since the Unix epoch. Leases are measured against it.
pub(crate) fn unix_seconds() -> StoreResult<i64> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| StoreError::Invalid("system clock is before the Unix epoch".to_owned()))?
        .as_secs();
    let seconds = i64::try_from(seconds)
        .map_err(|_| StoreError::Invalid("system clock exceeds content-store range".to_owned()))?;
    #[cfg(test)]
    let seconds = seconds + CLOCK_OFFSET.get();
    Ok(seconds)
}

#[cfg(test)]
static SEARCH_INDEX_FAMILY_WRITES: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
pub(crate) fn reset_search_index_family_writes() {
    SEARCH_INDEX_FAMILY_WRITES.store(0, Ordering::Relaxed);
}

#[cfg(test)]
pub(crate) fn search_index_family_writes() -> usize {
    SEARCH_INDEX_FAMILY_WRITES.load(Ordering::Relaxed)
}

#[cfg(test)]
thread_local! {
    static WRITE_LOCK_HELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static WRITE_LOCK_SECTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static HASHES_UNDER_WRITE_LOCK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Marks this thread as holding a publication write transaction until dropped.
#[cfg(test)]
pub(super) struct WriteLockSection;

#[cfg(test)]
impl WriteLockSection {
    pub(super) fn enter() -> Self {
        WRITE_LOCK_HELD.with(|held| held.set(true));
        WRITE_LOCK_SECTIONS.with(|sections| sections.set(sections.get() + 1));
        Self
    }
}

#[cfg(test)]
impl Drop for WriteLockSection {
    fn drop(&mut self) {
        WRITE_LOCK_HELD.with(|held| held.set(false));
    }
}

/// Returns `(write sections entered, SHA-256 digests computed inside them)` on this thread
/// since the last call, and resets both.
#[cfg(test)]
pub(super) fn take_write_lock_hash_counts() -> (usize, usize) {
    (
        WRITE_LOCK_SECTIONS.with(|sections| sections.replace(0)),
        HASHES_UNDER_WRITE_LOCK.with(|hashes| hashes.replace(0)),
    )
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HashKey {
    algorithm: i64,
    lanes: [i64; 4],
}

impl HashKey {
    pub(crate) fn is_git_sha256(self) -> bool {
        self.algorithm == HASH_GIT_SHA256
    }

    pub(crate) fn sha256_bytes(self) -> Option<[u8; 32]> {
        (self.algorithm == HASH_SHA256).then(|| {
            let mut digest = [0_u8; 32];
            for (chunk, lane) in digest.chunks_exact_mut(8).zip(self.lanes) {
                chunk.copy_from_slice(&lane.to_be_bytes());
            }
            digest
        })
    }

    pub fn sha256(bytes: &[u8]) -> Self {
        #[cfg(test)]
        if WRITE_LOCK_HELD.with(std::cell::Cell::get) {
            HASHES_UNDER_WRITE_LOCK.with(|hashes| hashes.set(hashes.get() + 1));
        }
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        Self::from_sha256_digest(digest)
    }

    pub(crate) fn from_sha256_digest(digest: [u8; 32]) -> Self {
        Self::from_padded(HASH_SHA256, digest)
    }

    pub fn git_sha1(bytes: [u8; 20]) -> Self {
        let mut padded = [0_u8; 32];
        padded[..20].copy_from_slice(&bytes);
        Self::from_padded(HASH_GIT_SHA1, padded)
    }

    /// A Git SHA-1 id's twenty bytes.
    pub(crate) fn git_sha1_bytes(self) -> Option<[u8; 20]> {
        (self.algorithm == HASH_GIT_SHA1).then(|| {
            let mut bytes = [0_u8; 32];
            for (chunk, lane) in bytes.chunks_exact_mut(8).zip(self.lanes) {
                chunk.copy_from_slice(&lane.to_be_bytes());
            }
            bytes[..20].try_into().expect("twenty bytes")
        })
    }

    pub fn git_sha256(bytes: [u8; 32]) -> Self {
        Self::from_padded(HASH_GIT_SHA256, bytes)
    }

    fn from_padded(algorithm: i64, bytes: [u8; 32]) -> Self {
        let mut lanes = [0_i64; 4];
        for (index, chunk) in bytes.chunks_exact(8).enumerate() {
            lanes[index] = i64::from_be_bytes(chunk.try_into().expect("eight-byte hash lane"));
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
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(error) => write!(formatter, "content store database error: {error}"),
            Self::Io(error) => write!(formatter, "content store I/O error: {error}"),
            Self::Invalid(message) => write!(formatter, "invalid content store input: {message}"),
            Self::Corrupt(message) => write!(formatter, "corrupt content store: {message}"),
            Self::Busy => formatter.write_str("content store writer is busy"),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct SourceId(i64);
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct TapeId(i64);

impl TapeId {
    /// The row id, for SQL over the store's tables (the prefilter's lowering).
    pub(crate) fn row(self) -> i64 {
        self.0
    }
}

/// A stored tape whose search index is not built yet, with what building it reads.
pub(crate) struct UnindexedTape {
    pub tape: TapeId,
    /// The workspace's grammar id (`workspace::grammar_id`, or a derived document's
    /// `workspace::document_grammar_id`).
    pub grammar: i64,
    pub source: Vec<u8>,
    /// The decoded tape image.
    pub image: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TapeSearchIndex {
    pub tape_id: TapeId,
    pub index_version: i64,
    pub node_count: u32,
    pub has_errors: bool,
    pub kind_tokens: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SearchKindLookup {
    pub tape_id: TapeId,
    pub index_version: i64,
    pub kind_tokens: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StoredKindPresence {
    Unknown,
    Present,
    Absent,
    SyntaxErrors,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnapshotId(i64);
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct WorkspaceId(i64);

/// The workspace a snapshot publication registers over the snapshot it records, keyed by `root`.
#[derive(Clone, Copy, Debug)]
pub struct WorkspaceRegistration<'a> {
    pub root: &'a str,
    pub repository: &'a str,
    pub owner: &'a str,
    pub generation: i64,
    /// How long the lease lasts, measured from the registration itself: publication before it
    /// may take any time.
    pub lease_seconds: i64,
}

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

    fn key(self) -> HashKey {
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
    pub compressed_sources: usize,
    pub compressed_tapes: usize,
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
    pub raw_length: usize,
    pub frame: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct StoredLocalResult {
    pub candidate: CandidateClause,
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

pub struct CachedBytes {
    backing: CachedBacking,
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

pub struct ContentStore {
    database_path: PathBuf,
    connection: ThreadConnection,
    compression_level: i32,
    decompressed_cache: Mutex<DecompressedReadCache>,
    local_result_cache: Mutex<LocalResultCache>,
    /// Facts plan sets by digest (immutable once written: `facts_memo`).
    facts_plan_sets: parking_lot::Mutex<HashMap<[u8; 32], std::sync::Arc<[[u8; 32]]>>>,
    /// The mirror of the `atoms` table (atoms.rs).
    atoms: parking_lot::Mutex<atoms::Interner>,
    #[cfg(test)]
    git_tree_lookup_count: AtomicUsize,
    #[cfg(test)]
    git_tree_write_count: AtomicUsize,
    #[cfg(test)]
    git_tree_transactions: AtomicUsize,
}

impl ContentStore {
    pub fn open(path: &Path) -> StoreResult<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_FULL_MUTEX;
        let mut connection = Connection::open_with_flags(path, flags)?;
        // Only creating or migrating the schema takes the write lock (`initialize`).
        retry_busy(&mut BusyRetries::default(), || {
            Self::initialize(&mut connection)
        })?;
        Ok(Self {
            database_path: path.to_path_buf(),
            connection: ThreadConnection {
                database_path: path.to_path_buf(),
            },
            compression_level: DEFAULT_COMPRESSION_LEVEL,
            decompressed_cache: Mutex::new(DecompressedReadCache::new(
                DEFAULT_READ_CACHE_BYTES,
                DEFAULT_DECOMPRESSED_CACHE_TTL,
            )),
            local_result_cache: Mutex::new(LocalResultCache::new(DEFAULT_READ_CACHE_BYTES)),
            facts_plan_sets: parking_lot::Mutex::new(HashMap::new()),
            atoms: parking_lot::Mutex::new(atoms::Interner::default()),
            #[cfg(test)]
            git_tree_lookup_count: AtomicUsize::new(0),
            #[cfg(test)]
            git_tree_write_count: AtomicUsize::new(0),
            #[cfg(test)]
            git_tree_transactions: AtomicUsize::new(0),
        })
    }

    /// Drop process-local read caches and close every thread's idle connection to the store, with
    /// its SQLite page and statement caches. Call this only when a workspace has released its
    /// resident documents: an active apply benefits from the caches, while an inactive one should
    /// not retain an arena or SQLite page cache. Connections reopen on the next read.
    pub fn release_memory(&self) -> StoreResult<()> {
        self.decompressed_cache
            .lock()
            .expect("decompressed cache mutex poisoned")
            .clear();
        self.local_result_cache
            .lock()
            .expect("local result cache mutex poisoned")
            .clear();
        self.connection.release_all();
        Ok(())
    }
}
