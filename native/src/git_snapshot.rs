//! Read-only source views of Git trees. Paths identify objects; they never name an extracted checkout.
//! Like nera-data-layer's library Git importer, tree walks retain blob IDs and bytes are read on demand.

use std::cell::RefCell;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use super::HashKey;
use crate::content_store::analysis_scope::AnalysisOwner;
use gix::bstr::ByteSlice;

#[path = "git_ignore_coverage.rs"]
mod ignore_coverage;

#[path = "git_snapshot_revision.rs"]
mod revision;
pub use revision::{failure_reason as revision_failure_reason, resolve as resolve_revision};

#[path = "git_snapshot_storage.rs"]
pub mod storage;

#[path = "git_snapshot_identity.rs"]
mod identity;
pub use identity::Identity;

#[derive(Debug)]
struct View {
    id: u64,
    root: PathBuf,
    repository: Mutex<RepositoryState>,
    prefix: PathBuf,
    tree: gix::ObjectId,
    commit: Option<gix::ObjectId>,
    cache_root: PathBuf,
    identity: Identity,
    directories: identity::Directories,
    scope: Option<String>,
    inventory: Mutex<InventoryState>,
    observed_trees: Mutex<std::collections::HashMap<gix::ObjectId, ObservedTree>>,
    source_errors: Mutex<std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, Default)]
struct InventoryState {
    generation: u64,
    saved: Option<(u64, Arc<Inventory>)>,
    paths: Option<Vec<SelectedPath>>,
    host_languages: Option<SourceLanguages>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, serde::Serialize)]
struct SelectedPath {
    path: PathBuf,
    directory_only: bool,
}

impl SelectedPath {
    fn includes(&self, path: &Path, directory: bool) -> bool {
        path.starts_with(&self.path) && (directory || !self.directory_only || path != self.path)
    }
}

/// Whether the spelling requires the final component to be a directory, before Path normalizes it.
pub fn requires_directory(path: &Path) -> bool {
    let bytes = path.as_os_str().as_encoded_bytes();
    matches!(bytes.rsplit(|&b| std::path::is_separator(char::from(b))).next(), Some(b"" | b"." | b".."))
}

fn relative_path(path: &Path) -> io::Result<PathBuf> {
    let mut under = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => under.push(name),
            Component::CurDir => {}
            Component::ParentDir if under.pop() => {}
            _ => return Err(io::Error::new(io::ErrorKind::PermissionDenied, "expected a repository-relative path within the selected root")),
        }
    }
    Ok(under)
}

/// The embedded programs that have already run with this evaluation's store and owner.
/// Workers share these markers only while that store selection is active.
#[derive(Debug, Default)]
pub struct InventoryReads {
    root: Mutex<Option<(u64, String)>>,
    inventory: Mutex<Weak<Inventory>>,
    incomplete: std::sync::atomic::AtomicBool,
}

impl InventoryReads {
    pub fn mark_incomplete(&self) { self.incomplete.store(true, Ordering::Relaxed); }

    pub fn complete(&self) -> bool { !self.incomplete.load(Ordering::Relaxed) }
}

#[derive(Debug)]
struct RepositoryState {
    epoch: u64,
    repo: gix::ThreadSafeRepository,
    storage: Option<Arc<storage::Storage>>,
}

#[derive(Debug)]
pub struct Inventory {
    pub files: Vec<super::BaseFile>,
    pub trees: Vec<crate::content_store::GitTree>,
    pub languages: std::collections::BTreeMap<String, String>,
    pub invalid_paths: Vec<String>,
    pub tree_failures: Vec<SourceFailure>,
    pub tree_observations: std::collections::BTreeMap<gix::ObjectId, String>,
    pub selection: String,
}

#[derive(Clone, Debug)]
pub struct RawTreeEntry {
    pub name: Vec<u8>,
    pub mode: u16,
    pub target: gix::ObjectId,
}

impl RawTreeEntry {
    fn is_tree(&self) -> bool { self.mode & 0o170000 == 0o040000 }
    fn is_blob(&self) -> bool { self.mode & 0o170000 == 0o100000 }
}

fn decode_tree(tree: gix::Tree<'_>) -> io::Result<Arc<[RawTreeEntry]>> {
    rerail_prof::count("git.tree_decodes", 1);
    let entries = tree.iter().map(|entry| {
        let entry = entry.map_err(error)?;
        let name = entry.filename().as_bytes();
        if name.is_empty() || name == b"." || name == b".." || name.contains(&b'/') {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid Git entry name"));
        }
        Ok(RawTreeEntry { name: name.to_vec(), mode: entry.mode().value(), target: entry.id().detach() })
    }).collect::<io::Result<Vec<_>>>()?;
    if entries.windows(2).any(|pair| gix::objs::tree::name_order(
        &pair[0].name, pair[0].is_tree(), &pair[1].name, pair[1].is_tree(),
    ).is_gt()) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("entries out of order in Git tree {}", tree.id)));
    }
    let entries = sorted_entries(entries);
    if let Some(pair) = entries.windows(2).find(|pair| pair[0].name == pair[1].name) {
        return Err(io::Error::new(io::ErrorKind::InvalidData,
            format!("duplicate entry {:?} in Git tree {}", pair[0].name.as_bstr(), tree.id)));
    }
    Ok(entries)
}

fn sorted_entries(mut entries: Vec<RawTreeEntry>) -> Arc<[RawTreeEntry]> {
    entries.sort_unstable_by(|left, right| left.name.cmp(&right.name));
    entries.into()
}

pub fn raw_entry_name(hex: &str) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 { return None; }
    hex.as_bytes().chunks_exact(2).map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()).collect()
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct SourceFailure {
    pub path: String,
    pub message: String,
}

