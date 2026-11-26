//! Read-only source views of Git trees. Paths identify objects; they never name an extracted checkout.
//! Like the internal data layer's library Git importer, tree walks retain blob IDs and bytes are read on demand.

use rerail_store::Canonical as _;
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
    observed_trees: Mutex<TreeCache>,
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
    pub invalid_paths: Vec<InvalidGitPath>,
    pub tree_failures: Vec<SourceFailure>,
    pub tree_observations: std::collections::BTreeMap<gix::ObjectId, String>,
    pub selection: String,
    /// The memo plans the runs that made it recorded manifests of, under `manifest_root`: a run that reuses it owns
    /// them again for its scope (`own_recorded_manifests`), as it would by running them.
    pub plans: rerail_blocks::vector::Vector<[u8; 32]>,
    pub manifest_root: String,
}

/// Whether a run reusing `inventory` holds what its runs recorded: under an owner, their manifests owned again for its
/// scope. False when they are gone (collected): the inventory runs again, and records them.
fn owned_again(inventory: &Inventory, store: Option<&Path>, owner: Option<&AnalysisOwner>) -> Result<bool, InventoryError> {
    let (Some(store), Some(owner)) = (store, owner) else { return Ok(true) };
    if inventory.plans.is_empty() {
        return Ok(true);
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    // The store is above git (rerail-store over rerail-observe): reached through what it registers at load.
    let opened = crate::content_store::observer_store::base::STORE.get().map_err(|missing| InventoryError::Cache(crate::content_store::StoreError::Invalid(missing.to_string())))?;
    Ok(opened.own_recorded_manifests(store, &inventory.manifest_root, &inventory.plans, owner, now).map_err(InventoryError::Cache)? > 0)
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

/// A directory entry's raw name and its mode.
pub type RawEntryMode = (Vec<u8>, u16);

fn sorted_entries(mut entries: Vec<RawTreeEntry>) -> Arc<[RawTreeEntry]> {
    entries.sort_unstable_by(|left, right| left.name.cmp(&right.name));
    entries.into()
}

pub fn raw_entry_name(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) { return None; }
    hex.as_bytes().as_chunks::<2>().0.iter().map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()).collect()
}

#[derive(Debug)]
pub struct InvalidGitPath {
    pub directory: String,
    pub hex_name: String,
}

impl InvalidGitPath {
    fn failure(&self, prefix: &Path) -> SourceFailure {
        let parent = Path::new(&self.directory).strip_prefix(prefix).ok().and_then(Path::to_str).unwrap_or(&self.directory);
        let prefix = if parent.is_empty() { String::new() } else { format!("{parent}/") };
        let mut hex = rerail_prof::code_rule_dsl::prof::lower_hex(prefix.as_bytes());
        hex.push_str(&self.hex_name);
        SourceFailure {
            path: format!("{prefix}[hex:{}]", self.hex_name), path_identity: PathIdentity::Hex(hex),
            message: "Git path has invalid UTF-8".into(),
        }
    }
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "encoding", content = "value", rename_all = "lowercase")]
pub enum PathIdentity { Utf8(String), Hex(String) }

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceFailure {
    pub path: String,
    pub message: String,
    pub path_identity: PathIdentity,
}

impl SourceFailure {
    pub fn text(path: String, message: String) -> Self {
        Self { path_identity: PathIdentity::Utf8(path.clone()), path, message }
    }
}

#[derive(Debug)]
struct ObservedTree {
    stamp: String,
    entries: Arc<[RawTreeEntry]>,
    bytes: usize,
}

const TREE_CACHE_BYTES: usize = 8 * 1024 * 1024;
type TreeCache = clru::CLruCache<gix::ObjectId, ObservedTree, std::collections::hash_map::RandomState, TreeWeight>;

#[derive(Debug)]
struct TreeWeight;

impl clru::WeightScale<gix::ObjectId, ObservedTree> for TreeWeight {
    fn weight(&self, _: &gix::ObjectId, tree: &ObservedTree) -> usize { tree.bytes }
}

fn tree_cache(bytes: usize) -> TreeCache {
    TreeCache::with_scale(std::num::NonZeroUsize::new(bytes).expect("positive Git tree cache budget"), TreeWeight)
}

