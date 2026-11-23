//! Rust-only content-addressed storage for workspace source and syntax tapes.
//!
//! JavaScript does not receive connections, SQL, row identifiers, or hash lanes.
//! The code-rule runtime will call this module through coarse Rust operations.

// The store's base and its observer are the crates below (rerail-store-base, rerail-observe): the keys, ids, errors
// and payloads (`base.rs`), the atoms, the segments, the rows, the postings, the vocabulary, compression, the journal,
// the hazards, the scope leases and the private files, and the observer (`observe`), re-exported here.
pub use rerail_observe::content_store::*;
pub mod analysis_scope;
pub mod atoms;
pub use atoms::{ProcessIds, StoreIds, Sym};
#[cfg(target_os = "macos")]
mod accel_models;
// Publication of the prefilter is wired into the scan in a later slice; until then only tests call it.
#[allow(dead_code)]
mod collection_guard;
pub use collection_guard::CollectionLease;
mod connection;
pub use connection::set_connection_functions;
mod corpus_runs;
pub use corpus_runs::process_gone;
pub use corpus_runs::{CorpusKept, CorpusPass, CorpusPolicy};
#[cfg(not(any(windows, target_family = "wasm")))]
pub use connection::sqlite_on_jemalloc;
pub use connection::{flush_of, store_flush};
pub use rerail_blocks::block::Flush;
mod derivation;
mod document_places;
pub use document_places::{DOCUMENT_WORDS, DocumentPlaces, PLACED_WORDS};
mod held;
pub mod kept_facts;
/// The store as the observer and git below it reach it (`observer_store::base::STORE`), registered at load.
pub mod observer_store;
pub mod guest_keys;
mod object_segments;
mod plan_set;
/// The demand's terminal matches' row estimate, a kept fact (t33 step 6).
mod estimates;
mod objects;
pub use objects::{ResidentFrames, TapeFrame};
/// G's positional search index (value and text streams), for G's literal anchors (O21).
#[cfg(any(test, feature = "test-hooks"))]
pub mod positional_search;
#[allow(dead_code)]
pub mod prefilter;
pub use prefilter::TapePrefilter;
mod publication;
mod archive;
pub use archive::{ARCHIVED, place_archived_tape_at};
mod reclamation;
pub use reclamation::Ageing;
pub mod record;
mod relationships;
mod results;
mod schema;
mod search_index;
mod snapshots;
mod tree_diff;
pub mod upkeep;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
mod workspaces;
pub mod write_back;

pub use held::forget_snapshot as forget_held_snapshot;
pub use object_segments::{copy_beside, ObjectBytes, ObjectFrame, ObjectMaps, QueuedTape, TapeBytes, TapeHeld};
pub use held::{Rel as KeptRel, Stored as KeptStored, local_space, MemoBlob, MemoPut, MemoRow};
pub use connection::BusyRetries;
pub use connection::OperationDeadline;
pub use derivation::{ABSENT, Binding, Derivation, DerivationInput, DerivationRecord, StoredDerivation};
pub use plan_set::FactsPlanSet;
pub use schema::STORE_FILE_NAME;
pub use tree_diff::{TreeChange, TreeDiff};

#[cfg(any(test, feature = "test-hooks"))]
pub use connection::{
    BUSY_ATTEMPT_HOOK, BUSY_DEADLINE_OVERRIDE, BUSY_TIMEOUT_OVERRIDE, BUSY_WAIT_HOOK,
    operation_waited,
};
#[cfg(any(test, feature = "test-hooks"))]
pub use snapshots::AFTER_GIT_TREE_TRANSACTION;

use connection::{ThreadConnection, retry_busy};
pub use results::WorkspaceResults;
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};
#[cfg(any(test, feature = "test-hooks"))]
use std::sync::atomic::AtomicUsize;

pub struct ContentStore {
    database_path: PathBuf,
    connection: ThreadConnection,
    compression_level: i32,
    /// This process's atoms of the store (atoms.rs, decision 0049), shared with the planner's symbols.
    atoms: std::sync::Arc<atoms::Atoms>,
    /// What pushes wrote, held for the waves that read it (`working_set`, decision 0024 §5).
    working_set: working_set::WorkingSet,
    /// On the writer's handle, the run whose wave it applies (`write_back::Wave`; 0: none): decision 0306's named sources.
    applying: parking_lot::Mutex<(u64, Option<std::sync::Arc<write_back::Writer>>)>,
    /// The store's writer, shared by its handles in this process (`write_back`, decision 0102); `None` for the
    /// writer's own handle, which writes at once.
    write_back: Option<std::sync::Arc<write_back::Writer>>,
    /// This handle's shared collection lease (`hold_collection`): a file lock, which WASI has not.
    #[cfg(not(target_family = "wasm"))]
    lease: std::sync::Arc<collection_guard::SharedLease>,
    /// How long an ended store session's data is kept (decision 0450 step 3).
    session_ageing: reclamation::SessionAgeing,
    /// An in-memory store's own folder, which its path names and its side folders (`.facts`, `.objects`, ...) are made
    /// in: none for a store on disk, whose folders are beside its file.
    own_folder: Option<std::sync::Arc<OwnFolder>>,
    #[cfg(any(test, feature = "test-hooks"))]
    git_tree_lookup_count: AtomicUsize,
    #[cfg(any(test, feature = "test-hooks"))]
    git_tree_write_count: AtomicUsize,
    #[cfg(any(test, feature = "test-hooks"))]
    git_tree_transactions: AtomicUsize,
}