#[derive(Debug)]
struct ObservedTree {
    stamp: String,
    entries: Arc<[RawTreeEntry]>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct SourceLanguages {
    pub suffixes: std::collections::BTreeMap<String, String>,
    pub names: std::collections::BTreeMap<String, String>,
}

#[derive(Debug)]
pub enum InventoryError {
    Source(String),
    Cache(crate::content_store::StoreError),
}

impl From<String> for InventoryError {
    fn from(error: String) -> Self {
        Self::Source(error)
    }
}

impl From<&str> for InventoryError {
    fn from(error: &str) -> Self {
        Self::Source(error.to_owned())
    }
}

type InventoryReader = fn(&Path, Option<&Path>, Option<&AnalysisOwner>) -> Result<Inventory, InventoryError>;
static READ_INVENTORY: OnceLock<InventoryReader> = OnceLock::new();
type SourceChecker = fn(&Path, &Path, &[String], Option<&AnalysisOwner>) -> Result<std::collections::HashMap<String, HashKey>, InventoryError>;
static CHECK_SOURCES: OnceLock<SourceChecker> = OnceLock::new();

pub fn install_inventory_reader(reader: InventoryReader) {
    READ_INVENTORY.get_or_init(|| reader);
}

pub fn install_source_checker(check: SourceChecker) {
    CHECK_SOURCES.get_or_init(|| check);
}

static VIEWS: Mutex<Vec<Arc<View>>> = Mutex::new(Vec::new());
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
thread_local! {
    // One object-store handle per worker, reused across reads. No cache of a snapshot's whole contents.
    pub static STORAGE_BATCH: std::cell::Cell<Option<(u64, u64)>> = const { std::cell::Cell::new(None) };
    static REPOSITORY: RefCell<Option<((u64, u64), gix::Repository)>> = const { RefCell::new(None) };
}

fn error(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}
fn missing() -> io::Error {
    io::Error::from(io::ErrorKind::NotFound)
}

pub fn physical_objects(mut repo: gix::Repository) -> gix::Repository {
    repo.objects.ignore_replacements = true;
    repo
}

pub fn verified_object(repo: &gix::Repository, id: gix::ObjectId) -> io::Result<gix::Object<'_>> {
    let object = repo.try_find_object(id).map_err(error)?.ok_or_else(missing)?;
    gix::objs::Data::new(&object.data, object.kind, id.kind()).verify_checksum(id.as_ref())
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, format!("Git object checksum failed: {err}")))?;
    Ok(object)
}

fn verified_blob(repo: &gix::Repository, id: gix::ObjectId) -> io::Result<gix::Blob<'_>> {
    verified_object(repo, id)?.try_into_blob().map_err(error)
}

pub fn decoded_commit<'a>(commit: &'a gix::Commit<'_>) -> io::Result<gix::objs::CommitRef<'a>> {
    let decoded = commit.decode().map_err(error)?;
    decoded.author().and_then(|sig| sig.time()).map_err(error)?;
    decoded.committer().and_then(|sig| sig.time()).map_err(error)?;
    Ok(decoded)
}

#[derive(Clone, Copy)]
enum Entry {
    File(gix::ObjectId),
    Dir(gix::ObjectId),
}

/// One evaluation shares the validated pack layout across its workers. An independent API read
/// validates the layout on entry. The evaluation checks its original layout before returning.
pub struct StorageBatch {
    previous: Option<(u64, u64)>,
    storage: Arc<storage::Storage>,
    view: Arc<View>,
}

impl StorageBatch {
    pub fn begin(root: &Path) -> io::Result<Option<Self>> {
        let Some(view) = view(root) else { return Ok(None) };
        let state = view.current_repository()?;
        let id = (view.id, state.epoch);
        if STORAGE_BATCH.get() == Some(id) { return Ok(None); }
        let storage = state.storage.clone();
        drop(state);
        Ok(storage.map(|storage| Self { previous: STORAGE_BATCH.replace(Some(id)), storage, view }))
    }

    pub fn is_current(&self) -> bool { self.view.directories.check().is_ok() && self.storage.is_current() }
}

impl Drop for StorageBatch {
    fn drop(&mut self) { STORAGE_BATCH.set(self.previous); }
}

impl View {
    fn observed_tree(&self, id: gix::ObjectId, stamp: &str) -> Option<Arc<[RawTreeEntry]>> {
        let mut observed = self.observed_trees.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(tree) = observed.get(&id).filter(|tree| tree.stamp == stamp) {
            return Some(Arc::clone(&tree.entries));
        }
        let key = super::hash_key(id.as_ref()).ok()?;
        let (_, inventory) = self.saved_inventory()?;
        if inventory.tree_observations.get(&id)?.as_str() != stamp {
            return None;
        }
        let index = inventory.trees.binary_search_by_key(&key, |tree| tree.id).ok()?;
        let entries = inventory.trees[index].entries.iter().map(|entry| {
            let name = match entry.name.strip_prefix('\0') {
                Some(hex) => raw_entry_name(hex)?,
                None => entry.name.as_bytes().to_vec(),
            };
            let width = if entry.target.is_git_sha256() { 32 } else { 20 };
            let target = gix::ObjectId::from_bytes_or_panic(&entry.target.key_bytes()[8..8 + width]);
            Some(RawTreeEntry { name, mode: entry.mode, target })
        }).collect::<Option<Vec<_>>>()?;
        let entries = sorted_entries(entries);
        observed.insert(id, ObservedTree { stamp: stamp.to_owned(), entries: Arc::clone(&entries) });
        Some(entries)
    }

    fn remember_tree(&self, stamp: &str, tree: gix::Tree<'_>) -> io::Result<Arc<[RawTreeEntry]>> {
        let id = tree.id;
        let mut trees = self.observed_trees.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(tree) = trees.get(&id).filter(|tree| tree.stamp == stamp) {
            return Ok(Arc::clone(&tree.entries));
        }
        let entries = decode_tree(tree)?;
        trees.insert(id, ObservedTree { stamp: stamp.to_owned(), entries: Arc::clone(&entries) });
        Ok(entries)
    }

    fn tree_entries(&self, repo: &gix::Repository, storage: Option<&storage::Storage>, id: gix::ObjectId) -> io::Result<Arc<[RawTreeEntry]>> {
        let stamp = storage.and_then(|storage| storage.object(id));
        if let Some(stamp) = &stamp && let Some(tree) = self.observed_tree(id, stamp) && repo.has_object(id) {
            return Ok(tree);
        }
        let tree = verified_object(repo, id).map_err(|err| {
            if err.kind() == io::ErrorKind::NotFound {
                error(format_args!("missing Git tree object {id}"))
            } else {
                err
            }
        })?.try_into_tree().map_err(error)?;
        match stamp {
            Some(stamp) => self.remember_tree(&stamp, tree),
            None => decode_tree(tree),
        }
    }