fn cache_tree(cache: &mut TreeCache, id: gix::ObjectId, stamp: String, entries: Arc<[RawTreeEntry]>) {
    let bytes = size_of::<(gix::ObjectId, ObservedTree)>() + size_of::<[usize; 2]>() + stamp.capacity()
        + std::mem::size_of_val(entries.as_ref()) + entries.iter().map(|e| e.name.capacity()).sum::<usize>();
    if cache.put_with_weight(id, ObservedTree { stamp, entries, bytes }).is_err() {
        // An oversized tree is served directly. Drop any older observation for this object.
        cache.pop(&id);
    }
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

/// The pinned revisions a read under their roots goes to (`view`): the shared object's (decision 0442), shared by design
/// by the runs of the workspace or entry point that pinned them (`Snapshot::pin`), each with the count of its pins.
/// Another workspace's object holds its own, so its runs over the same root read the root's own files.
#[derive(Default)]
struct Views {
    pinned: Mutex<Vec<(Arc<View>, usize)>>,
    /// How many are pinned: a read finds none without the lock.
    active: AtomicUsize,
}

impl Views {
    fn find(&self, path: &Path) -> Option<Arc<View>> {
        if self.active.load(Ordering::Acquire) == 0 {
            return None;
        }
        self.pinned.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(v, _)| path.starts_with(&v.root)).map(|(v, _)| Arc::clone(v))
    }
    fn pin(&self, view: &Arc<View>) {
        let mut pinned = self.pinned.lock().unwrap_or_else(|e| e.into_inner());
        match pinned.iter_mut().find(|(v, _)| v.id == view.id) {
            Some((_, n)) => *n += 1,
            None => pinned.push((Arc::clone(view), 1)),
        }
        self.active.store(pinned.len(), Ordering::Release);
    }
    fn unpin(&self, id: u64) {
        let mut pinned = self.pinned.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(at) = pinned.iter().position(|(v, _)| v.id == id) {
            pinned[at].1 -= 1;
            if pinned[at].1 == 0 {
                pinned.remove(at);
            }
        }
        self.active.store(pinned.len(), Ordering::Release);
    }
}

/// A snapshot pinned in a shared object (`Snapshot::pin`): its runs' reads under its root go to it until this drops.
pub struct Pinned {
    views: Arc<Views>,
    id: u64,
}

impl std::fmt::Debug for Pinned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pinned").field("id", &self.id).finish()
    }
}

impl Drop for Pinned {
    fn drop(&mut self) {
        self.views.unpin(self.id);
    }
}
/// The storage batch under way (`StorageBatch`): its view and epoch, in the run's context (decision 0442), so its
/// jobs read through it too.
#[derive(Clone, Copy, Default)]
struct StorageBatchId(Option<(u64, u64)>);

fn storage_batch() -> Option<(u64, u64)> {
    rerail_prof::code_rule_dsl::logic::profile::bound::<StorageBatchId>().0
}