/// A private folder of an in-memory store's own in the system's temporary folder, removed with the store's last handle.
struct OwnFolder {
    path: PathBuf,
}

impl OwnFolder {
    /// The first free `rerail-memory-store-<pid>-<n>`, made private: the file system's `create_dir` is what makes it
    /// this store's alone.
    fn make() -> StoreResult<Self> {
        let parent = std::env::temp_dir();
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        for n in 0u64.. {
            let path = parent.join(format!("rerail-memory-store-{}-{n}", std::process::id()));
            match builder.create(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(StoreError::Io(e)),
            }
        }
        unreachable!("a free name below u64::MAX")
    }
}

impl Drop for OwnFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

impl Drop for ContentStore {
    /// Appends the atoms still pending (decision 0049 §4.3); another handle or the next open
    /// writes them if this fails, and nothing durable names them meanwhile. The store's last
    /// handle then closes the process's connections to its file: the arena outlives it (the
    /// planner's symbols keep it), its file stays closed until next read.
    fn drop(&mut self) {
        // With a writer, the atoms are its to write: every handle shares them, and its batch's commit appends them (a
        // handle dropped inside a wave writes nothing). The queued writes first: they may name atoms this handle holds.
        if self.atoms.flush_wanted() && self.write_back.is_some() {
            self.queue_atoms("the atoms at a handle's close");
            self.end_writer();
        } else {
            self.end_writer();
            if self.atoms.flush_wanted()
                && let Err(error) = self.immediate_transaction(|_| Ok(()))
            {
                eprintln!("rerail: atoms: writing the pending atoms at close failed: {error}");
            }
        }
        // Either way the handle closes: the writer's job runs on its own handle (`sibling`), which keeps the store
        // open until the job is done. A handle left uncounted kept every connection to its store open for good.
        if self.connection.close() {
            self.atoms.release_reader();
        }
    }
}

impl ContentStore {
    /// The process forgets the store at `path`, closed and about to be deleted while the process goes on (a test's
    /// own): its upkeep jobs, what it read of it to hold in memory (its kept blocks' rows and memo entries, decision 0104) and its identity. Held
    /// past the store's life, each is memory no run reads again, and a job's connection held the deleted file open.
    pub fn forget(path: &Path) {
        // Its upkeep jobs first: one holds a read connection on the file.
        rerail_accel::upkeep::forget_file(path);
        held::forget_snapshot(&path.to_string_lossy());
        atoms::forget_identity(&path.to_string_lossy());
    }

    /// A handle of the same store for the writer's thread (`write_back`): the same atoms, caches of its own, no writer
    /// (it writes at once) and each job's holder.
    fn sibling(&self) -> ContentStore {
        ContentStore {
            database_path: self.database_path.clone(),
            connection: ThreadConnection::new(self.database_path.clone(), self.connection.identity(), self.connection.flush()),
            compression_level: self.compression_level,
            atoms: std::sync::Arc::clone(&self.atoms),
            working_set: working_set::WorkingSet::default(),
            applying: parking_lot::Mutex::new((0, None)),
            write_back: None,
            session_ageing: self.session_ageing.clone(),
            own_folder: self.own_folder.clone(),
            #[cfg(not(target_family = "wasm"))]
            lease: Default::default(),
            #[cfg(any(test, feature = "test-hooks"))]
            git_tree_lookup_count: AtomicUsize::new(0),
            #[cfg(any(test, feature = "test-hooks"))]
            git_tree_write_count: AtomicUsize::new(0),
            #[cfg(any(test, feature = "test-hooks"))]
            git_tree_transactions: AtomicUsize::new(0),
        }
    }

    /// The store's RAM working set (`working_set`, decision 0024 §5).
    pub fn working_set(&self) -> &working_set::WorkingSet {
        &self.working_set
    }

    pub fn open(path: &Path) -> StoreResult<Self> {
        Self::open_with(path, true)
    }

    /// A store opened beside the run's own (the rule images', decision 0264): as `open`, but the Neural Engine's
    /// models stay the run's store's.
    pub fn open_images(path: &Path) -> StoreResult<Self> {
        Self::open_with(path, false)
    }