    /// Return absence only after reading a tree and checking its entries.
    fn lookup_tree_entry(&self, repo: &gix::Repository, storage: Option<&storage::Storage>, path: &Path) -> io::Result<Option<RawTreeEntry>> {
        let mut id = self.tree;
        let mut components = path.components().peekable();
        while let Some(component) = components.next() {
            let tree = self.tree_entries(repo, storage, id)?;
            let name = component.as_os_str().as_encoded_bytes();
            let Ok(index) = tree.binary_search_by(|entry| entry.name.as_slice().cmp(name)) else { return Ok(None) };
            let entry = &tree[index];
            if components.peek().is_none() {
                return Ok(Some(entry.clone()));
            }
            if !entry.is_tree() {
                return Ok(None);
            }
            id = entry.target;
        }
        Ok(None)
    }

    fn ignore_blob(&self) -> Result<Option<gix::ObjectId>, String> {
        let entry = self.with_repo(|repo, storage, _| self.lookup_tree_entry(repo, storage, Path::new(".rerailignore")))
            .and_then(|entry| entry).map_err(|e| e.to_string())?;
        Ok(entry.filter(RawTreeEntry::is_blob).map(|entry| entry.target))
    }

    fn saved_inventory(&self) -> Option<(u64, Arc<Inventory>)> {
        self.inventory.lock().unwrap_or_else(|e| e.into_inner()).saved.clone()
    }

    fn inventory(&self, store: Option<&Path>, owner: Option<&AnalysisOwner>, reads: Option<&InventoryReads>) -> Result<Arc<Inventory>, InventoryError> {
        let generation = self.inventory.lock().unwrap_or_else(|e| e.into_inner()).generation;
        let previous = reads.and_then(|reads| reads.inventory.lock().unwrap_or_else(|e| e.into_inner()).upgrade());
        let (epoch, observed, saved) = self.with_repo(|_, storage, epoch| {
            let saved = self.saved_inventory().filter(|(saved_epoch, inventory)| {
                previous.as_ref().is_some_and(|last| Arc::ptr_eq(last, inventory))
                    && *saved_epoch == epoch && inventory.tree_failures.is_empty() && storage.is_some_and(|storage| {
                    inventory.tree_observations.iter().all(|(id, stamp)| {
                        storage.object(*id).as_ref() == Some(stamp)
                    })
                })
            });
            (epoch, storage.is_some(), saved)
        }).map_err(|error| error.to_string())?;
        if let Some((_, value)) = saved
            && inventory_selection(&self.root).as_ref() == Some(&value.selection)
            && self.inventory.lock().unwrap_or_else(|e| e.into_inner()).generation == generation
        {
            return Ok(value);
        }
        let read = READ_INVENTORY.get().ok_or("Git tree inventory reader is unavailable")?;
        let value = Arc::new(read(&self.root, store, owner)?);
        let state = self.current_repository().map_err(|error| error.to_string())?;
        if observed && state.epoch != epoch {
            return Err("Git object storage changed during inventory".into());
        }
        let mut inventory = self.inventory.lock().unwrap_or_else(|e| e.into_inner());
        if generation != inventory.generation {
            return Err("Git source configuration changed during inventory".into());
        }
        inventory.saved = Some((epoch, Arc::clone(&value)));
        if let Some(reads) = reads {
            *reads.inventory.lock().unwrap_or_else(|e| e.into_inner()) = Arc::downgrade(&value);
        }
        Ok(value)
    }

    fn current_repository(&self) -> io::Result<std::sync::MutexGuard<'_, RepositoryState>> {
        let mut repository = self.repository.lock().unwrap_or_else(|e| e.into_inner());
        if STORAGE_BATCH.get() == Some((self.id, repository.epoch)) { return Ok(repository); }
        self.directories.check()?;
        if !repository.storage.as_deref().is_some_and(storage::Storage::is_current) {
            let options = super::checkout_options(Default::default()).open_path_as_is(true);
            let repo = gix::ThreadSafeRepository::open_opts(repository.repo.git_dir(), options).map_err(error)?;
            self.directories.check()?;
            repository.storage = storage::Storage::of(&repo.to_thread_local()).map(Arc::new);
            repository.repo = repo;
            repository.epoch += 1;
        }
        Ok(repository)
    }

    fn with_repo<T>(&self, f: impl FnOnce(&gix::Repository, Option<&storage::Storage>, u64) -> T) -> io::Result<T> {
        let repository = self.current_repository()?;
        let epoch = (self.id, repository.epoch);
        let storage = repository.storage.clone();
        REPOSITORY.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.as_ref().is_none_or(|(id, _)| *id != epoch) {
                *slot = Some((epoch, physical_objects(repository.repo.to_thread_local())));
            }
            drop(repository);
            let answer = f(&slot.as_ref().unwrap().1, storage.as_deref(), epoch.1);
            if STORAGE_BATCH.get() != Some(epoch) { self.directories.check()?; }
            Ok(answer)
        })
    }

    fn entry(&self, path: &Path) -> io::Result<Entry> {
        let relative = path.strip_prefix(&self.root).map_err(error)?;
        self.relative_entry(&relative_path(relative)?, requires_directory(path))
    }

    fn source_entry(&self, path: &Path) -> io::Result<Entry> {
        self.relative_entry(&relative_path(path)?, requires_directory(path))
    }

    fn relative_entry(&self, under: &Path, directory_only: bool) -> io::Result<Entry> {
        let full = self.prefix.join(under);
        if full.as_os_str().is_empty() {
            return Ok(Entry::Dir(self.tree));
        }
        self.with_repo(|repo, storage, _| {
            let entry = self.lookup_tree_entry(repo, storage, &full)?.ok_or_else(missing)?;
            if entry.is_tree() {
                Ok(Entry::Dir(entry.target))
            } else if entry.is_blob() && directory_only {
                Err(io::Error::from(io::ErrorKind::NotADirectory))
            } else if entry.is_blob() {
                Ok(Entry::File(entry.target))
            } else {
                Err(missing())
            } // Never follow symlinks or enter submodules.
        })?
    }
}

fn view(path: &Path) -> Option<Arc<View>> {
    if ACTIVE.load(Ordering::Acquire) == 0 {
        return None;
    }
    VIEWS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|v| path.starts_with(&v.root))
        .cloned()
}

pub fn contains(path: &Path) -> bool {
    view(path).is_some()
}

/// A scoped object-store view. Its stable root is an identity used by the engine's path-keyed stores.
/// No directory is created at that path, either when the view opens or while it is read.
#[derive(Debug)]
pub struct Snapshot {
    view: Arc<View>,
}