thread_local! {
    // One object-store handle per worker, reused across reads. No cache of a snapshot's whole contents.
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

/// The last tree entry matched by a path. A non-directory ancestor can prevent an exact match.
struct TreePathMatch {
    entry: RawTreeEntry,
    exact: bool,
}

enum SourcePath {
    Selected(Entry),
    Absent,
    Excluded(u16),
    FileAsDirectory(u16),
}

/// One evaluation shares the validated pack layout across its workers. An independent API read
/// validates the layout on entry. The evaluation checks its original layout before returning.
pub struct StorageBatch {
    _bound: rerail_prof::code_rule_dsl::logic::profile::Bound,
    storage: Arc<storage::Storage>,
    view: Arc<View>,
}

impl StorageBatch {
    pub fn begin(root: &Path) -> io::Result<Option<Self>> {
        let Some(view) = view(root) else { return Ok(None) };
        let state = view.current_repository()?;
        let id = (view.id, state.epoch);
        if storage_batch() == Some(id) { return Ok(None); }
        let storage = state.storage.clone();
        drop(state);
        Ok(storage.map(|storage| Self { _bound: rerail_prof::code_rule_dsl::logic::profile::bind(StorageBatchId(Some(id))), storage, view }))
    }

    pub fn is_current(&self) -> bool { self.view.directories.check().is_ok() && self.storage.is_current() }
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
        cache_tree(&mut observed, id, stamp.to_owned(), Arc::clone(&entries));
        Some(entries)
    }

    fn remember_tree(&self, stamp: &str, tree: gix::Tree<'_>) -> io::Result<Arc<[RawTreeEntry]>> {
        let id = tree.id;
        let mut trees = self.observed_trees.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(tree) = trees.get(&id).filter(|tree| tree.stamp == stamp) {
            return Ok(Arc::clone(&tree.entries));
        }
        let entries = decode_tree(tree)?;
        cache_tree(&mut trees, id, stamp.to_owned(), Arc::clone(&entries));
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
        Ok(self.match_tree_path(repo, storage, path)?.filter(|m| m.exact).map(|m| m.entry))
    }

    fn match_tree_path(&self, repo: &gix::Repository, storage: Option<&storage::Storage>, path: &Path) -> io::Result<Option<TreePathMatch>> {
        let mut id = self.tree;
        let mut components = path.components().peekable();
        while let Some(component) = components.next() {
            let tree = self.tree_entries(repo, storage, id)?;
            let name = component.as_os_str().as_encoded_bytes();
            let Ok(index) = tree.binary_search_by(|entry| entry.name.as_slice().cmp(name)) else { return Ok(None) };
            let entry = &tree[index];
            let exact = components.peek().is_none();
            if exact || !entry.is_tree() {
                return Ok(Some(TreePathMatch { entry: entry.clone(), exact }));
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
            && owned_again(&value, store, owner)?
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
        if storage_batch() == Some((self.id, repository.epoch)) { return Ok(repository); }
        self.directories.check()?;
        if !repository.storage.as_deref().is_some_and(storage::Storage::is_current) {
            let options = crate::git_access::opener::open_options().open_path_as_is(true);
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
            if storage_batch() != Some(epoch) { self.directories.check()?; }
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
        match self.select_path(under, directory_only)? {
            SourcePath::Selected(entry) => Ok(entry),
            SourcePath::FileAsDirectory(_) => Err(io::Error::from(io::ErrorKind::NotADirectory)),
            SourcePath::Absent | SourcePath::Excluded(_) => Err(missing()),
        }
    }

    fn select_path(&self, under: &Path, directory_only: bool) -> io::Result<SourcePath> {
        let full = self.prefix.join(under);
        if full.as_os_str().is_empty() {
            return Ok(SourcePath::Selected(Entry::Dir(self.tree)));
        }
        self.with_repo(|repo, storage, _| {
            let Some(TreePathMatch { entry, exact }) = self.match_tree_path(repo, storage, &full)? else { return Ok(SourcePath::Absent) };
            Ok(if !exact {
                SourcePath::Excluded(entry.mode)
            } else if entry.is_tree() {
                SourcePath::Selected(Entry::Dir(entry.target))
            } else if directory_only && entry.is_blob() {
                SourcePath::FileAsDirectory(entry.mode)
            } else if entry.is_blob() {
                SourcePath::Selected(Entry::File(entry.target))
            } else {
                SourcePath::Excluded(entry.mode)
            })
        })?
    }
}

/// The revision pinned on a root `path` is under, in the run's shared object (`Views`): none with no run. On a test's
/// thread with no run, the tests' object (`starter_object`), where a test's snapshot pins itself.
fn view(path: &Path) -> Option<Arc<View>> {
    use rerail_prof::code_rule_dsl::logic::profile;
    if profile::run_own().is_some() {
        return profile::with_shared::<Views, _>(|views| views.find(path));
    }
    profile::starter_object()?.get::<Views>().find(path)
}

pub fn contains(path: &Path) -> bool {
    view(path).is_some()
}

/// The root against which a policy read records repository-relative include paths.
pub fn view_root(path: &Path) -> Option<PathBuf> {
    view(path).map(|view| view.root.clone())
}

/// A scoped object-store view. Its stable root is an identity used by the engine's path-keyed stores.
/// No directory is created at that path, either when the view opens or while it is read.
#[derive(Debug)]
pub struct Snapshot {
    view: Arc<View>,
    /// Its pin in the object of the run it was made in (a test's, `starter_object`), held for its drop; none when made
    /// outside any run (the host's `GitRevisionSource`), whose taker pins it (`pin`).
    _made_in: Option<Pinned>,
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
        self.view.saved_inventory().map_or_else(Vec::new, |(_, inventory)|
            inventory.invalid_paths.iter().map(|p| p.failure(Path::new("")).path).collect())
    }

    pub fn source_failures(&self) -> Vec<SourceFailure> {
        let inventory_failures = self.view.saved_inventory().map_or_else(Vec::new, |(_, inventory)| {
            let invalid = inventory.invalid_paths.iter().map(|p| p.failure(&self.view.prefix));
            invalid.chain(inventory.tree_failures.iter().filter_map(|failure| {
                let path = Path::new(&failure.path).strip_prefix(&self.view.prefix)
                    .map_or_else(|_| failure.path.clone(), |path| path.to_string_lossy().into_owned());
                if matches!(self.directory_entries(&path), Ok(Some(_))) { return None; }
                Some(SourceFailure::text(path, failure.message.clone()))
            })).collect()
        });
        let paths = self.view.inventory.lock().unwrap_or_else(|e| e.into_inner()).paths.clone();
        let unreadable = self.view.source_errors.lock().unwrap_or_else(|e| e.into_inner());
        inventory_failures.into_iter().chain(unreadable.iter().filter(|(path, _)| paths.as_ref().is_none_or(|selected|
            selected.iter().any(|wanted| wanted.includes(Path::new(path.as_str()), false)))).map(|(path, message)|
            SourceFailure::text(path.clone(), message.clone()))).collect()
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

    pub fn read_source_prefix(&self, path: &str, limit: u64) -> io::Result<Vec<u8>> {
        self.with_blob_header(path, |_, storage, id, size, _| {
            storage.ok_or_else(|| error("cannot locate Git blob storage"))?
                .blob_prefix(id, size, limit).map_err(|e| error(format_args!("cannot read Git blob prefix {id}: {e}")))
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

    /// An excluded path names the mode of the entry that stopped the lookup. A missing tree object is an error.
    pub fn path_status(&self, path: &Path) -> io::Result<(&'static str, Option<u16>)> {
        let matched = self.view.select_path(&relative_path(path)?, requires_directory(path))?;
        Ok(match matched {
            SourcePath::Selected(_) => ("selected", None),
            SourcePath::Absent => ("absent", None),
            SourcePath::FileAsDirectory(mode) | SourcePath::Excluded(mode) => {
                let reason = match mode & 0o170000 {
                    0o100000 => "not-directory",
                    0o120000 => "symlink",
                    0o160000 => "gitlink",
                    _ => "unsupported-mode",
                };
                (reason, Some(mode))
            }
        })
    }

    /// List raw entry modes without opening any of their objects.
    pub fn directory_entries(&self, path: &str) -> io::Result<Option<Vec<RawEntryMode>>> {
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
        self.with_blob_header(path, |repo, _, id, size, under| {
            f(size, &|| {
                let bytes = verified_blob(repo, id).map_err(|err| {
                    if err.kind() == io::ErrorKind::NotFound { error(format_args!("missing Git blob object {id}")) } else { err }
                })?.data.clone();
                let key = gix::path::to_unix_separators_on_windows(gix::path::into_bstr(under).map_err(error)?);
                self.view.source_errors.lock().unwrap_or_else(|err| err.into_inner()).remove(key.to_str().map_err(error)?);
                Ok(bytes)
            })
        })
    }

    fn with_blob_header<T>(&self, path: &str, f: impl FnOnce(&gix::Repository, Option<&storage::Storage>, gix::ObjectId, u64, &Path) -> io::Result<T>) -> io::Result<T> {
        let path = Path::new(path);
        let under = relative_path(path)?;
        let Entry::File(id) = self.view.relative_entry(&under, requires_directory(path))? else {
            return Err(io::Error::from(io::ErrorKind::IsADirectory));
        };
        self.view.with_repo(|repo, storage, _| {
            let header = repo.find_header(id).map_err(|e| error(format_args!("cannot read Git blob {id}: {e}")))?;
            if header.kind() != gix::object::Kind::Blob {
                return Err(error(format_args!("expected Git blob {id}, found {}", header.kind())));
            }
            f(repo, storage, id, header.size(), &under)
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
        let repo = physical_objects(crate::git_access::opener::open(source).map_err(|e| e.to_string())?);
        let directories = identity::Directories::open(&repo).map_err(|e| e.to_string())?;
        let work = repo.workdir().map(dunce::canonicalize).transpose().map_err(|e| e.to_string())?;
        let relative = |path: &Path| -> Result<PathBuf, String> {
            let Some(work) = &work else { return Ok(PathBuf::new()); };
            path.canonical().map_err(|e| e.to_string())?.strip_prefix(work)
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
            .canonical()
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
            observed_trees: Mutex::new(tree_cache(TREE_CACHE_BYTES)),
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
        let made_in = rerail_prof::code_rule_dsl::logic::profile::starter_object().map(|shared| pin_view(&view, &shared));
        Ok(Self { view, _made_in: made_in })
    }

    /// Pins this revision in `shared` (decision 0442): the runs on that object (a workspace's, an entry point's) read
    /// the paths under its root from it until the guard drops. Runs on any other object read the root's files.
    pub fn pin(&self, shared: &Arc<rerail_prof::code_rule_dsl::logic::profile::Shared>) -> Pinned {
        pin_view(&self.view, shared)
    }
}

fn pin_view(view: &Arc<View>, shared: &rerail_prof::code_rule_dsl::logic::profile::Shared) -> Pinned {
    let views = shared.get::<Views>();
    views.pin(view);
    Pinned { views, id: view.id }
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

/// What a run over `path` is of, as the whole-run cutoff keys its record: the repository's facts root under the
/// selected object (its commit, else its tree). Not the view's own path: a named analysis's view is another path in
/// each run (its invocation's), so no record of one run was found by the next. The same tree under another commit
/// answers `git_commit` otherwise, so the commit is the object when there is one.
pub fn run_identity(path: &Path) -> Option<String> {
    let v = view(path)?;
    let under = v.cache_root.join(path.strip_prefix(&v.root).ok()?);
    Some(format!("{}\0{}", under.display(), v.commit.unwrap_or(v.tree)))
}

/// The selected tree whose facts a named analysis may retain.
pub fn scoped_tree(path: &Path) -> Option<gix::ObjectId> {
    let v = view(path)?;
    v.scope.as_ref().map(|_| v.tree)
}

pub fn analysis_scope(path: &Path) -> Option<String> {
    view(path)?.scope.clone()
}

/// A file of the inventory, by its path, with its grammar when one is known.
pub type ScannedFile = (String, Option<String>);

pub fn scan_files(root: &Path, reads: Option<&InventoryReads>) -> Option<Result<Vec<ScannedFile>, InventoryError>> {
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
    let key = gix::path::to_unix_separators_on_windows(gix::path::into_bstr(full).ok()?);
    Some(inventory.languages.get(key.to_str().ok()?).cloned())
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
        cache_tree(&mut v.observed_trees.lock().unwrap_or_else(|err| err.into_inner()), v.tree, stamp, sorted_entries(entries));
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
    let options = crate::git_access::opener::open_options().open_path_as_is(true);
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

/// Logical selection identity. The tree identifies its ignore file. Storage stamps are checked separately.
pub fn selection_context(root: &Path) -> Option<(String, String, String)> {
    let v = view(root)?;
    let prefix = v.prefix.join(root.strip_prefix(&v.root).ok()?);
    let slash_path = |p: &Path| {
        let parts = p.components().map(|part| part.as_os_str().to_str()).collect::<Option<Vec<_>>>()?;
        Some(parts.join("/"))
    };
    let paths = v.inventory.lock().unwrap_or_else(|e| e.into_inner()).paths.clone();
    let selection = inventory_stamp(&v, &paths)?;
    let normalized = match &paths {
        Some(paths) => Some(paths.iter().map(|p| Some((slash_path(&p.path)?, p.directory_only))).collect::<Option<Vec<_>>>()?),
        None => None,
    };
    let config = serde_json::to_string(&(INVENTORY_FILTER_VERSION, normalized)).expect("serializable selection");
    Some((slash_path(&prefix)?, config, selection))
}

const INVENTORY_FILTER_VERSION: &str = "ignore-v7";

pub fn inventory_selection(root: &Path) -> Option<String> {
    let v = view(root)?;
    let paths = v.inventory.lock().unwrap_or_else(|err| err.into_inner()).paths.clone();
    inventory_stamp(&v, &paths)
}

fn inventory_stamp(v: &View, paths: &Option<Vec<SelectedPath>>) -> Option<String> {
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
    serde_json::to_string(&(INVENTORY_FILTER_VERSION, ignores, &v.prefix, paths)).ok()
}

/// The only repository and filter the active inventory can answer for.
pub fn inventory_request_key(root: &Path) -> Option<(String, String)> {
    Some((pinned_identity(root)?.repository, inventory_selection(root)?))
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

/// Both the selected path's blob and the storage observation that covers its bytes.
pub fn file_stamp(path: &Path) -> Option<String> {
    let v = view(path)?;
    let Entry::File(id) = v.entry(path).ok()? else { return None };
    v.with_repo(|repo, storage, _| {
        if !repo.has_object(id) { return None; }
        storage?.object(id).map(|stamp| format!("{id}:{stamp}"))
    }).ok()?
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

/// Where a read's bytes go: `len` bytes, written once by the fill it is given, in the home it makes for them (the
/// engine's: a slab's room where a device reads them, decision 0353 step 7).
pub type Placer<'a> = &'a (dyn Fn(usize, &mut dyn FnMut(&mut [u8]) -> io::Result<()>) -> io::Result<rerail_tape_format::home::Home> + Sync);

/// `read`, its bytes written once where `place` puts them: the file read straight into its home, or a blob decoded
/// into it. A file that grows while it is read is refused (its stat names another content).
pub fn read_placed(path: impl AsRef<Path>, place: Placer<'_>) -> io::Result<rerail_tape_format::home::Home> {
    use std::io::Read as _;
    let path = path.as_ref();
    let Some(v) = view(path) else {
        let mut file = crate::content_store::observe::listing::lookup(|| std::fs::File::open(path))?;
        let len = usize::try_from(file.metadata()?.len()).map_err(error)?;
        let home = place(len, &mut |out| file.read_exact(out))?;
        let mut more = [0u8; 1];
        if file.read(&mut more)? != 0 {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "the file grew while it was read"));
        }
        return Ok(home);
    };
    let Entry::File(id) = v.entry(path)? else {
        return Err(io::Error::from(io::ErrorKind::IsADirectory));
    };
    v.with_repo(|repo, _, _| {
        let blob = verified_blob(repo, id)?;
        place(blob.data.len(), &mut |out| {
            // copy: load (the object decoded into the bytes its readers read)
            out.copy_from_slice(&blob.data);
            Ok(())
        })
    })?
}

pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    String::from_utf8(read(path)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn canonicalize(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    let path = path.as_ref();
    // Off a snapshot, the disk's answer, with no `\\?\` prefix on Windows (std's is verbatim, which `/`-joined
    // paths and text comparisons break).
    let Some(v) = view(path) else {
        return dunce::canonicalize(path);
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

    /// Q455 (decision 0442): a pinned revision is its shared object's. Two workspaces' objects in one process: the
    /// runs of the one that pinned the revision read the paths under its root from it, the other's see no revision
    /// there, and once the pin drops neither does. A read with no run sees none.
    #[test]
    fn a_revision_pinned_in_one_shared_object_is_read_by_its_runs_alone() {
        use rerail_prof::code_rule_dsl::logic::profile::{Shared, engine_thread, within};
        let dir = tempfile::tempdir().unwrap();
        let repo = gix::init_bare(dir.path().join("repo.git")).unwrap();
        let blob = repo.write_blob(b"pinned\n").unwrap().detach();
        let entries = vec![TreeEntry { mode: EntryKind::Blob.into(), filename: "a.py".into(), oid: blob }];
        let tree = repo.write_object(&gix::objs::Tree { entries }).unwrap().detach();
        // A thread that is no test's and holds no run (`engine_thread`): a host's, as Node's is.
        let off_any_run = |f: &(dyn Fn() -> bool + Sync)| std::thread::scope(|s| s.spawn(|| { engine_thread(); f() }).join().unwrap());
        // Made off any run (a host's `GitRevisionSource`): pinned nowhere until taken.
        let (git_dir, tree) = (repo.git_dir().to_owned(), tree.to_string());
        let snapshot = std::thread::scope(|s| s.spawn(|| { engine_thread(); Snapshot::new(&git_dir, &tree).unwrap() }).join().unwrap());
        assert!(snapshot._made_in.is_none());
        let file = snapshot.root().join("a.py");
        let (pinning, other) = (Shared::new(), Shared::new());
        let pinned = snapshot.pin(&pinning);
        let reads = |shared: &Arc<Shared>| {
            let _in = within(Arc::clone(shared));
            (contains(&file), read(&file).ok())
        };
        assert_eq!(reads(&pinning), (true, Some(b"pinned\n".to_vec())), "the pinning workspace's runs read the revision");
        assert_eq!(reads(&other), (false, None), "another workspace's runs read the root's own files (there are none)");
        assert!(!off_any_run(&|| contains(&file)), "a read with no run sees no revision");
        drop(pinned);
        assert!(!reads(&pinning).0, "unpinned, its runs see it no more");
    }

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
    fn disjoint_selections_bound_decoded_tree_bytes_and_serve_oversized_directories() {
        let dir = tempfile::tempdir().unwrap();
        let repo = gix::init_bare(dir.path().join("repo.git")).unwrap();
        let blob = repo.write_blob(b"text").unwrap().detach();
        let subtree = |name: &str, count| {
            let entries = (0..count).map(|i| TreeEntry {
                mode: EntryKind::Blob.into(), filename: format!("{name}-{i:04}.py").into(), oid: blob,
            }).collect();
            repo.write_object(&gix::objs::Tree { entries }).unwrap().detach()
        };
        let a = subtree("a", 8);
        let b = subtree("b", 8);
        let c = subtree("c", 8);
        let big = subtree("big", 128);
        let entries = [("a", a), ("b", b), ("big", big), ("c", c)].into_iter()
            .map(|(name, oid)| TreeEntry { mode: EntryKind::Tree.into(), filename: name.into(), oid }).collect();
        let root = repo.write_object(&gix::objs::Tree { entries }).unwrap().detach();
        let source = Snapshot::scoped(repo.git_dir(), &root.to_string(), "bounded").unwrap();
        let cache = || source.view.observed_trees.lock().unwrap();
        let read = |name: &str, count: usize| {
            source.restrict_paths(vec![name.to_owned()]).unwrap();
            let decodes = || rerail_prof::stages().get("git.tree_decodes").map_or(0, |s| s.1);
            let before = decodes();
            let entries = source.directory_entries(name).unwrap().unwrap();
            assert_eq!(entries.len(), count);
            assert_eq!(entries[0].0, format!("{name}-0000.py").as_bytes());
            let trees = cache();
            let retained = trees.weight() + trees.len();
            assert!(retained <= trees.capacity());
            if rerail_prof::on() {
                eprintln!("directory {name}: {} tree decodes, {retained} retained bytes", decodes() - before);
            }
            retained
        };
        read("a", 8);
        // Use the measured size of the root and two equally sized leaves on this architecture.
        let budget = read("b", 8);
        *cache() = tree_cache(budget);
        read("a", 8);
        read("b", 8);
        let held_a = Arc::clone(&cache().peek(&a).unwrap().entries);
        let held_b = Arc::clone(&cache().peek(&b).unwrap().entries);
        read("a", 8);
        read("c", 8);
        assert!(Arc::ptr_eq(&held_a, &cache().peek(&a).unwrap().entries));
        assert!(!cache().contains(&b));
        read("b", 8);
        assert!(!Arc::ptr_eq(&held_b, &cache().peek(&b).unwrap().entries));
        assert!(!cache().contains(&a));
        let held_c = Arc::clone(&cache().peek(&c).unwrap().entries);
        for _ in 0..2 {
            assert_eq!(read("big", 128), budget);
            assert!(!cache().contains(&big));
            assert!(Arc::ptr_eq(&held_c, &cache().peek(&c).unwrap().entries));
        }
        assert_eq!(held_a.len(), 8);
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