    fn open_with(path: &Path, models: bool) -> StoreResult<Self> {
        // A store this thread holds open already: its identity read and its schema checked as it opened, the file at
        // the path still the one its connection opened (`connection::held_here`). A run opened its store by its path
        // six times (its phases, its cone, its choices), each a connection opened, configured and its schema read.
        if let Some(identity) = connection::held_here(path) {
            rerail_prof::count("store.opened_held", 1);
            let flush = connection::store_flush()?;
            Self::models_of(path, models);
            return Ok(Self::with_connection(
                path.to_path_buf(), ThreadConnection::new(path.to_path_buf(), identity, flush),
                atoms::Atoms::for_database(path, Some(identity)), Some(write_back::register_store(path, identity)),
            ));
        }
        // Create the database with mode 0600 before SQLite copies its mode to the WAL.
        private_files::prepare(path)?;
        let before = connection::file_key(path);
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_FULL_MUTEX;
        rerail_prof::count("store.opened", 1);
        let mut connection = Connection::open_with_flags(path, flags)?;
        let flush = connection::store_flush()?;
        // Only creating or migrating the schema takes the write lock (`initialize`).
        let (identity, found) = retry_busy(&mut BusyRetries::default(), || {
            Self::initialize_found(&mut connection, flush)
        })?;
        // Configured as a runtime connection (`initialize` runs `configure_connection`) over the store's atoms, when
        // the store was found as it is (`initialize_found`): this thread's connection to it, not closed for the
        // thread's first read to open another.
        let thread = ThreadConnection::new(path.to_path_buf(), identity, flush);
        if found {
            let after = connection::file_key(path);
            thread.seed(connection, before.filter(|b| Some(*b) == after));
        }
        Self::models_of(path, models);
        // A store created again at its path has atoms and a writer of its own too.
        Ok(Self::with_connection(
            path.to_path_buf(), thread,
            atoms::Atoms::for_database(path, Some(identity)), Some(write_back::register_store(path, identity)),
        ))
    }

    /// Compiled Neural Engine models are facts of the store at `path` (decision 0098 §3), unless `models` is false.
    fn models_of(path: &Path, models: bool) {
        #[cfg(target_os = "macos")]
        if models {
            rerail_accel::ane::set_store(std::sync::Arc::new(accel_models::StoreModels::new(path)));
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (path, models);
    }

    /// The scratch store for fact-only evaluation with the durable cache disabled: in memory, so nothing is flushed
    /// (decision 0481's `none`).
    pub fn open_ephemeral() -> StoreResult<Self> {
        let mut connection = Connection::open_in_memory()?;
        let atoms = atoms::Atoms::ephemeral();
        atoms::Atoms::share_memory(&connection, std::sync::Arc::clone(&atoms))?;
        let flush = connection::store_flush()?;
        let identity = Self::initialize(&mut connection, flush)?;
        // Its path is its own folder's: what is kept beside a store (its kept facts, objects and memo) and what the
        // process keys by a store's path (its bound kept spaces, its snapshots) is then this store's alone. Named
        // `:memory:`, every in-memory store shared one folder in the working directory, and each one's write swept away
        // the files another's rows named.
        let folder = OwnFolder::make()?;
        let mut store = Self::with_connection(folder.path.join("store"), ThreadConnection::memory(connection, identity, flush), atoms, None);
        store.own_folder = Some(std::sync::Arc::new(folder));
        Ok(store)
    }

    fn with_connection(database_path: PathBuf, connection: ThreadConnection, atoms: std::sync::Arc<atoms::Atoms>, write_back: Option<std::sync::Arc<write_back::Writer>>) -> Self {
        Self {
            database_path,
            connection,
            compression_level: DEFAULT_COMPRESSION_LEVEL,
            atoms,
            working_set: working_set::WorkingSet::default(),
            applying: parking_lot::Mutex::new((0, None)),
            write_back,
            session_ageing: reclamation::SessionAgeing::default(),
            own_folder: None,
            #[cfg(not(target_family = "wasm"))]
            lease: Default::default(),
            #[cfg(any(test, feature = "test-hooks"))]
            git_tree_lookup_count: AtomicUsize::new(0),
            #[cfg(any(test, feature = "test-hooks"))]
            git_tree_write_count: AtomicUsize::new(0),
            #[cfg(any(test, feature = "test-hooks"))]
            git_tree_transactions: AtomicUsize::new(0),
        }
    }

    /// Drop process-local read caches and close every thread's idle connection to the store, with
    /// its SQLite page and statement caches. Call this only when a workspace has released its
    /// resident documents: an active apply benefits from the caches, while an inactive one should
    /// not retain an arena or SQLite page cache. Connections reopen on the next read.
    pub fn release_memory(&self) -> StoreResult<()> {
        self.connection.release_all();
        Ok(())
    }
}
/// A stored tape whose search index is not built yet, with what building it reads.
pub struct UnindexedTape {
    pub tape: TapeId,
    /// The workspace's grammar id (`workspace::grammar_id`, or a derived document's
    /// `workspace::document_grammar_id`).
    pub grammar: i64,
    /// The source and the tape, read in place in their segments.
    pub source: ObjectBytes,
    pub image: ObjectBytes,
}

#[derive(Clone, Debug)]
pub struct StoredLocalResult {
    pub candidate: CandidateClause,
    /// The result's bytes where its entry holds them, mapped (`results.rs`).
    pub bytes: ObjectBytes,
}

/// The store handles open on the database at `path` in this process, and the thread connections to it still open.
#[cfg(any(test, feature = "test-hooks"))]
pub fn holding(path: &Path) -> (usize, usize) {
    connection::holding(path)
}

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