/// A repository, selected object and directory within it, shared by equivalent source views.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct SnapshotKey {
    repository_prefix: PathBuf,
    object: gix::ObjectId,
}

pub fn pinned_identity(root: &Path) -> Option<Identity> {
    view(root).map(|v| v.identity.clone())
}

impl Snapshot {
    pub fn run_source(&self) -> crate::content_store::analysis_scope::RunSource {
        crate::content_store::analysis_scope::RunSource::Git {
            repository: self.view.identity.repository.clone(), tree: self.tree(), commit: self.commit(),
        }
    }

    pub fn source_key(&self) -> SnapshotKey {
        SnapshotKey { repository_prefix: self.view.cache_root.clone(), object: self.view.commit.unwrap_or(self.view.tree) }
    }

    pub fn set_source_languages(&self, languages: SourceLanguages) {
        let mut state = self.view.inventory.lock().unwrap_or_else(|e| e.into_inner());
        if state.host_languages.as_ref() != Some(&languages) {
            state.host_languages = Some(languages);
            state.generation += 1;
            state.saved = None;
        }
    }

    pub fn host_language(&self, path: &str) -> Option<Option<String>> {
        let state = self.view.inventory.lock().unwrap_or_else(|e| e.into_inner());
        state.host_languages.as_ref()?;
        Some(state.saved.as_ref()?.1.languages.get(path).cloned())
    }

    pub fn uses_source_languages(&self, languages: &SourceLanguages) -> bool {
        self.view.inventory.lock().unwrap_or_else(|e| e.into_inner()).host_languages.as_ref() == Some(languages)
    }

    pub fn restrict_paths(&self, paths: Vec<String>) -> io::Result<()> {
        let mut paths = paths.iter().map(|name| {
            let input = Path::new(name);
            Ok(SelectedPath { path: relative_path(input)?, directory_only: requires_directory(input) })
        }).collect::<io::Result<Vec<_>>>()?;
        paths.sort_unstable();
        // Path ordering groups each parent with its descendants. For equal paths the unrestricted spelling comes first.
        let mut selected = Vec::<SelectedPath>::new();
        for path in paths {
            if !selected.last().is_some_and(|parent| parent.includes(&path.path, path.directory_only)) {
                selected.push(path);
            }
        }
        let root_selected = selected.first().is_some_and(|entry| entry.path.as_os_str().is_empty());
        let paths = (!root_selected).then_some(selected);
        let mut state = self.view.inventory.lock().unwrap_or_else(|e| e.into_inner());
        if state.paths != paths {
            state.paths = paths;
            state.generation += 1;
            state.saved = None;
        }
        Ok(())
    }

    pub fn root(&self) -> &Path {
        &self.view.root
    }

    pub fn tree(&self) -> String {
        self.view.tree.to_string()
    }

    pub fn invalid_paths(&self) -> Vec<String> {
        self.view.saved_inventory().map_or_else(Vec::new, |(_, inventory)| inventory.invalid_paths.clone())
    }

    pub fn source_failures(&self) -> Vec<SourceFailure> {
        let invalid = self.invalid_paths().into_iter().map(|path| SourceFailure {
            path, message: "Git path has invalid UTF-8".to_owned(),
        });
        let tree_failures = self.view.saved_inventory().map_or_else(Vec::new, |(_, inventory)| {
            inventory.tree_failures.iter().filter_map(|failure| {
                let path = Path::new(&failure.path).strip_prefix(&self.view.prefix)
                    .map_or_else(|_| failure.path.clone(), |path| path.to_string_lossy().into_owned());
                if matches!(self.directory_entries(&path), Ok(Some(_))) { return None; }
                Some(SourceFailure { path, message: failure.message.clone() })
            }).collect()
        });
        let paths = self.view.inventory.lock().unwrap_or_else(|e| e.into_inner()).paths.clone();
        let unreadable = self.view.source_errors.lock().unwrap_or_else(|e| e.into_inner());
        invalid.chain(tree_failures).chain(unreadable.iter().filter(|(path, _)| paths.as_ref().is_none_or(|selected|
            selected.iter().any(|wanted| wanted.includes(Path::new(path.as_str()), false)))).map(|(path, message)| SourceFailure {
            path: path.clone(), message: message.clone(),
        })).collect()
    }

    /// Verify demanded source identities before a consumer restores saved results.
    pub fn checked_sources(&self, store: &Path, paths: &[String], owner: Option<&AnalysisOwner>) -> Result<std::collections::HashMap<String, HashKey>, InventoryError> {
        CHECK_SOURCES.get().ok_or("Git source checker is unavailable")?(self.root(), store, paths, owner)
    }

    pub fn commit(&self) -> Option<String> {
        self.view.commit.map(|id| id.to_string())
    }

    pub fn workspace(self: &Arc<Self>, store: &Path, owner: Option<&AnalysisOwner>) -> Result<super::GitWorkspace, InventoryError> {
        let inventory = self.view.inventory(Some(store), owner, None)?;
        self.view.with_repo(|repo, _, _| {
            Ok(super::GitWorkspace {
                snapshot: Some(Arc::clone(self)),
                worktree_root: self.view.root.clone(),
                common_directory: repo.common_dir().to_path_buf(),
                hash_algorithm: super::hash_algorithm(repo.object_hash())
                    .map_err(|e| e.to_string())?,
                root_tree: super::hash_key(self.view.tree.as_ref()).map_err(|e| e.to_string())?,
                trees: inventory.trees.clone(),
                base_files: inventory.files.clone(),
                overlays: Vec::new(),
                nested_roots: Vec::new(),
            })
        }).map_err(|error| error.to_string())?
    }

    pub fn read_source(&self, path: &str, limit: u64) -> io::Result<Vec<u8>> {
        self.with_blob(path, |size, read| {
            if size > limit {
                return Err(io::Error::from(io::ErrorKind::FileTooLarge));
            }
            read()
        })
    }

    pub fn contains_path(&self, path: &Path) -> io::Result<bool> {
        let Ok(under) = relative_path(path) else { return Ok(false) };
        match self.view.relative_entry(&under, requires_directory(path)) {
            Ok(_) => Ok(true),
            Err(err) if matches!(err.kind(), io::ErrorKind::NotFound | io::ErrorKind::NotADirectory) => Ok(false),
            Err(err) => Err(err),
        }
    }

