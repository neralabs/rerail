//! The shared content store: the process-wide store registry, background maintenance (search-index
//! backlog) and a workspace's storage context.

use super::persisted::{grammar_set, grammar_version, tape_dialect};
use crate::code_rule_dsl::json::invalid;
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::code_rule_dsl::tape;
use crate::content_store::{
    CapacityState, ContentStore, MAX_COLLECTION_BATCH, OperationDeadline, SnapshotId, StoreError,
    StoreResult, WorkspaceId, WorkspaceRegistration,
};
use crate::git_workspace;
use napi::Result;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Arc as StdArc, LazyLock, Mutex};
use std::time::{Duration, Instant};

pub(super) struct StoreRegistryEntry {
    store: std::sync::Weak<ContentStore>,
    maintenance: std::sync::Weak<StorageMaintenance>,
}

pub(super) static CONTENT_STORES: LazyLock<Mutex<HashMap<std::path::PathBuf, StoreRegistryEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(super) fn shared_content_store(
    path: std::path::PathBuf,
) -> StoreResult<(StdArc<ContentStore>, StdArc<StorageMaintenance>)> {
    let mut stores = CONTENT_STORES
        .lock()
        .map_err(|_| StoreError::Invalid("content store registry mutex poisoned".to_owned()))?;
    if let Some(store) = stores.get(&path).and_then(|entry| entry.store.upgrade()) {
        let maintenance = stores
            .get(&path)
            .and_then(|entry| entry.maintenance.upgrade())
            .unwrap_or_else(|| StdArc::new(StorageMaintenance::start(StdArc::clone(&store))));
        stores.insert(
            path,
            StoreRegistryEntry {
                store: StdArc::downgrade(&store),
                maintenance: StdArc::downgrade(&maintenance),
            },
        );
        return Ok((store, maintenance));
    }
    stores.retain(|_, entry| entry.store.strong_count() > 0);
    let store = StdArc::new(ContentStore::open(&path)?);
    let maintenance = StdArc::new(StorageMaintenance::start(StdArc::clone(&store)));
    stores.insert(
        path,
        StoreRegistryEntry {
            store: StdArc::downgrade(&store),
            maintenance: StdArc::downgrade(&maintenance),
        },
    );
    Ok((store, maintenance))
}