    /// List raw entry modes without opening any of their objects.
    pub fn directory_entries(&self, path: &str) -> io::Result<Option<Vec<(Vec<u8>, u16)>>> {
        let directory = match self.view.source_entry(Path::new(path)) {
            Ok(Entry::Dir(id)) => id,
            Ok(Entry::File(_)) => return Err(io::Error::from(io::ErrorKind::NotADirectory)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        };
        self.view.with_repo(|repo, storage, _| {
            let tree = self.view.tree_entries(repo, storage, directory)?;
            Ok(Some(tree.iter().map(|entry| (entry.name.clone(), entry.mode)).collect()))
        })?
    }

    pub fn owns_blob(&self, path: &str, length: u64) -> bool {
        self.with_blob(path, |size, read| Ok(size == length && read().is_ok()))
            .unwrap_or(false)
    }

    fn with_blob<T>(
        &self,
        path: &str,
        f: impl FnOnce(u64, &dyn Fn() -> io::Result<Vec<u8>>) -> io::Result<T>,
    ) -> io::Result<T> {
        let path = Path::new(path);
        let under = relative_path(path)?;
        let Entry::File(id) = self.view.relative_entry(&under, requires_directory(path))? else {
            return Err(io::Error::from(io::ErrorKind::IsADirectory));
        };
        self.view.with_repo(|repo, _, _| {
            let header = repo.find_header(id).map_err(|e| error(format_args!("cannot read Git blob {id}: {e}")))?;
            if header.kind() != gix::object::Kind::Blob {
                return Err(error(format_args!("expected Git blob {id}, found {}", header.kind())));
            }
            f(header.size(), &|| {
                let bytes = verified_blob(repo, id).map_err(|err| {
                    if err.kind() == io::ErrorKind::NotFound { error(format_args!("missing Git blob object {id}")) } else { err }
                })?.data.clone();
                let key = gix::path::to_unix_separators_on_windows(gix::path::into_bstr(under.as_path()).map_err(error)?);
                self.view.source_errors.lock().unwrap_or_else(|err| err.into_inner()).remove(key.to_str().map_err(error)?);
                Ok(bytes)
            })
        })?
    }

    pub fn new(source: &Path, revision: &str) -> Result<Self, String> {
        Self::open(source, revision, None, source)
    }

    pub fn scoped(source: &Path, revision: &str, name: &str) -> Result<Self, String> {
        Self::scoped_from(source, revision, name, source)
    }

    pub fn scoped_from(source: &Path, revision: &str, name: &str, invocation: &Path) -> Result<Self, String> {
        let scope = crate::content_store::analysis_scope::named(name).ok_or("analysis scope name must not be empty")?;
        Self::open(source, revision, Some(scope), invocation)
    }

    fn open(source: &Path, revision: &str, scope: Option<String>, invocation: &Path) -> Result<Self, String> {
        let repo = physical_objects(super::discover_repository(source).map_err(|e| e.to_string())?);
        let directories = identity::Directories::open(&repo).map_err(|e| e.to_string())?;
        let work = repo.workdir().map(std::fs::canonicalize).transpose().map_err(|e| e.to_string())?;
        let relative = |path: &Path| -> Result<PathBuf, String> {
            let Some(work) = &work else { return Ok(PathBuf::new()); };
            path.canonicalize().map_err(|e| e.to_string())?.strip_prefix(work)
                .map(Path::to_owned).map_err(|e| e.to_string())
        };
        let prefix = relative(source)?;
        let (id, commit, tree, selects_path) = {
            let invocation = relative(invocation)?;
            let base = work.as_ref().map(|_| invocation.as_path());
            let (target, selects_path) = revision::selection(&repo, revision, base)
                .map_err(|e| format!("{revision}: {}", revision::failure_detail(&e)))?;
            let object = verified_object(&repo, target).map_err(|e| format!("{target}: {e}"))?;
            let object = revision::peel_object(object, None, |id| verified_object(&repo, id)
                .map_err(|e| gix::Error::from_error(error(format_args!("{id}: {e}")))))
                .map_err(|e| format!("{revision}: {}", revision::failure_detail(&e)))?;
            let id = object.id;
            match object.kind {
                gix::object::Kind::Commit => {
                    let commit = object.into_commit();
                    let tree = decoded_commit(&commit).map_err(|e| format!("{revision}: malformed Git commit {id}: {e}"))?.tree();
                    revision::check_local_kind(&repo, tree, gix::object::Kind::Tree)
                        .map_err(|e| format!("{revision}: {}", revision::failure_detail(&e)))?;
                    (id, Some(id), tree, selects_path)
                }
                gix::object::Kind::Tree => (id, None, id, selects_path),
                _ => return Err(format!("{revision}: Git object {id} is {}, expected a commit or tree", object.kind)),
            }
        };
        let prefix = if selects_path { PathBuf::new() } else { prefix };
        let git_dir = repo
            .git_dir()
            .canonicalize()
            .map_err(|e| e.to_string())?;
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let invocation = NEXT.fetch_add(1, Ordering::Relaxed);
        let mut root = git_dir.join("rerail-object-views");
        if let Some(scope) = &scope {
            use sha2::Digest as _;
            let hash: [u8; 32] = sha2::Sha256::digest(scope.as_bytes()).into();
            root.push(crate::content_store::observe::hex(&hash));
            root.push(invocation.to_string());
        }
        let root = root.join(id.to_string()).join(&prefix);
        let storage = storage::Storage::of(&repo).map(Arc::new);
        let view = Arc::new(View {
            id: invocation,
            root,
            tree,
            commit,
            identity: Identity::of(&repo, scope.is_some()).map_err(|err| err.to_string())?,
            directories,
            cache_root: git_dir.join("rerail-object-facts").join(&prefix),
            prefix,
            scope,
            inventory: Mutex::default(),
            observed_trees: Mutex::default(),
            source_errors: Mutex::default(),
            repository: Mutex::new(RepositoryState { epoch: 0, repo: repo.into_sync(), storage }),
        });
        view.directories.check().map_err(|e| e.to_string())?;
        if !view.prefix.as_os_str().is_empty() {
            view.with_repo(|repo, storage, _| {
                let entry = view.lookup_tree_entry(repo, storage, &view.prefix).map_err(|err| err.to_string())?
                    .filter(RawTreeEntry::is_tree).ok_or("the revision lacks the run root")?;
                view.tree_entries(repo, storage, entry.target).map_err(|err| err.to_string())?;
                Ok::<(), String>(())
            }).map_err(|error| error.to_string())??;
        }
        let mut views = VIEWS.lock().unwrap_or_else(|e| e.into_inner());
        views.push(Arc::clone(&view));
        ACTIVE.store(views.len(), Ordering::Release);
        Ok(Self { view })
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        let mut views = VIEWS.lock().unwrap_or_else(|e| e.into_inner());
        views.retain(|v| v.id != self.view.id);
        ACTIVE.store(views.len(), Ordering::Release);
    }
}

/// The actual repository and repository-relative prefix behind a source root.
pub fn repository(
    root: &Path,
) -> Option<(
    gix::Repository,
    PathBuf,
    gix::ObjectId,
    Option<gix::ObjectId>,
)> {
    let v = view(root)?;
    let under = root.strip_prefix(&v.root).ok()?;
    Some((
        physical_objects(v.repository.lock().unwrap_or_else(|e| e.into_inner()).repo.to_thread_local()),
        v.prefix.join(under),
        v.tree,
        v.commit,
    ))
}

pub fn fact_root(path: &Path) -> Option<PathBuf> {
    let v = view(path)?;
    Some(v.cache_root.join(path.strip_prefix(&v.root).ok()?))
}

/// The selected tree whose facts a named analysis may retain.
pub fn scoped_tree(path: &Path) -> Option<gix::ObjectId> {
    let v = view(path)?;
    v.scope.as_ref().map(|_| v.tree)
}

pub fn analysis_scope(path: &Path) -> Option<String> {
    view(path)?.scope.clone()
}

pub fn scan_files(root: &Path, reads: Option<&InventoryReads>) -> Option<Result<Vec<(String, Option<String>)>, InventoryError>> {
    let v = view(root)?;
    let prefix = v.prefix.join(root.strip_prefix(&v.root).ok()?);
    Some(v.inventory(None, None, reads).map(|inventory| {
        if (!inventory.invalid_paths.is_empty() || !inventory.tree_failures.is_empty()) && let Some(reads) = reads {
            reads.mark_incomplete();
        }
        inventory.files
            .iter()
            .filter_map(|file| {
                Path::new(&file.path)
                    .strip_prefix(&prefix)
                    .ok()
                    .map(|p| (p.to_string_lossy().into_owned(), inventory.languages.get(&file.path).cloned()))
            })
            .collect()
    }))
}

/// The source library's language result, once this view has run its inventory.
pub fn source_language(path: &Path) -> Option<Option<String>> {
    let v = view(path)?;
    let (_, inventory) = v.saved_inventory()?;
    let full = v.prefix.join(path.strip_prefix(&v.root).ok()?);
    Some(inventory.languages.get(full.to_str()?).cloned())
}

/// The trees already read during this invocation, before or after source selection.
pub fn observed_tree(root: &Path, id: gix::ObjectId, stamp: &str) -> Option<Arc<[RawTreeEntry]>> {
    view(root)?.observed_tree(id, stamp)
}

/// Reuse a root program only after this evaluation has recorded its manifest.
pub fn root_observed(root: &Path, reads: &InventoryReads) -> bool {
    let Some(v) = view(root) else { return false };
    let Some((id, stamp)) = reads.root.lock().unwrap_or_else(|e| e.into_inner()).clone() else { return false };
    id == v.id && v.with_repo(|repo, storage, _| {
        storage.and_then(|storage| storage.object(v.tree)).as_ref() == Some(&stamp) && repo.has_object(v.tree)
    }).unwrap_or(false)
}

pub fn remember_root(root: &Path, stamp: String, entries: Vec<RawTreeEntry>, reads: Option<&InventoryReads>) {
    if let Some(v) = view(root) {
        if let Some(reads) = reads {
            *reads.root.lock().unwrap_or_else(|e| e.into_inner()) = Some((v.id, stamp.clone()));
        }
        v.observed_trees.lock().unwrap_or_else(|err| err.into_inner())
            .insert(v.tree, ObservedTree { stamp, entries: sorted_entries(entries) });
    }
}

pub fn decode_verified_tree(root: Option<&Path>, stamp: Option<&str>, tree: gix::Tree<'_>) -> io::Result<Arc<[RawTreeEntry]>> {
    match (root.and_then(view), stamp) {
        (Some(view), Some(stamp)) => view.remember_tree(stamp, tree),
        _ => decode_tree(tree),
    }
}

pub fn with_repository<T>(root: &Path, f: impl FnOnce(&gix::Repository, Option<&storage::Storage>) -> T) -> io::Result<T> {
    view(root).ok_or_else(missing)?.with_repo(|repo, storage, _| f(repo, storage))
}

/// A reused source view keeps its object store. Mutable revision queries also need this evaluation's Git configuration.
pub fn reopen_for_revisions(repo: &gix::Repository) -> io::Result<gix::Repository> {
    let options = super::checkout_options(Default::default()).open_path_as_is(true);
    Ok(physical_objects(gix::ThreadSafeRepository::open_opts(repo.git_dir(), options).map_err(error)?.to_thread_local()))
}

pub struct InventoryFilter {
    prefix: PathBuf,
    paths: Option<Vec<SelectedPath>>,
    rules: super::IgnoreRules,
    coverage: ignore_coverage::IgnoreCoverage,
}

impl InventoryFilter {
    pub fn child(&self, path: &str, name: &str, directory: bool) -> Option<String> {
        let path = if path.is_empty() { name.to_owned() } else { format!("{path}/{name}") };
        let relative = Path::new(&path);
        if !relative.starts_with(&self.prefix) {
            return (directory && self.prefix.starts_with(relative)).then_some(path);
        }
        if let Some(paths) = &self.paths {
            let under = relative.strip_prefix(&self.prefix).ok()?;
            if !paths.iter().any(|p| p.includes(under, directory) || (directory && p.path.starts_with(under))) {
                return None;
            }
        }
        (!self.ignored(path.as_bytes(), directory)).then_some(path)
    }

    /// An invalid name can only be in scope when its valid parent is in scope.
    pub fn invalid_child(&self, parent: &str, name: &str, directory: bool) -> Option<String> {
        let relative = Path::new(parent).strip_prefix(&self.prefix).ok()?;
        if self.paths.as_ref().is_some_and(|paths| !paths.iter().any(|path| path.includes(relative, true))) {
            return None;
        }
        let mut bytes = parent.as_bytes().to_vec();
        if !parent.is_empty() {
            bytes.push(b'/');
        }
        bytes.extend(raw_entry_name(name)?);
        if self.ignored(&bytes, directory) {
            return None;
        }
        let marker = format!("[hex:{name}]");
        Some(if parent.is_empty() { marker } else { format!("{parent}/{marker}") })
    }

    fn ignored(&self, path: &[u8], directory: bool) -> bool {
        super::search_ignored_path(&self.rules.mandatory, path, self.rules.case, directory)
            || (super::search_ignored_path(&self.rules.workspace, path, self.rules.case, directory)
                && (!directory || !self.coverage.may_reinclude(path)))
    }
}

pub fn inventory_filter(root: &Path) -> Result<InventoryFilter, String> {
    let v = view(root).ok_or("Git source view is closed")?;
    let bytes = match v.ignore_blob()? {
        Some(id) => v.with_repo(|repo, _, _| verified_blob(repo, id).map(|blob| blob.data.clone()))
            .and_then(|bytes| bytes).map_err(|e| e.to_string())?,
        None => Vec::new(),
    };
    let rules = super::ignore_patterns(Path::new("/"), gix::ignore::glob::pattern::Case::Sensitive, &bytes)
        .map_err(|err| err.to_string())?;
    let coverage = ignore_coverage::IgnoreCoverage::new(
        rules.workspace.patterns.iter().flat_map(|list| &list.patterns).map(|entry| &entry.pattern),
    );
    let paths = v.inventory.lock().unwrap_or_else(|err| err.into_inner()).paths.clone();
    Ok(InventoryFilter { rules, coverage, paths, prefix: v.prefix.clone() })
}

pub fn inventory_selection(root: &Path) -> Option<String> {
    let v = view(root)?;
    let paths = v.inventory.lock().unwrap_or_else(|err| err.into_inner()).paths.clone();
    let ignores = match v.ignore_blob() {
        Ok(Some(id)) => v.with_repo(|repo, storage, _| {
            storage.and_then(|storage| storage.object(id)).filter(|_| repo.has_object(id))
                .map(|stamp| format!("{id}:{stamp}"))
                .unwrap_or_else(|| match verified_blob(repo, id) {
                    Ok(_) => format!("verified:{id}"),
                    Err(_) => format!("unreadable:{id}"),
                })
        }).unwrap_or_else(|_| format!("unreadable:{id}")),
        Ok(None) => String::new(),
        Err(_) => format!("unreadable:{}", v.tree),
    };
    serde_json::to_string(&("ignore-v7", ignores, &v.prefix, &paths)).ok()
}

pub fn source_languages(root: &Path) -> Option<SourceLanguages> {
    view(root)?.inventory.lock().unwrap_or_else(|e| e.into_inner()).host_languages.clone()
}

/// An immutable regular file's object identity. Directories, symlinks and gitlinks give no blob.
pub fn blob(path: &Path) -> Option<HashKey> {
    let v = view(path)?;
    let Entry::File(id) = v.entry(path).ok()? else {
        return None;
    };
    super::hash_key(id.as_ref()).ok()
}

pub fn source_object(root: &Path, path: &str) -> Option<gix::ObjectId> {
    let view = view(root)?;
    match view.entry(&root.join(path)).ok()? {
        Entry::File(id) => Some(id),
        Entry::Dir(_) => None,
    }
}

pub fn record_source_error(root: &Path, path: &str, message: String) {
    if let Some(view) = view(root) {
        view.source_errors.lock().unwrap_or_else(|e| e.into_inner()).insert(path.to_owned(), message);
    }
}

pub fn clear_source_error(root: &Path, path: &str) {
    if let Some(view) = view(root) {
        view.source_errors.lock().unwrap_or_else(|e| e.into_inner()).remove(path);
    }
}

pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    let path = path.as_ref();
    let Some(v) = view(path) else {
        return std::fs::read(path);
    };
    let Entry::File(id) = v.entry(path)? else {
        return Err(io::Error::from(io::ErrorKind::IsADirectory));
    };
    v.with_repo(|repo, _, _| Ok(verified_blob(repo, id)?.data.clone()))?
}