pub(super) struct StorageMaintenance {
    stop: StdArc<AtomicBool>,
    workspaces: StdArc<Mutex<HashMap<WorkspaceId, (String, usize)>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl StorageMaintenance {
    fn start(store: StdArc<ContentStore>) -> Self {
        let stop = StdArc::new(AtomicBool::new(false));
        let workspaces = StdArc::new(Mutex::new(HashMap::<WorkspaceId, (String, usize)>::new()));
        let thread_stop = StdArc::clone(&stop);
        let thread_workspaces = StdArc::clone(&workspaces);
        let thread = std::thread::Builder::new()
            .name("rerail-store-maintenance".to_owned())
            .spawn(move || {
                let mut search_index_cursor = None;
                loop {
                    std::thread::park_timeout(Duration::from_secs(30));
                    if thread_stop.load(Ordering::Acquire) {
                        break;
                    }
                    let active = thread_workspaces
                        .lock()
                        .map(|workspaces| {
                            workspaces
                                .iter()
                                .map(|(workspace_id, (owner, _))| (*workspace_id, owner.clone()))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    if let Ok(now) = unix_seconds() {
                        let leases = active
                            .iter()
                            .map(|(workspace_id, owner)| (*workspace_id, owner.as_str()))
                            .collect::<Vec<_>>();
                        let _ = store.renew_workspace_leases(&leases, now + 120);
                        if store
                            .capacity()
                            .is_ok_and(|capacity| !matches!(capacity.state, CapacityState::Normal))
                        {
                            let _ = store.collect(now, MAX_COLLECTION_BATCH);
                        }
                    }
                    index_stored_tapes(&store, &thread_stop, &mut search_index_cursor);
                }
            })
            .expect("start content-store maintenance thread");
        Self {
            stop,
            workspaces,
            thread: Some(thread),
        }
    }

    fn register(&self, workspace_id: WorkspaceId, owner: &str) {
        if let Ok(mut workspaces) = self.workspaces.lock() {
            let entry = workspaces
                .entry(workspace_id)
                .or_insert_with(|| (owner.to_owned(), 0));
            entry.0 = owner.to_owned();
            entry.1 += 1;
        }
    }

    fn unregister(&self, workspace_id: WorkspaceId) {
        if let Ok(mut workspaces) = self.workspaces.lock()
            && let Some((_, count)) = workspaces.get_mut(&workspace_id)
        {
            *count -= 1;
            if *count == 0 {
                workspaces.remove(&workspace_id);
            }
        }
    }
}

/// Builds the search (selector) indexes of stored tapes that have none, for at most about a
/// second per maintenance wake: publication never waits on them, and a tape without an index is
/// simply not narrowed (`restore_persisted_file`).
///
/// The keyset cursor only moves forward until the backlog after it is empty, so a batch this build
/// cannot index never pins it; then it starts over and picks up tapes stored below it meanwhile.
/// A tape this build can never decode leaves the backlog with an incomplete index set.
pub(super) fn index_stored_tapes(
    store: &ContentStore,
    stop: &AtomicBool,
    cursor: &mut Option<crate::content_store::TapeId>,
) {
    let started = Instant::now();
    let mut grammars = HashMap::<i64, Arc<tape::Grammar>>::new();
    while started.elapsed() < Duration::from_secs(1) && !stop.load(Ordering::Acquire) {
        let Ok(backlog) = store.tapes_without_search_index_after(
            grammar_version(),
            i64::from(tape::TAPE_FORMAT_VERSION),
            *cursor,
            64,
        ) else {
            // Retry this batch on the next maintenance wake.
            return;
        };
        let Some(last) = backlog.last().map(|unindexed| unindexed.tape) else {
            *cursor = None;
            return;
        };
        let mut tapes = Vec::with_capacity(backlog.len());
        let mut documents = Vec::with_capacity(backlog.len());
        let mut undecodable = Vec::new();
        for unindexed in backlog {
            let tape_id = unindexed.tape;
            match decode_stored_tape(unindexed, &mut grammars, stop) {
                Some(document) => {
                    tapes.push(tape_id);
                    documents.push(document);
                }
                // A restore the stop flag cancelled says nothing about the tape.
                None if stop.load(Ordering::Acquire) => return,
                None => undecodable.push(tape_id),
            }
        }
        *cursor = Some(last);
        // A failed write leaves its tapes in the backlog for the next pass over it.
        if !undecodable.is_empty()
            && store
                .put_unavailable_search_indexes(
                    &undecodable,
                    crate::code_rule_dsl::transducer::POSITIONAL_INDEX_VERSION,
                )
                .is_err()
        {
            return;
        }
        if !tapes.is_empty()
            && crate::code_rule_dsl::transducer::persist_search_index_batch(
                store, &tapes, &documents,
            )
            .is_err()
        {
            return;
        }
    }
}

/// The tape document of one backlog tape, or `None` when this build cannot decode it: an unknown
/// grammar tag, a source that is not UTF-8, or a tape image this grammar rejects. A derived
/// document's tape (`file_documents`) is an ordinary row: it decodes in its own dialect over its
/// own text, and gets a search index of its own.
fn decode_stored_tape(
    unindexed: crate::content_store::UnindexedTape,
    grammars: &mut HashMap<i64, Arc<tape::Grammar>>,
    stop: &AtomicBool,
) -> Option<tape::TapeDoc> {
    let language = tape_dialect(unindexed.grammar)?;
    let grammar = match grammars.get(&unindexed.grammar) {
        Some(grammar) => Arc::clone(grammar),
        None => {
            let loaded = tape::Grammar::new(language).ok()?;
            grammars.insert(unindexed.grammar, Arc::clone(&loaded));
            loaded
        }
    };
    let source = String::from_utf8(unindexed.source).ok()?;
    let image = tape::TapeImage::from_bytes(&unindexed.image, &grammar).ok()?;
    let lines = Arc::new(LineTable::new(&source, Terminators::of(language)));
    tape::TapeDoc::restore_with_lines(
        Arc::new(source),
        grammar,
        &tape::FileImage::single(image),
        lines,
        &|| stop.load(Ordering::Acquire),
    )
    .ok()
}

impl Drop for StorageMaintenance {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

pub(super) struct StorageRegistration {
    maintenance: StdArc<StorageMaintenance>,
    workspace_id: WorkspaceId,
}

impl Drop for StorageRegistration {
    fn drop(&mut self) {
        self.maintenance.unregister(self.workspace_id);
    }
}

#[derive(Clone)]
pub(super) struct StorageContext {
    pub(super) store: StdArc<ContentStore>,
    pub(super) repository: Option<StdArc<git_workspace::GitWorkspace>>,
    /// The workspace root relative to the repository worktree (`""`, or `dir/` with a trailing
    /// slash). Workspace paths are root-relative; `repository_prefix + path` is the repository path.
    pub(super) repository_prefix: String,
    pub(super) workspace_id: WorkspaceId,
    pub(super) snapshot_id: SnapshotId,
    pub(super) owner: String,
    pub(super) _maintenance: Option<StdArc<StorageRegistration>>,
}

pub(super) fn unix_seconds() -> Result<i64> {
    crate::content_store::unix_seconds().map_err(|error| invalid(error.to_string()))
}

/// A workspace's storage context over an explicit store file, discovering its Git worktree.
#[cfg(test)]
pub(super) fn storage_context_in(
    root: &str,
    store_path: std::path::PathBuf,
) -> Result<StorageContext> {
    let discovered = git_workspace::discover(std::path::Path::new(root))
        .map_err(|error| invalid(error.to_string()))?;
    match attach_storage(root, discovered, store_path)? {
        Attached::Storage(context) => Ok(context),
        Attached::Busy(reason) => Err(invalid(reason)),
    }
}

/// What a storage attach produced.
pub(super) enum Attached {
    Storage(StorageContext),
    /// Another writer held the store past the write deadline, so the workspace runs without
    /// reuse: the store is a cache, and waiting on it costs time, never results. Says why.
    Busy(String),
}

/// The storage context of the workspace at `root` in the `discovered` Git worktree, over an
/// explicit store file; `Attached::Busy` when the store stays busy.
pub(super) fn attach_storage(
    root: &str,
    discovered: git_workspace::GitWorkspace,
    store_path: std::path::PathBuf,
) -> Result<Attached> {
    let busy = |during: &str| {
        Attached::Busy(format!(
            "content store busy: another writer held it past the write deadline {during}"
        ))
    };
    // One write deadline for the whole attach: the open's schema setup and the snapshot
    // publication share it (`publish_snapshot` joins it).
    let _deadline = OperationDeadline::enter();
    let repository_prefix = repository_prefix(root, &discovered.worktree_root)?;
    let (store, maintenance) = match shared_content_store(store_path) {
        Ok(shared) => shared,
        Err(StoreError::Busy) => return Ok(busy("while opening the store")),
        Err(error) => return Err(invalid(error.to_string())),
    };
    let owner = format!(
        "rerail:{}:{}",
        std::process::id(),
        discovered.worktree_root.to_string_lossy()
    );
    let published = discovered.publish_snapshot(
        &store,
        1,
        grammar_set(),
        i64::from(tape::TAPE_FORMAT_VERSION),
        &WorkspaceRegistration {
            root: discovered.worktree_root.to_string_lossy().as_ref(),
            repository: discovered.common_directory.to_string_lossy().as_ref(),
            owner: &owner,
            generation: 1,
            lease_seconds: 120,
        },
    );
    let (snapshot, workspace_id) = match published {
        Ok(published) => published,
        Err(StoreError::Busy) => return Ok(busy("while publishing the Git snapshot")),
        Err(error) => {
            return Err(invalid(format!(
                "snapshot publication failed: cannot persist Git snapshot: {error}"
            )));
        }
    };
    maintenance.register(workspace_id, &owner);
    let registration = StdArc::new(StorageRegistration {
        maintenance,
        workspace_id,
    });
    Ok(Attached::Storage(StorageContext {
        store,
        repository: Some(StdArc::new(discovered)),
        repository_prefix,
        workspace_id,
        snapshot_id: snapshot,
        owner,
        _maintenance: Some(registration),
    }))
}

/// `root` relative to `worktree` as a path prefix: `""` at the worktree root, else `dir/`.
fn repository_prefix(root: &str, worktree: &std::path::Path) -> Result<String> {
    let root = std::path::Path::new(root)
        .canonicalize()
        .map_err(|error| invalid(format!("cannot resolve workspace root: {error}")))?;
    let relative = root
        .strip_prefix(worktree)
        .map_err(|_| invalid("workspace root is outside its Git worktree"))?;
    let mut prefix = String::new();
    for component in relative.components() {
        let name = component
            .as_os_str()
            .to_str()
            .ok_or_else(|| invalid("workspace root path is not UTF-8"))?;
        prefix.push_str(name);
        prefix.push('/');
    }
    Ok(prefix)
}