/// Bounded read of a regular file in a registered Git source view.
pub fn read_bounded(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let v = view(path).ok_or_else(missing)?;
    let Entry::File(id) = v.entry(path)? else {
        return Err(io::Error::from(io::ErrorKind::IsADirectory));
    };
    v.with_repo(|repo, _, _| {
        if repo.find_header(id).map_err(error)?.size() > limit {
            return Err(io::Error::from(io::ErrorKind::FileTooLarge));
        }
        Ok(verified_blob(repo, id)?.data.clone())
    })?
}

pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    String::from_utf8(read(path)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn canonicalize(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    let path = path.as_ref();
    let Some(v) = view(path) else {
        return std::fs::canonicalize(path);
    };
    v.entry(path)?;
    // entry validates .. without allowing escape from this view.
    let mut out = v.root.clone();
    for part in path.strip_prefix(&v.root).map_err(error)?.components() {
        match part {
            Component::Normal(name) => out.push(name),
            Component::ParentDir => {
                out.pop();
            }
            _ => {}
        }
    }
    Ok(out)
}

pub fn is_file(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    view(path).map_or_else(
        || path.is_file(),
        |v| matches!(v.entry(path), Ok(Entry::File(_))),
    )
}

pub fn is_dir(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    view(path).map_or_else(
        || path.is_dir(),
        |v| matches!(v.entry(path), Ok(Entry::Dir(_))),
    )
}

pub fn listing(path: impl AsRef<Path>) -> io::Result<Vec<(String, bool)>> {
    let path = path.as_ref();
    let mut entries = if let Some(v) = view(path) {
        let Entry::Dir(id) = v.entry(path)? else {
            return Err(io::Error::from(io::ErrorKind::NotADirectory));
        };
        v.with_repo(|repo, storage, _| {
            let tree = v.tree_entries(repo, storage, id)?;
            let mut entries = Vec::new();
            for entry in tree.iter() {
                if entry.is_tree() || entry.is_blob() {
                    entries.push((
                        std::str::from_utf8(&entry.name).map_err(error)?.to_owned(),
                        entry.is_tree(),
                    ));
                }
            }
            Ok::<_, io::Error>(entries)
        })??
    } else {
        std::fs::read_dir(path)?
            .map(|entry| {
                let entry = entry?;
                Ok((
                    entry.file_name().to_string_lossy().into_owned(),
                    entry.path().is_dir(),
                ))
            })
            .collect::<io::Result<Vec<_>>>()?
    };
    entries.sort();
    Ok(entries)
}

/// Filesystem stat indexes do not apply to immutable Git sources; their keys are blob IDs.
pub fn file_stat(path: impl AsRef<Path>) -> Option<crate::content_store::FileStat> {
    let path = path.as_ref();
    if contains(path) {
        None
    } else {
        std::fs::metadata(path)
            .ok()
            .map(|m| crate::content_store::FileStat::of(&m))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gix::objs::tree::{Entry as TreeEntry, EntryKind};

    #[test]
    fn bare_tree_sources_are_lazy_case_sensitive_and_never_materialized() {
        let dir = tempfile::tempdir().unwrap();
        let repo = gix::init_bare(dir.path().join("repo.git")).unwrap();
        let upper = repo.write_blob(b"UPPER\n").unwrap().detach();
        let lower = repo.write_blob(b"lower\n").unwrap().detach();
        // Deliberately absent: opening or listing this snapshot must not read file blobs.
        let absent = gix::ObjectId::from_hex(b"9999999999999999999999999999999999999999").unwrap();
        let tree = repo
            .write_object(&gix::objs::Tree {
                entries: vec![
                    TreeEntry {
                        mode: EntryKind::Blob.into(),
                        filename: "A.py".into(),
                        oid: upper,
                    },
                    TreeEntry {
                        mode: EntryKind::Blob.into(),
                        filename: "a.py".into(),
                        oid: lower,
                    },
                    TreeEntry {
                        mode: EntryKind::Link.into(),
                        filename: "link.py".into(),
                        oid: upper,
                    },
                    TreeEntry {
                        mode: EntryKind::Blob.into(),
                        filename: "missing.bin".into(),
                        oid: absent,
                    },
                    TreeEntry {
                        mode: EntryKind::Commit.into(),
                        filename: "submodule".into(),
                        oid: absent,
                    },
                ],
            })
            .unwrap()
            .detach();
        let snapshot = Snapshot::new(repo.git_dir(), &tree.to_string()).unwrap();
        let root = snapshot.root();
        assert!(
            !root.exists(),
            "the source view is never a directory on disk"
        );
        assert!(!repo.git_dir().join("rerail-object-views").exists());
        assert_eq!(read(root.join("A.py")).unwrap(), b"UPPER\n");
        assert_eq!(read(root.join("a.py")).unwrap(), b"lower\n");
        assert!(
            read(root.join("missing.bin")).is_err(),
            "the missing blob is read only when asked for"
        );
        assert!(!is_file(root.join("link.py")));
        assert!(!is_dir(root.join("submodule")));
        assert_eq!(
            listing(root).unwrap(),
            [
                ("A.py".into(), false),
                ("a.py".into(), false),
                ("missing.bin".into(), false)
            ]
        );
        assert_eq!(
            read(root.join("../config")).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        crate::content_store::observe::with(None, |observer| {
            let path = root.join("a.py");
            assert_eq!(
                observer.kind(&path),
                crate::content_store::observe::Kind::File
            );
            assert_eq!(observer.canonical(&path), Some(path.clone()));
            assert_eq!(observer.read(&path).unwrap().0, b"lower\n");
            assert_eq!(observer.digest(&path), Some(sha256(b"lower\n")));
            assert_eq!(observer.listing(root).unwrap().len(), 3);
        });
        assert!(
            !root.exists(),
            "even observer reads never extract a source file"
        );
    }

    fn sha256(bytes: &[u8]) -> [u8; 32] {
        use sha2::Digest;
        sha2::Sha256::digest(bytes).into()
    }

    #[test]
    fn equivalent_path_sets_keep_the_inventory_key_and_generation() {
        let dir = tempfile::tempdir().unwrap();
        let repo = gix::init_bare(dir.path().join("repo.git")).unwrap();
        let tree = gix::ObjectId::empty_tree(repo.object_hash()).to_string();
        let snapshot = Snapshot::scoped(repo.git_dir(), &tree, "selection").unwrap();
        let state = || (inventory_selection(snapshot.root()).unwrap(), snapshot.view.inventory.lock().unwrap().generation);
        let restrict = |paths: &[&str]| snapshot.restrict_paths(paths.iter().map(|s| s.to_string()).collect());
        let all = state();
        restrict(&["./", "dir/a"]).unwrap();
        assert_eq!(state(), all);
        restrict(&["dir/", "dir-other", "z.py"]).unwrap();
        let selected = state();
        restrict(&["z.py", "dir/child", "./dir/.", "dir-other", "dir-other", "abc/../z.py"]).unwrap();
        assert_eq!(state(), selected);
        assert!(restrict(&["dir", "../outside"]).is_err());
        assert_eq!(state(), selected);
        let filter = inventory_filter(snapshot.root()).unwrap();
        assert_eq!(filter.child("", "dir", false), None);
        assert_eq!(filter.child("", "dir", true).as_deref(), Some("dir"));
        assert_eq!(filter.child("dir", "child", false).as_deref(), Some("dir/child"));
        restrict(&[]).unwrap();
        assert_ne!(state().0, all.0);
        assert_eq!(inventory_filter(snapshot.root()).unwrap().child("", "z.py", false), None);
        restrict(&["."]).unwrap();
        assert_eq!(state().0, all.0);
    }

    #[test]
    fn identical_views_share_an_identity_and_have_independent_scopes() {
        let dir = tempfile::tempdir().unwrap();
        let repo = gix::init_bare(dir.path().join("repo.git")).unwrap();
        let blob = repo.write_blob(b"content").unwrap().detach();
        let tree = repo
            .write_object(&gix::objs::Tree {
                entries: vec![TreeEntry {
                    mode: EntryKind::Blob.into(),
                    filename: "file.py".into(),
                    oid: blob,
                }],
            })
            .unwrap()
            .detach();
        let one = Snapshot::new(repo.git_dir(), &tree.to_string()).unwrap();
        let two = Snapshot::new(repo.git_dir(), &tree.to_string()).unwrap();
        assert_eq!(one.root(), two.root());
        drop(one);
        assert_eq!(
            read_to_string(two.root().join("file.py")).unwrap(),
            "content"
        );
        let root = two.root().to_owned();
        drop(two);
        assert!(!contains(&root));
        assert!(!root.exists());
    }
}
