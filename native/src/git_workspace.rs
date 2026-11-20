//! In-process Git and worktree discovery through `gix`.
//! Production code in this module never executes an external Git process.

use crate::content_store::{
    ContentStore, GitTree, GitTreeEntry, HashKey, SnapshotId, SourceId, StoreResult, WorkspaceId,
    WorkspaceRegistration,
};
use gix::bstr::ByteSlice;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt;
#[cfg(unix)]
use std::io::Read;
use std::path::{Path, PathBuf};
#[cfg(any(test, feature = "test-hooks"))]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(unix)]
use std::{
    ffi::CString,
    os::fd::{AsRawFd, FromRawFd},
    os::unix::ffi::OsStrExt,
    os::unix::fs::MetadataExt,
};

/// Rerail's records folder (`records_folder`) is among them: Git status leaves a file tracked there
/// unread, which another account sharing the checkout may not read, and the walk never enters it.
const BUILTIN_IGNORE: &[u8] =
    b".git/\nnode_modules/\ntarget/\ntmp/\n__pycache__/\n**/.rerail/sessions/\n";
const STATUS_EXCLUDES: &[&str] = &[
    ":(glob,exclude)**/.git/**",
    ":(glob,exclude)**/node_modules/**",
    ":(glob,exclude)**/target/**",
    ":(glob,exclude)**/tmp/**",
    ":(glob,exclude)**/__pycache__/**",
    ":(glob,exclude)**/.rerail/sessions/**",
];
/// The index entry flags with which gix status skips an entry without comparing it to the
/// worktree (gix-status `index_as_worktree`).
const UNCOMPARED_INDEX_ENTRY: gix::index::entry::Flags = gix::index::entry::Flags::UPTODATE
    .union(gix::index::entry::Flags::SKIP_WORKTREE)
    .union(gix::index::entry::Flags::ASSUME_VALID)
    .union(gix::index::entry::Flags::FSMONITOR_VALID);
/// How a restore compares a clean file's worktree stat with its index entry: every field Git
/// records, nanoseconds and change time included, stricter than Git's defaults. A mismatch only
/// costs a hash, while a false match would reuse results of other bytes: a write that restores
/// the size and modification time (`touch -r`) still changes the change time.
const RESTORE_STAT: gix::index::entry::stat::Options = gix::index::entry::stat::Options {
    trust_ctime: true,
    check_stat: true,
    use_nsec: true,
    use_stdev: false,
};
/// The checkout conversions that can change a file's bytes and keep its size. An end-of-line
/// conversion only inserts carriage returns, so a size check sees it; these need a hash.
const SIZE_PRESERVING_CONVERSIONS: [&str; 3] = ["filter", "ident", "working-tree-encoding"];

#[cfg(any(test, feature = "test-hooks"))]
static SOURCE_ALIAS_HASHES: AtomicUsize = AtomicUsize::new(0);

#[cfg(any(test, feature = "test-hooks"))]
pub fn reset_source_alias_hashes() {
    SOURCE_ALIAS_HASHES.store(0, Ordering::Relaxed);
}

#[cfg(any(test, feature = "test-hooks"))]
pub fn source_alias_hashes() -> usize {
    SOURCE_ALIAS_HASHES.load(Ordering::Relaxed)
}

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    /// Runs once, inside the next discovery on this thread, after it walked the HEAD tree and
    /// before it reads the index: a test commits there.
    pub static AFTER_HEAD_TREE_VISIT: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BaseFile {
    pub path: String,
    pub blob: HashKey,
    pub mode: u16,
    /// The stat Git recorded in the file's index entry when that entry is not racy and no
    /// size-preserving conversion applies to the path: while the worktree file still has exactly
    /// this stat it holds the bytes Git compared with the blob (`holds_blob_bytes`).
    pub checkout: Option<gix::index::entry::Stat>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OverlayKind {
    Staged,
    Unstaged,
    StagedAndUnstaged,
    Deleted,
    Untracked,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceOverlay {
    pub path: String,
    pub kind: OverlayKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitHashAlgorithm {
    Sha1,
    Sha256,
}

#[derive(Clone, Debug)]
pub struct GitWorkspace {
    pub worktree_root: PathBuf,
    pub common_directory: PathBuf,
    pub hash_algorithm: GitHashAlgorithm,
    pub root_tree: HashKey,
    pub trees: Vec<GitTree>,
    pub base_files: Vec<BaseFile>,
    pub overlays: Vec<WorkspaceOverlay>,
    pub nested_roots: Vec<PathBuf>,
}

#[derive(Debug)]
pub enum DiscoveryError {
    /// No Git repository contains the root: a plain directory, which a host walks.
    NoRepository(String),
    /// A repository exists but Git cannot describe it.
    Failed(String),
}

impl DiscoveryError {
    pub fn is_no_repository(&self) -> bool {
        matches!(self, Self::NoRepository(_))
    }
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoRepository(message) | Self::Failed(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for DiscoveryError {}

/// Open `relative_path` below `root` as a regular file without following symlinks, returning it
/// with its metadata as opened.
#[cfg(unix)]
fn open_anchored(
    root: &Path,
    relative_path: &str,
) -> std::io::Result<(std::fs::File, std::fs::Metadata)> {
    let mut directory = std::fs::File::open(root)?;
    let parts = Path::new(relative_path).components().collect::<Vec<_>>();
    for component in &parts[..parts.len().saturating_sub(1)] {
        let name = CString::new(component.as_os_str().as_bytes())?;
        let descriptor = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if descriptor < 0 {
            return Err(std::io::Error::last_os_error());
        }
        directory = unsafe { std::fs::File::from_raw_fd(descriptor) };
    }
    let name = CString::new(
        parts
            .last()
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?
            .as_os_str()
            .as_bytes(),
    )?;
    let descriptor = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let file = unsafe { std::fs::File::from_raw_fd(descriptor) };
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidData));
    }
    Ok((file, metadata))
}

/// Read `relative_path` below `root` without following symlinks, refusing a file larger than
/// `max_bytes` with `ErrorKind::FileTooLarge` before reading it.
#[cfg(unix)]
fn read_anchored(root: &Path, relative_path: &str, max_bytes: u64) -> std::io::Result<Vec<u8>> {
    let (mut file, before) = open_anchored(root, relative_path)?;
    if before.len() > max_bytes {
        return Err(std::io::Error::from(std::io::ErrorKind::FileTooLarge));
    }
    let mut bytes = Vec::with_capacity(before.len().try_into().unwrap_or(0));
    file.read_to_end(&mut bytes)?;
    let after = file.metadata()?;
    if before.dev() != after.dev()
        || before.ino() != after.ino()
        || before.len() != after.len()
        || before.mtime() != after.mtime()
        || before.mtime_nsec() != after.mtime_nsec()
    {
        return Err(std::io::Error::other(
            "workspace source changed while it was read",
        ));
    }
    Ok(bytes)
}

/// Read at most the first `prefix_bytes` of `relative_path` below `root`, without following
/// symlinks.
#[cfg(unix)]
fn read_anchored_prefix(
    root: &Path,
    relative_path: &str,
    prefix_bytes: u64,
) -> std::io::Result<Vec<u8>> {
    let (file, metadata) = open_anchored(root, relative_path)?;
    let mut bytes = Vec::with_capacity(metadata.len().min(prefix_bytes).try_into().unwrap_or(0));
    file.take(prefix_bytes).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// The path of `relative_path` below `root`, refused where it resolves outside `root`.
#[cfg(not(unix))]
fn anchored_path(root: &Path, relative_path: &str) -> std::io::Result<PathBuf> {
    let root = dunce::canonicalize(root)?;
    let path = dunce::canonicalize(root.join(relative_path))?;
    if !path.starts_with(&root) {
        return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
    }
    Ok(path)
}

#[cfg(not(unix))]
fn read_anchored(root: &Path, relative_path: &str, max_bytes: u64) -> std::io::Result<Vec<u8>> {
    let path = anchored_path(root, relative_path)?;
    if std::fs::metadata(&path)?.len() > max_bytes {
        return Err(std::io::Error::from(std::io::ErrorKind::FileTooLarge));
    }
    std::fs::read(path)
}

#[cfg(not(unix))]
fn read_anchored_prefix(
    root: &Path,
    relative_path: &str,
    prefix_bytes: u64,
) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(anchored_path(root, relative_path)?)?
        .take(prefix_bytes)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Refuse a workspace source path that is absolute or holds an empty or `..` component.
fn check_source_path(relative_path: &str) -> std::io::Result<()> {
    if relative_path.starts_with('/')
        || relative_path
            .split('/')
            .any(|part| part == ".." || part.is_empty())
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid workspace source path",
        ));
    }
    Ok(())
}

fn discovery_error(context: &str, error: impl fmt::Display) -> DiscoveryError {
    DiscoveryError::Failed(format!("{context}: {error}"))
}

fn hash_algorithm(kind: gix::hash::Kind) -> Result<GitHashAlgorithm, DiscoveryError> {
    match kind {
        gix::hash::Kind::Sha1 => Ok(GitHashAlgorithm::Sha1),
        gix::hash::Kind::Sha256 => Ok(GitHashAlgorithm::Sha256),
        _ => Err(DiscoveryError::Failed(
            "unsupported Git object hash".to_owned(),
        )),
    }
}

fn hash_kind(algorithm: GitHashAlgorithm) -> gix::hash::Kind {
    match algorithm {
        GitHashAlgorithm::Sha1 => gix::hash::Kind::Sha1,
        GitHashAlgorithm::Sha256 => gix::hash::Kind::Sha256,
    }
}

fn hash_key(id: &gix::hash::oid) -> Result<HashKey, DiscoveryError> {
    match id.kind() {
        gix::hash::Kind::Sha1 => {
            let bytes: [u8; 20] = id
                .as_bytes()
                .try_into()
                .map_err(|_| DiscoveryError::Failed("invalid Git SHA-1 width".to_owned()))?;
            Ok(HashKey::git_sha1(bytes))
        }
        gix::hash::Kind::Sha256 => {
            let bytes: [u8; 32] = id
                .as_bytes()
                .try_into()
                .map_err(|_| DiscoveryError::Failed("invalid Git SHA-256 width".to_owned()))?;
            Ok(HashKey::git_sha256(bytes))
        }
        _ => Err(DiscoveryError::Failed(
            "unsupported Git object hash".to_owned(),
        )),
    }
}

struct IgnoreRules {
    mandatory: gix::ignore::Search,
    workspace: gix::ignore::Search,
    /// The repository's `core.ignoreCase` folding: Git status applies it to `STATUS_EXCLUDES`,
    /// so a path status hides is always ignored here too, never a clean base file.
    case: gix::ignore::glob::pattern::Case,
}

fn load_ignore(
    worktree: &Path,
    case: gix::ignore::glob::pattern::Case,
) -> Result<IgnoreRules, DiscoveryError> {
    let mut mandatory = gix::ignore::Search::default();
    mandatory
        .add_patterns_buffer(
            BUILTIN_IGNORE,
            worktree.join(".rerailignore.defaults"),
            Some(worktree),
            gix::ignore::search::Ignore::default(),
        )
        .map_err(|error| discovery_error("cannot load the built-in ignore rules", error))?;
    let mut workspace = gix::ignore::Search::default();
    let path = worktree.join(".rerailignore");
    match std::fs::read(&path) {
        Ok(bytes) => workspace
            .add_patterns_buffer(
                &bytes,
                path,
                Some(worktree),
                gix::ignore::search::Ignore::default(),
            )
            .map_err(|error| discovery_error("cannot load .rerailignore", error))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(discovery_error("cannot read .rerailignore", error)),
    }
    Ok(IgnoreRules {
        mandatory,
        workspace,
        case,
    })
}

fn search_ignored(
    search: &gix::ignore::Search,
    path: &str,
    case: gix::ignore::glob::pattern::Case,
) -> bool {
    let bytes = path.as_bytes();
    let mut ignored = false;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'/' {
            continue;
        }
        let ancestor = bytes[..index].as_bstr();
        if let Some(found) = search.pattern_matching_relative_path(ancestor, Some(true), case) {
            ignored = !found.pattern.is_negative();
        }
    }
    if let Some(found) = search.pattern_matching_relative_path(bytes.as_bstr(), Some(false), case) {
        ignored = !found.pattern.is_negative();
    }
    ignored
}

fn ignored(rules: &IgnoreRules, path: &str) -> bool {
    search_ignored(&rules.mandatory, path, rules.case)
        || search_ignored(&rules.workspace, path, rules.case)
}

fn mandatory_nested_ignored(rules: &IgnoreRules, path: &str) -> bool {
    search_ignored(&rules.mandatory, path, rules.case)
        || search_ignored(&rules.mandatory, &format!("{path}/.rerail"), rules.case)
}

fn path_string(path: &gix::bstr::BStr) -> Result<String, DiscoveryError> {
    path.to_str()
        .map(str::to_owned)
        .map_err(|_| DiscoveryError::Failed("Git path is not valid UTF-8".to_owned()))
}

fn visit_tree(
    repository: &gix::Repository,
    tree: gix::Tree<'_>,
    prefix: &str,
    ignore: &IgnoreRules,
    trees: &mut Vec<GitTree>,
    base_files: &mut Vec<BaseFile>,
    nested_paths: &mut BTreeSet<String>,
) -> Result<(), DiscoveryError> {
    let tree_id = hash_key(tree.id().detach().as_ref())?;
    let decoded = tree
        .decode()
        .map_err(|error| discovery_error("cannot decode Git tree", error))?;
    let mut entries = Vec::with_capacity(decoded.entries.len());
    for entry in decoded.entries {
        let name = path_string(entry.filename)?;
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let target = hash_key(entry.oid)?;
        entries.push(GitTreeEntry {
            name,
            mode: entry.mode.value(),
            target,
        });
        if entry.mode.is_tree() {
            if ignored(ignore, &path) {
                continue;
            }
            let object = repository
                .find_object(entry.oid)
                .map_err(|error| discovery_error("cannot read Git subtree", error))?;
            let child = object
                .try_into_tree()
                .map_err(|error| discovery_error("Git subtree has the wrong object kind", error))?;
            visit_tree(
                repository,
                child,
                &path,
                ignore,
                trees,
                base_files,
                nested_paths,
            )?;
        } else if entry.mode.is_commit() {
            nested_paths.insert(path);
        } else if entry.mode.is_blob() && !ignored(ignore, &path) {
            base_files.push(BaseFile {
                path,
                blob: target,
                mode: entry.mode.value(),
                checkout: None,
            });
        }
    }
    trees.push(GitTree {
        id: tree_id,
        prefix: prefix.to_owned(),
        entries,
    });
    Ok(())
}

#[derive(Default)]
struct OverlayFlags {
    staged: bool,
    worktree: bool,
    untracked: bool,
}

impl GitWorkspace {
    /// The source paths Git knows in this worktree, each with whether it is clean: what discovery
    /// found, for its tests. Scans list the worktree themselves and use Git only to restore.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn index_files(&self) -> Vec<(String, bool)> {
        let mut files = self
            .base_files
            .iter()
            .map(|file| (file.path.clone(), true))
            .collect::<BTreeMap<_, _>>();
        for overlay in &self.overlays {
            if overlay.kind == OverlayKind::Deleted {
                files.remove(&overlay.path);
            } else {
                files.insert(overlay.path.clone(), false);
            }
        }
        files.into_iter().collect()
    }

    /// The trees no overlay touches. An overlay dirties the trees of its ancestor directories and
    /// the tree at its own path: a gitlink or an untracked nested repository overlays the
    /// directory itself.
    pub fn clean_trees(&self) -> Vec<(String, HashKey)> {
        let mut dirty = HashSet::new();
        for overlay in &self.overlays {
            dirty.insert("");
            dirty.insert(overlay.path.as_str());
            let mut end = overlay.path.len();
            while let Some(index) = overlay.path[..end].rfind('/') {
                dirty.insert(&overlay.path[..index]);
                end = index;
            }
        }
        self.trees
            .iter()
            .filter(|tree| !dirty.contains(tree.prefix.as_str()))
            .map(|tree| (tree.prefix.clone(), tree.id))
            .collect()
    }

    /// Persist the reusable Git tree graph and register `workspace` over its snapshot, in the
    /// snapshot's own transaction. The store's own error is kept, so a caller can tell a busy
    /// store (a cache miss) from a failure.
    pub fn publish_snapshot(
        &self,
        store: &ContentStore,
        scanner_version: i64,
        grammar_set: HashKey,
        tape_format_version: i64,
        workspace: &WorkspaceRegistration<'_>,
    ) -> StoreResult<(SnapshotId, WorkspaceId)> {
        store.publish_snapshot(
            &self.trees,
            self.root_tree,
            scanner_version,
            grammar_set,
            tape_format_version,
            workspace,
        )
    }

    /// Return a committed blob alias without reading bytes only when the path was clean at
    /// discovery. Clean means Git's cleaned bytes match the blob: a restore still confirms the
    /// worktree holds the blob's own bytes (`holds_blob_bytes`).
    pub fn clean_source_alias(&self, relative_path: &str) -> Option<HashKey> {
        self.overlays
            .binary_search_by(|overlay| overlay.path.as_str().cmp(relative_path))
            .is_err()
            .then(|| {
                self.base_files
                    .binary_search_by(|file| file.path.as_str().cmp(relative_path))
                    .ok()
                    .map(|index| self.base_files[index].blob)
            })
            .flatten()
    }

    /// Whether the worktree file at `relative_path` holds exactly its committed blob's bytes, which
    /// are `blob_length` long, now. One `lstat` decides while the file's stat equals the stat of
    /// its index entry (`BaseFile::checkout`); otherwise the file is hashed. The file may have
    /// changed since discovery, and a checkout conversion (`core.autocrlf`, a filter driver) keeps
    /// a file clean in Git while its bytes differ from the blob's.
    pub fn holds_blob_bytes(&self, relative_path: &str, blob_length: u64) -> bool {
        let Ok(index) = self
            .base_files
            .binary_search_by(|file| file.path.as_str().cmp(relative_path))
        else {
            return false;
        };
        let Ok(metadata) =
            gix::index::fs::Metadata::from_path_no_follow(&self.worktree_root.join(relative_path))
        else {
            return false;
        };
        if !metadata.is_file() || metadata.len() != blob_length {
            return false;
        }
        if let Some(recorded) = self.base_files[index].checkout
            && gix::index::entry::Stat::from_fs(&metadata)
                .is_ok_and(|current| recorded.matches(&current, RESTORE_STAT))
        {
            return true;
        }
        self.read_source(relative_path, blob_length)
            .is_ok_and(|bytes| self.source_alias(relative_path, &bytes).is_some())
    }

    /// A clean base file's blob, unread: the path has no overlay and its worktree stat is exactly its index entry's
    /// (`BaseFile::checkout`), so its bytes are the blob's (phase 0, t23). `None` where only a hash could tell.
    pub fn clean_blob_unread(&self, relative_path: &str) -> Option<HashKey> {
        if self
            .overlays
            .binary_search_by(|o| o.path.as_str().cmp(relative_path))
            .is_ok()
        {
            return None;
        }
        let file = &self.base_files[self
            .base_files
            .binary_search_by(|f| f.path.as_str().cmp(relative_path))
            .ok()?];
        let recorded = file.checkout?;
        let metadata =
            gix::index::fs::Metadata::from_path_no_follow(&self.worktree_root.join(relative_path))
                .ok()?;
        (metadata.is_file()
            && gix::index::entry::Stat::from_fs(&metadata)
                .is_ok_and(|now| recorded.matches(&now, RESTORE_STAT)))
        .then_some(file.blob)
    }

    /// Every file of the worktree as Git sees it, relative and sorted: HEAD's (less those deleted), the staged new
    /// ones and the untracked ones Git does not ignore. Ignored directories are not entered; nested repositories are
    /// their own.
    pub fn worktree_files(&self) -> Vec<String> {
        const TREE: u16 = 0o040000;
        const COMMIT: u16 = 0o160000;
        let mut files: BTreeSet<String> = BTreeSet::new();
        for tree in &self.trees {
            for entry in tree
                .entries
                .iter()
                .filter(|e| e.mode & 0o170000 != TREE && e.mode & 0o170000 != COMMIT)
            {
                files.insert(if tree.prefix.is_empty() {
                    entry.name.clone()
                } else {
                    format!("{}/{}", tree.prefix, entry.name)
                });
            }
        }
        for overlay in &self.overlays {
            match overlay.kind {
                OverlayKind::Deleted => {
                    files.remove(&overlay.path);
                }
                _ if self.worktree_root.join(&overlay.path).is_file() => {
                    files.insert(overlay.path.clone());
                }
                _ => {}
            }
        }
        files.into_iter().collect()
    }

    /// Return the committed blob alias only when the evaluated bytes have that identity.
    pub fn source_alias(&self, relative_path: &str, bytes: &[u8]) -> Option<HashKey> {
        let index = self
            .base_files
            .binary_search_by(|file| file.path.as_str().cmp(relative_path))
            .ok()?;
        let algorithm = if self.base_files[index].blob.is_git_sha256() {
            GitHashAlgorithm::Sha256
        } else {
            GitHashAlgorithm::Sha1
        };
        #[cfg(any(test, feature = "test-hooks"))]
        SOURCE_ALIAS_HASHES.fetch_add(1, Ordering::Relaxed);
        let computed = gix::objs::compute_hash(hash_kind(algorithm), gix::objs::Kind::Blob, bytes)
            .ok()
            .and_then(|id| hash_key(&id).ok())?;
        (computed == self.base_files[index].blob).then_some(computed)
    }

    /// Store evaluated source bytes with a safe Git alias when one exists.
    pub fn store_source(
        &self,
        store: &ContentStore,
        relative_path: &str,
    ) -> Result<SourceId, DiscoveryError> {
        let bytes = self
            .read_source(relative_path, u64::MAX)
            .map_err(|error| discovery_error("cannot read workspace source", error))?;
        let aliases = self
            .source_alias(relative_path, &bytes)
            .into_iter()
            .collect::<Vec<_>>();
        store
            .put_source_at(&bytes, &aliases, relative_path)
            .map_err(|error| discovery_error("cannot persist workspace source", error))
    }

    /// Read the exact bytes that code-rule evaluation uses, refusing a file larger than `max_bytes`
    /// with `ErrorKind::FileTooLarge`. The bytes carry no Git identity: publication hashes them
    /// against the committed blob only when the path is clean (`source_alias`).
    pub fn read_source(&self, relative_path: &str, max_bytes: u64) -> std::io::Result<Vec<u8>> {
        check_source_path(relative_path)?;
        read_anchored(&self.worktree_root, relative_path, max_bytes)
    }

    /// Read at most the first `prefix_bytes` of a source under the same path rules as
    /// `read_source`, as a test of a file too large to read whole.
    pub fn read_source_prefix(
        &self,
        relative_path: &str,
        prefix_bytes: u64,
    ) -> std::io::Result<Vec<u8>> {
        check_source_path(relative_path)?;
        read_anchored_prefix(&self.worktree_root, relative_path, prefix_bytes)
    }
}

/// The attribute stack that tells which files a size-preserving conversion applies to: gix's
/// global attribute files with the system file Rerail's Git reads in front, which the opener keeps
/// out of gix's own stack since finding it starts a Git on Windows. `None` where that file cannot
/// be named, so no file takes the stat shortcut.
fn conversion_attributes<'repository>(
    repository: &'repository gix::Repository,
    index: &gix::index::State,
) -> Result<Option<gix::AttributeStack<'repository>>, DiscoveryError> {
    let Ok(system) = crate::git_access::opener::system_attributes() else {
        return Ok(None);
    };
    let read = |error: &dyn fmt::Display| discovery_error("cannot read Git attributes", error);
    let mut files = system.into_iter().collect::<Vec<_>>();
    files.extend(
        repository
            .attribute_global_paths()
            .map_err(|error| read(&error))?,
    );
    for file in &files {
        crate::git_access::opener::check_size(file).map_err(|error| read(&error))?;
    }
    let mut buf = Vec::new();
    let mut collection = gix::attrs::search::MetadataCollection::default();
    let globals = gix::attrs::Search::new_globals(files, &mut buf, &mut collection)
        .map_err(|error| read(&error))?;
    let state =
        gix::worktree::stack::State::AttributesStack(gix::worktree::stack::state::Attributes::new(
            globals,
            Some(repository.common_dir().join("info").join("attributes")),
            gix::worktree::stack::state::attributes::Source::IdMapping,
            collection,
        ));
    let case = if repository
        .filesystem_options()
        .map_err(|error| discovery_error("cannot read core.ignoreCase", error))?
        .ignore_case
    {
        gix::glob::pattern::Case::Fold
    } else {
        gix::glob::pattern::Case::Sensitive
    };
    let id_mappings = state.id_mappings_from_index(index, index.path_backing(), case);
    let root = repository.workdir().unwrap_or(repository.git_dir());
    Ok(Some(gix::AttributeStack::new(
        gix::worktree::Stack::new(root, state, case, buf, id_mappings),
        repository,
    )))
}

/// Gives each base file whose index entry is not racy, is compared with the worktree and has no
/// size-preserving conversion the stat of that entry (`BaseFile::checkout`).
fn record_checkout_stats(
    attributes: &mut gix::AttributeStack<'_>,
    index: &gix::index::State,
    base_files: &mut [BaseFile],
) -> Result<(), DiscoveryError> {
    let mut conversions = attributes.selected_attribute_matches(SIZE_PRESERVING_CONVERSIONS);
    let index_written = index.timestamp();
    for file in base_files {
        let Some(entry) = index.entry_by_path(file.path.as_bytes().as_bstr()) else {
            continue;
        };
        if entry.stat.is_racy(index_written, RESTORE_STAT)
            || entry
                .flags
                .intersects(UNCOMPARED_INDEX_ENTRY.union(gix::index::entry::Flags::INTENT_TO_ADD))
        {
            continue;
        }
        attributes
            .at_entry(file.path.as_str(), Some(gix::index::entry::Mode::FILE))
            .map_err(|error| discovery_error("cannot read Git attributes", error))?
            .matching_attributes(&mut conversions);
        if conversions.iter_selected().all(|found| {
            matches!(
                found.assignment.state,
                gix::attrs::StateRef::Unspecified | gix::attrs::StateRef::Unset
            )
        }) {
            file.checkout = Some(entry.stat);
        }
    }
    Ok(())
}

/// Discover one repository without an external `git` process.
///
/// The repository opens through the engine's one opener (`git_access::opener::open`, under the Git
/// read time limit), so a repository Git refuses is not discovered, and its status runs no filter
/// driver: a file under one whose stat changed reads as changed. The walk and status that follow
/// have no limit, as a large checkout may take longer. The result is sorted by path and object
/// identity. The caller can use the Git object IDs as aliases only after it compares them with
/// evaluated bytes.
pub fn discover(root: &Path) -> Result<GitWorkspace, DiscoveryError> {
    discover_in(root, false)
}

/// [`discover`] for the files under `root` only: HEAD's subtree there, and status over that part of the
/// worktree. Its trees, base files and overlays are those under `root`; a file elsewhere is none of them.
/// Where the Git repository holding `root` keeps its work tree, its own directory (`HEAD`, the index) and the
/// directory it shares with its other work trees (refs, config, info), each canonical: found without reading its
/// status (decision 0105: a run watches them before it reads the status). Opened through the engine's one opener,
/// as [`discover`] opens it: a repository Git refuses has none.
pub fn locations(root: &Path) -> Option<(PathBuf, PathBuf, PathBuf)> {
    let repository = crate::git_access::opener::open(root).ok()?;
    let canonical = |p: &Path| dunce::canonicalize(p).ok();
    Some((
        canonical(repository.workdir()?)?,
        canonical(repository.git_dir())?,
        canonical(repository.common_dir())?,
    ))
}

pub fn discover_under(root: &Path) -> Result<GitWorkspace, DiscoveryError> {
    discover_in(root, true)
}

/// The parts of a repository a discovery reads: the repository, its worktree (canonical), the
/// ignore rules, and the root's path in the worktree ("" at the top or unscoped).
struct Discovery {
    repository: gix::Repository,
    worktree: PathBuf,
    ignore: IgnoreRules,
    scope: String,
}

/// What a walk of HEAD's tree found: the tree's id, its trees and base files, and its gitlinks.
struct HeadWalk {
    tree: gix::ObjectId,
    trees: Vec<GitTree>,
    base_files: Vec<BaseFile>,
    nested_paths: BTreeSet<String>,
}

fn discover_in(root: &Path, scoped: bool) -> Result<GitWorkspace, DiscoveryError> {
    let (repository, worktree) = open_worktree(root)?;
    let ignore = load_ignore(&worktree, ignore_case(&repository)?)?;
    let scope = scope_in(root, &worktree, scoped)?;
    let found = Discovery {
        repository,
        worktree,
        ignore,
        scope,
    };
    let head = visit_head(&found)?;
    describe(found, head)
}

/// The repository holding `root`, opened through the engine's one opener with no filter program,
/// and its worktree, canonical.
fn open_worktree(root: &Path) -> Result<(gix::Repository, PathBuf), DiscoveryError> {
    let mut repository = crate::git_access::opener::open(root).map_err(|error| match error {
        crate::git_access::opener::OpenError::NoRepository(message) => {
            DiscoveryError::NoRepository(message)
        }
        crate::git_access::opener::OpenError::Failed(message) => DiscoveryError::Failed(message),
    })?;
    crate::git_access::opener::without_filter_programs(&mut repository)
        .map_err(DiscoveryError::Failed)?;
    let worktree = dunce::canonicalize(repository.workdir().ok_or_else(|| {
        DiscoveryError::Failed("bare Git repositories have no workspace".to_owned())
    })?)
    .map_err(|error| discovery_error("cannot resolve Git worktree", error))?;
    Ok((repository, worktree))
}

/// How the repository's `core.ignoreCase` folds the case of paths its ignore rules match.
fn ignore_case(
    repository: &gix::Repository,
) -> Result<gix::ignore::glob::pattern::Case, DiscoveryError> {
    let fold = repository
        .filesystem_options()
        .map_err(|error| discovery_error("cannot read core.ignoreCase", error))?
        .ignore_case;
    Ok(if fold {
        gix::ignore::glob::pattern::Case::Fold
    } else {
        gix::ignore::glob::pattern::Case::Sensitive
    })
}

/// Scoped: the root's path in the worktree ("" at the top), whose subtree and status a discovery
/// reads. Unscoped: "".
fn scope_in(root: &Path, worktree: &Path, scoped: bool) -> Result<String, DiscoveryError> {
    if !scoped {
        return Ok(String::new());
    }
    let canonical = dunce::canonicalize(root)
        .map_err(|error| discovery_error("cannot resolve the root", error))?;
    Ok(canonical
        .strip_prefix(worktree)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default())
}

/// HEAD's tree, walked from the top or, scoped, from its subtree at the scope: a scope where HEAD
/// holds no tree walks nothing.
fn visit_head(found: &Discovery) -> Result<HeadWalk, DiscoveryError> {
    let root_tree = found
        .repository
        .head_tree()
        .map_err(|error| discovery_error("cannot read HEAD tree", error))?;
    // Status compares the index with this same tree: re-reading HEAD there would miss a commit
    // that lands during the walk, leaving its files clean against blobs of the walked tree.
    let mut head = HeadWalk {
        tree: root_tree.id().detach(),
        trees: Vec::new(),
        base_files: Vec::new(),
        nested_paths: BTreeSet::new(),
    };
    let start = if found.scope.is_empty() {
        Some(root_tree)
    } else {
        scope_tree(&root_tree, &found.scope)?
    };
    if let Some(tree) = start {
        visit_tree(
            &found.repository,
            tree,
            &found.scope,
            &found.ignore,
            &mut head.trees,
            &mut head.base_files,
            &mut head.nested_paths,
        )?;
    }
    Ok(head)
}

/// The tree HEAD holds at `scope`, if HEAD holds a tree there.
fn scope_tree<'repo>(
    root_tree: &gix::Tree<'repo>,
    scope: &str,
) -> Result<Option<gix::Tree<'repo>>, DiscoveryError> {
    let Some(entry) = root_tree
        .lookup_entry_by_path(scope)
        .map_err(|error| discovery_error("cannot read HEAD tree", error))?
        .filter(|entry| entry.mode().is_tree())
    else {
        return Ok(None);
    };
    let subtree = entry
        .object()
        .map_err(|error| discovery_error("cannot read Git subtree", error))?
        .try_into_tree()
        .map_err(|error| discovery_error("Git subtree has the wrong object kind", error))?;
    Ok(Some(subtree))
}

/// Runs, once, the hook a test set to commit between the HEAD tree walk and the index read.
fn after_head_tree_visit() {
    #[cfg(any(test, feature = "test-hooks"))]
    if let Some(hook) = AFTER_HEAD_TREE_VISIT.take() {
        hook();
    }
}

/// The rest of a discovery once HEAD's tree is walked: the paths that differ from it, then the
/// workspace.
fn describe(found: Discovery, mut head: HeadWalk) -> Result<GitWorkspace, DiscoveryError> {
    after_head_tree_visit();
    let index = found
        .repository
        .index_or_empty()
        .map_err(|error| discovery_error("cannot read Git index", error))?;
    let mut flags = status_flags(&found, &index, &mut head)?;
    uncompared_flags(&index, &found.ignore, &mut flags)?;
    checkout_stats(&found.repository, &index, &mut head.base_files)?;
    workspace(found, head, flags)
}

/// Rerail's session records folder, `.rerail/sessions` in any folder (`src/runtime/session-paths.ts`).
const RECORDS_FOLDER: &str = ".rerail/sessions";

/// Whether `path` is a records folder ([`RECORDS_FOLDER`]). Rerail writes a `.gitignore` of `*`
/// there, so Git lists nothing in it. On a checkout several accounts share, the folder lets each
/// other account pass through it and make its own folder there, but not list it, and that
/// `.gitignore` is its maker's alone: a walk by another account can neither list the folder nor
/// read that Git ignores what it holds. So no walk enters it, for any account.
fn records_folder(path: &gix::bstr::BStr) -> bool {
    path == RECORDS_FOLDER || path.ends_with(b"/.rerail/sessions")
}

/// What status's walk for untracked files emits, kept as gix's own status keeps it: never an entry
/// of a type Git cannot track (a socket, a FIFO). Unlike that walk, it never enters a records
/// folder ([`records_folder`]) nor names one: gix stops a walk at the first folder it cannot list,
/// where Git warns and goes on.
#[derive(Default)]
struct UntrackedWalk {
    entries: Vec<gix::dir::Entry>,
}

impl gix::dir::walk::Delegate for UntrackedWalk {
    fn emit(
        &mut self,
        entry: gix::dir::EntryRef<'_>,
        _collapsed_directory_status: Option<gix::dir::entry::Status>,
    ) -> gix::dir::walk::Action {
        if entry.disk_kind != Some(gix::dir::entry::Kind::Untrackable)
            && !records_folder(&entry.rela_path)
        {
            self.entries.push(entry.to_owned());
        }
        std::ops::ControlFlow::Continue(())
    }

    fn can_recurse(
        &mut self,
        entry: gix::dir::EntryRef<'_>,
        for_deletion: Option<gix::dir::walk::ForDeletionMode>,
        worktree_root_is_repository: bool,
    ) -> bool {
        !records_folder(&entry.rela_path)
            && entry.status.can_recurse(
                entry.disk_kind,
                entry.pathspec_match,
                for_deletion,
                worktree_root_is_repository,
            )
    }
}

/// The pathspec of a discovery's status: the scope, if any, then the mandatory excludes.
fn status_patterns(scope: &str) -> Vec<gix::bstr::BString> {
    (!scope.is_empty())
        .then(|| gix::bstr::BString::from(format!(":(top,literal){scope}")))
        .into_iter()
        .chain(
            STATUS_EXCLUDES
                .iter()
                .map(|pattern| gix::bstr::BString::from(*pattern)),
        )
        .collect()
}

/// The untracked files and repositories in the worktree under `patterns`, as status lists them:
/// gix's walk with status's own options and [`UntrackedWalk`] as its delegate.
fn untracked_entries(
    repository: &gix::Repository,
    index: &gix::index::State,
    patterns: &[gix::bstr::BString],
) -> Result<Vec<gix::dir::Entry>, DiscoveryError> {
    let options = repository
        .dirwalk_options()
        .map_err(|error| discovery_error("cannot prepare Git status", error))?
        .emit_untracked(gix::dir::walk::EmissionMode::Matching);
    let mut walk = UntrackedWalk::default();
    let interrupt = std::sync::atomic::AtomicBool::new(false);
    repository
        .dirwalk(index, patterns, &interrupt, options, &mut walk)
        .map_err(|error| discovery_error("cannot read Git status item", error))?;
    Ok(walk.entries)
}

/// Each path Git status finds staged or changed in the worktree, with how; its walk for untracked
/// files is [`untracked_entries`]'s.
fn changed_items(
    found: &Discovery,
    index: &gix::worktree::Index,
    head_tree: gix::ObjectId,
    patterns: &[gix::bstr::BString],
) -> Result<BTreeMap<String, OverlayFlags>, DiscoveryError> {
    let status = found
        .repository
        .status(gix::progress::Discard)
        .map_err(|error| discovery_error("cannot prepare Git status", error))?
        .index(gix::worktree::IndexPersistedOrInMemory::Persisted(
            index.clone(),
        ))
        .untracked_files(gix::status::UntrackedFiles::None)
        .index_worktree_submodules(None)
        .head_tree(head_tree)
        .into_iter(patterns.iter().cloned())
        .map_err(|error| discovery_error("cannot read Git status", error))?;
    let mut flags = BTreeMap::new();
    for item in status {
        let item = item.map_err(|error| discovery_error("cannot read Git status item", error))?;
        flag_item(&found.ignore, &item, &mut flags)?;
    }
    Ok(flags)
}

/// Flags the source of a staged rename, which status names only as the rename's source.
fn flag_rename_source(
    ignore: &IgnoreRules,
    item: &gix::status::Item,
    flags: &mut BTreeMap<String, OverlayFlags>,
) -> Result<(), DiscoveryError> {
    if let gix::status::Item::TreeIndex(gix::diff::index::Change::Rewrite {
        source_location,
        copy: false,
        ..
    }) = item
    {
        let source = path_string(source_location)?;
        if !ignored(ignore, &source) {
            flags.entry(source).or_default().staged = true;
        }
    }
    Ok(())
}

/// Flags the path a status item names, staged or changed in the worktree, unless Rerail ignores it.
fn flag_item(
    ignore: &IgnoreRules,
    item: &gix::status::Item,
    flags: &mut BTreeMap<String, OverlayFlags>,
) -> Result<(), DiscoveryError> {
    flag_rename_source(ignore, item, flags)?;
    let path = path_string(item.location())?;
    if ignored(ignore, &path) {
        return Ok(());
    }
    let flag = flags.entry(path).or_default();
    match item {
        gix::status::Item::TreeIndex(_) => flag.staged = true,
        gix::status::Item::IndexWorktree(_) => flag.worktree = true,
    }
    Ok(())
}

/// Flags each untracked path Rerail does not ignore; an untracked repository's goes to
/// `nested_paths` too.
fn flag_untracked(
    found: &Discovery,
    entries: Vec<gix::dir::Entry>,
    flags: &mut BTreeMap<String, OverlayFlags>,
    nested_paths: &mut BTreeSet<String>,
) -> Result<(), DiscoveryError> {
    for entry in entries {
        let path = path_string(entry.rela_path.as_ref())?;
        if ignored(&found.ignore, &path) {
            continue;
        }
        if matches!(
            entry.disk_kind,
            Some(gix::dir::entry::Kind::Directory | gix::dir::entry::Kind::Repository)
        ) && found.worktree.join(&path).join(".git").exists()
        {
            nested_paths.insert(path.clone());
        }
        flags.entry(path).or_default().untracked = true;
    }
    Ok(())
}

/// What status finds changed against HEAD and the index, and each untracked path, from the walk for
/// untracked files on a thread beside it, as gix's status runs the two.
fn status_flags(
    found: &Discovery,
    index: &gix::worktree::Index,
    head: &mut HeadWalk,
) -> Result<BTreeMap<String, OverlayFlags>, DiscoveryError> {
    let patterns = status_patterns(&found.scope);
    let walker = found.repository.clone().into_sync();
    let (changes, untracked) = rerail_accel::exec::scoped(|scope| {
        let walk = scope.spawn(|| untracked_entries(&walker.to_thread_local(), index, &patterns));
        let changes = changed_items(found, index, head.tree, &patterns);
        // A panic of the walk goes on in this thread, as it would have had the walk run here.
        let untracked = match walk.join() {
            Ok(untracked) => untracked,
            Err(panic) => std::panic::resume_unwind(panic),
        };
        (changes, untracked)
    });
    let mut flags = changes?;
    flag_untracked(found, untracked?, &mut flags, &mut head.nested_paths)?;
    Ok(flags)
}

/// Status never compares these entries with the worktree (assume-unchanged, skip-worktree), so
/// nothing proves their worktree bytes are the committed ones: they are changed.
fn uncompared_flags(
    index: &gix::index::State,
    ignore: &IgnoreRules,
    flags: &mut BTreeMap<String, OverlayFlags>,
) -> Result<(), DiscoveryError> {
    for entry in index.entries() {
        if !entry.flags.intersects(UNCOMPARED_INDEX_ENTRY) {
            continue;
        }
        let path = path_string(entry.path(index))?;
        if !ignored(ignore, &path) {
            flags.entry(path).or_default().worktree = true;
        }
    }
    Ok(())
}

/// The index entry stat of each base file a restore may verify by one `lstat`. A racy entry
/// (written in the same timestamp tick as the index) proves nothing about later writes, and a
/// size-preserving conversion makes the checkout bytes differ from the blob's at equal size: both
/// leave the file to be hashed when restored, as does every file where the system attributes file
/// cannot be named.
fn checkout_stats(
    repository: &gix::Repository,
    index: &gix::index::State,
    base_files: &mut [BaseFile],
) -> Result<(), DiscoveryError> {
    if let Some(mut attributes) = conversion_attributes(repository, index)? {
        record_checkout_stats(&mut attributes, index, base_files)?;
    }
    Ok(())
}

/// Each flagged path as its overlay: untracked, deleted, or staged and changed as status found it.
fn overlays(worktree: &Path, flags: BTreeMap<String, OverlayFlags>) -> Vec<WorkspaceOverlay> {
    flags
        .into_iter()
        .map(|(path, flags)| {
            let exists = worktree.join(&path).is_file();
            let kind = if flags.untracked {
                OverlayKind::Untracked
            } else if !exists {
                OverlayKind::Deleted
            } else if flags.staged && flags.worktree {
                OverlayKind::StagedAndUnstaged
            } else if flags.staged {
                OverlayKind::Staged
            } else {
                OverlayKind::Unstaged
            };
            WorkspaceOverlay { path, kind }
        })
        .collect()
}

/// A nested checkout (a gitlink or an untracked repository) is judged by its own repository, never
/// folded into this one; its directory is overlaid so no clean tree claims it.
fn nested_roots(
    worktree: &Path,
    ignore: &IgnoreRules,
    nested_paths: BTreeSet<String>,
    overlays: &mut Vec<WorkspaceOverlay>,
) -> Vec<PathBuf> {
    nested_paths
        .into_iter()
        .filter(|path| !mandatory_nested_ignored(ignore, path))
        .filter(|path| {
            let root = worktree.join(path);
            root.is_dir() && root.join(".git").exists()
        })
        .map(|path| {
            if !overlays.iter().any(|overlay| overlay.path == path) {
                overlays.push(WorkspaceOverlay {
                    path: path.clone(),
                    kind: OverlayKind::Untracked,
                });
            }
            worktree.join(path)
        })
        .collect()
}

/// The workspace a discovery found, sorted by path and object identity.
fn workspace(
    found: Discovery,
    head: HeadWalk,
    flags: BTreeMap<String, OverlayFlags>,
) -> Result<GitWorkspace, DiscoveryError> {
    let HeadWalk {
        tree,
        mut trees,
        mut base_files,
        nested_paths,
    } = head;
    let mut overlays = overlays(&found.worktree, flags);
    let nested_roots = nested_roots(&found.worktree, &found.ignore, nested_paths, &mut overlays);
    let common_directory = found.repository.common_dir().to_path_buf();
    let hash_algorithm = hash_algorithm(found.repository.object_hash())?;
    let root_tree = hash_key(tree.as_ref())?;
    overlays.sort_by(|left, right| left.path.cmp(&right.path));
    trees.sort_by_key(|tree| tree.id);
    base_files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(GitWorkspace {
        worktree_root: found.worktree,
        common_directory,
        hash_algorithm,
        root_tree,
        trees,
        base_files,
        overlays,
        nested_roots,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_store::test_support::TemporaryDatabase;

    #[test]
    fn discovers_current_repository_without_a_git_process() {
        let workspace = discover(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
        assert!(workspace.worktree_root.is_absolute());
        assert!(workspace.common_directory.is_absolute());
        assert!(!workspace.trees.is_empty());
        assert!(
            workspace
                .base_files
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
        );
        assert!(
            workspace
                .base_files
                .iter()
                .any(|file| file.path == "native/src/lib.rs"),
            "source files: {:?}",
            workspace
                .base_files
                .iter()
                .filter(|file| file.path.starts_with("native/"))
                .take(20)
                .map(|file| &file.path)
                .collect::<Vec<_>>()
        );
        assert!(
            workspace
                .base_files
                .iter()
                .all(|file| !file.path.starts_with("tmp/"))
        );
        let clean_trees = workspace.clean_trees();
        assert!(clean_trees.iter().all(|(prefix, _)| {
            !workspace.overlays.iter().any(|overlay| {
                prefix.is_empty()
                    || overlay.path == *prefix
                    || overlay.path.starts_with(&format!("{prefix}/"))
            })
        }));
        let indexed = workspace.index_files();
        assert!(indexed.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert!(workspace.overlays.iter().all(|overlay| {
            overlay.kind == OverlayKind::Deleted
                || indexed
                    .iter()
                    .any(|(path, clean)| path == &overlay.path && !clean)
        }));
        let source = workspace.read_source("native/build.rs", u64::MAX).unwrap();
        assert!(!source.is_empty());
        let alias = workspace
            .source_alias("native/build.rs", &source)
            .expect("unchanged source has Git alias");

        let (_database, store) = temporary_store("discovery");
        let registration = published_workspace();
        let published = workspace
            .publish_snapshot(&store, 1, HashKey::sha256(b"grammar"), 1, &registration)
            .unwrap();
        assert_eq!(
            workspace
                .publish_snapshot(&store, 1, HashKey::sha256(b"grammar"), 1, &registration)
                .unwrap(),
            published
        );
        let source_id = workspace.store_source(&store, "native/build.rs").unwrap();
        assert_eq!(store.source_by_alias(alias).unwrap(), Some(source_id));
    }

    fn run_git(directory: &Path, arguments: &[&str]) {
        // The machine's git settings (a signing key, hooks, a default branch) are not the test's.
        let output = std::process::Command::new("git")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .arg("-C")
            .arg(directory)
            .args(arguments)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git -C {} {} failed:\n{}",
            directory.display(),
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn initialize_repository(root: &Path, files: &[(&str, &[u8])]) {
        std::fs::create_dir_all(root).unwrap();
        run_git(root, &["init", "--initial-branch=main"]);
        run_git(root, &["config", "user.email", "rerail-tests@example.com"]);
        run_git(root, &["config", "user.name", "Rerail tests"]);
        for (path, contents) in files {
            std::fs::write(root.join(path), contents).unwrap();
        }
        let mut paths = vec!["add", "--"];
        paths.extend(files.iter().map(|(path, _)| *path));
        run_git(root, &paths);
        run_git(root, &["commit", "-m", "initial"]);
    }

    fn fixture_directory(name: &str) -> PathBuf {
        static NEXT_FIXTURE: std::sync::atomic::AtomicUsize =
            std::sync::atomic::AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "rerail-git-workspace-{name}-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))
    }

    /// Takes away this account's right to list `folder`, as another account has it on a records
    /// folder several accounts share: it may still pass through and make what it makes there.
    #[cfg(unix)]
    fn deny_listing(folder: &Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o300)).unwrap();
    }

    #[cfg(unix)]
    fn allow_listing(folder: &Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    /// The same on Windows: an entry that denies this account listing `folder` and nothing more.
    #[cfg(windows)]
    fn deny_listing(folder: &Path) {
        icacls_for_this_user(folder, "/deny", ":(RD)");
    }

    #[cfg(windows)]
    fn allow_listing(folder: &Path) {
        icacls_for_this_user(folder, "/remove:d", "");
    }

    #[cfg(windows)]
    fn icacls_for_this_user(folder: &Path, action: &str, rights: &str) {
        let user = std::env::var("USERNAME").unwrap();
        let status = std::process::Command::new("icacls")
            .arg(folder)
            .arg(action)
            .arg(format!("{user}{rights}"))
            .status()
            .unwrap();
        assert!(status.success());
    }

    /// What `git status --porcelain --untracked-files=all` prints, as (two-letter code, path)
    /// sorted by path.
    fn porcelain(root: &Path) -> Vec<(String, String)> {
        let output = std::process::Command::new("git")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .arg("-C")
            .arg(root)
            .args(["status", "--porcelain", "--untracked-files=all"])
            .output()
            .unwrap();
        assert!(output.status.success());
        // Git prints the changed paths first, then the untracked ones.
        let mut lines: Vec<(String, String)> = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| (line[..2].to_owned(), line[3..].to_owned()))
            .collect();
        lines.sort_by(|left, right| left.1.cmp(&right.1));
        lines
    }

    /// The overlays as `git status --porcelain` codes them, for the kinds the tests below make.
    fn overlay_codes(workspace: &GitWorkspace) -> Vec<(String, String)> {
        workspace
            .overlays
            .iter()
            .map(|overlay| {
                let code = match overlay.kind {
                    OverlayKind::Untracked => "??",
                    OverlayKind::Unstaged => " M",
                    OverlayKind::Staged => "M ",
                    OverlayKind::StagedAndUnstaged => "MM",
                    OverlayKind::Deleted => " D",
                };
                (code.to_owned(), overlay.path.clone())
            })
            .collect()
    }

    #[test]
    fn a_records_folder_this_account_may_not_list_is_skipped_as_git_skips_it() {
        let root = fixture_directory("unlistable-records");
        initialize_repository(
            &root,
            &[("kept.py", b"kept\n"), ("changed.py", b"before\n")],
        );
        std::fs::write(root.join("changed.py"), b"after\n").unwrap();
        std::fs::write(root.join("new.py"), b"new\n").unwrap();
        let records = root.join(".rerail").join("sessions");
        std::fs::create_dir_all(records.join("another-accounts-session")).unwrap();
        std::fs::write(records.join(".gitignore"), b"*\n").unwrap();
        std::fs::write(
            records.join("another-accounts-session/events.jsonl"),
            b"{}\n",
        )
        .unwrap();
        std::fs::write(root.join(".rerail/beside.txt"), b"beside\n").unwrap();

        deny_listing(&records);
        let discovered = discover(&root);
        let status = porcelain(&root);
        allow_listing(&records);

        let workspace = discovered.unwrap();
        assert_eq!(overlay_codes(&workspace), status);
        assert_eq!(
            status,
            [
                ("??", ".rerail/beside.txt"),
                (" M", "changed.py"),
                ("??", "new.py")
            ]
            .map(|(code, path)| (code.to_owned(), path.to_owned()))
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Takes away this account's right to read `file`'s attributes, as another account has it on a
    /// file in a records folder after the folder's repair; on Unix its folder's mode does that.
    #[cfg(windows)]
    fn deny_attributes(file: &Path) {
        icacls_for_this_user(file, "/deny", ":(RA)");
    }

    #[cfg(windows)]
    fn allow_attributes(file: &Path) {
        icacls_for_this_user(file, "/remove:d", "");
    }

    #[cfg(unix)]
    fn deny_attributes(file: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let folder = file.parent().unwrap();
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o000)).unwrap();
    }

    #[cfg(unix)]
    fn allow_attributes(file: &Path) {
        allow_listing(file.parent().unwrap());
    }

    #[test]
    fn a_records_folder_git_made_whose_file_this_account_may_not_read_is_left_out() {
        let root = fixture_directory("unreadable-records");
        initialize_repository(&root, &[("kept.py", b"kept\n")]);
        let records = root.join(".rerail").join("sessions");
        std::fs::create_dir_all(&records).unwrap();
        std::fs::write(records.join("notes.md"), b"kept\n").unwrap();
        run_git(&root, &["add", "--", ".rerail/sessions/notes.md"]);
        run_git(&root, &["commit", "-m", "a file in the records folder"]);
        std::fs::write(records.join(".gitignore"), b"*\n").unwrap();
        std::fs::write(root.join("new.py"), b"new\n").unwrap();
        let status = porcelain(&root);

        deny_attributes(&records.join("notes.md"));
        let discovered = discover(&root);
        allow_attributes(&records.join("notes.md"));

        let workspace = discovered.unwrap();
        assert_eq!(overlay_codes(&workspace), status);
        assert_eq!(status, [("??".to_owned(), "new.py".to_owned())]);
        assert!(
            workspace
                .base_files
                .iter()
                .all(|file| !file.path.starts_with(".rerail/"))
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_folder_git_ignores_that_this_account_may_not_list_is_never_listed() {
        let root = fixture_directory("unlistable-ignored");
        initialize_repository(
            &root,
            &[(".gitignore", b"build/\n"), ("kept.py", b"kept\n")],
        );
        let build = root.join("build");
        std::fs::create_dir(&build).unwrap();
        std::fs::write(build.join("out.o"), b"out\n").unwrap();

        deny_listing(&build);
        let discovered = discover(&root);
        allow_listing(&build);

        assert!(discovered.unwrap().overlays.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_folder_git_does_not_ignore_that_this_account_may_not_list_still_stops_discovery() {
        let root = fixture_directory("unlistable-untracked");
        initialize_repository(&root, &[("kept.py", b"kept\n")]);
        let folder = root.join("private");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("notes.txt"), b"notes\n").unwrap();

        deny_listing(&folder);
        let discovered = discover(&root);
        allow_listing(&folder);

        let error = discovered
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default();
        assert!(error.contains("cannot read Git status item"), "{error}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staged_rename_removes_the_source_path() {
        let root = fixture_directory("staged-rename");
        initialize_repository(&root, &[("old.py", b"print('old')\n")]);
        run_git(&root, &["mv", "old.py", "new.py"]);

        let workspace = discover(&root).unwrap();

        assert_eq!(workspace.index_files(), vec![("new.py".to_owned(), false)]);
        assert!(workspace.read_source("old.py", u64::MAX).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discovers_clean_php_sources() {
        let root = fixture_directory("php");
        initialize_repository(&root, &[("source.php", b"<?php echo 'rerail';\n")]);

        let workspace = discover(&root).unwrap();

        assert_eq!(
            workspace.index_files(),
            vec![("source.php".to_owned(), true)]
        );
        assert_eq!(
            workspace.read_source("source.php", u64::MAX).unwrap(),
            b"<?php echo 'rerail';\n"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reads_worktree_bytes_when_a_clean_file_changes_after_discovery() {
        let root = fixture_directory("changed-after-discovery");
        initialize_repository(&root, &[("source.py", b"print('before')\n")]);
        let workspace = discover(&root).unwrap();
        std::fs::write(root.join("source.py"), b"print('after')\n").unwrap();

        assert_eq!(
            workspace.read_source("source.py", u64::MAX).unwrap(),
            b"print('after')\n"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reads_checkout_bytes_for_clean_eol_transformed_sources() {
        let root = fixture_directory("eol");
        initialize_repository(
            &root,
            &[
                (".gitattributes", b"*.py text eol=crlf\n"),
                ("source.py", b"print('crlf')\r\n"),
            ],
        );
        let workspace = discover(&root).unwrap();

        assert_eq!(
            workspace.read_source("source.py", u64::MAX).unwrap(),
            b"print('crlf')\r\n"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Discovery compares the index with the HEAD tree it walked: a commit that lands after the
    /// walk overlays the files it changed, so their walked blobs never alias the new bytes.
    #[test]
    fn a_commit_during_discovery_overlays_the_files_it_changed() {
        let root = fixture_directory("commit-during-discovery");
        initialize_repository(
            &root,
            &[
                ("changed.py", b"print('a')\n"),
                ("kept.py", b"print('kept')\n"),
            ],
        );
        let committer = root.clone();
        AFTER_HEAD_TREE_VISIT.set(Some(Box::new(move || {
            std::fs::write(committer.join("changed.py"), b"print('b')\n").unwrap();
            run_git(&committer, &["commit", "-qam", "during discovery"]);
        })));

        let workspace = discover(&root).unwrap();

        assert!(
            AFTER_HEAD_TREE_VISIT.take().is_none(),
            "the commit landed inside discovery"
        );
        assert_eq!(
            workspace.overlays,
            [WorkspaceOverlay {
                path: "changed.py".to_owned(),
                kind: OverlayKind::Staged,
            }],
            "the file the commit changed differs from the walked HEAD tree"
        );
        assert_eq!(
            workspace.clean_source_alias("changed.py"),
            None,
            "a file committed after the walk is not restored from the walked blob"
        );
        assert!(workspace.clean_source_alias("kept.py").is_some());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// An untracked scratch repository without commits cannot be discovered; the outer worktree
    /// still is, with the scratch directory as untracked content that is read, never restored.
    #[test]
    fn an_untracked_repository_without_commits_is_untracked_content() {
        let root = fixture_directory("unborn-nested");
        std::fs::create_dir_all(root.join("lib")).unwrap();
        initialize_repository(&root, &[("lib/a.py", b"print('a')\n")]);
        let scratch = root.join("scratch");
        std::fs::create_dir_all(&scratch).unwrap();
        run_git(&scratch, &["init", "--initial-branch=main"]);
        std::fs::write(scratch.join("x.py"), b"print('x')\n").unwrap();

        let workspace = match discover(&root) {
            Ok(workspace) => workspace,
            Err(error) => panic!("an unborn nested repository failed discovery: {error}"),
        };

        assert_eq!(
            workspace.overlays,
            [WorkspaceOverlay {
                path: "scratch".to_owned(),
                kind: OverlayKind::Untracked,
            }]
        );
        assert_eq!(workspace.clean_source_alias("scratch/x.py"), None);
        assert!(workspace.clean_source_alias("lib/a.py").is_some());
        let clean = workspace
            .clean_trees()
            .into_iter()
            .map(|(prefix, _)| prefix)
            .collect::<Vec<_>>();
        assert_eq!(clean, ["lib"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A submodule checkout is a separate repository, never folded into the outer worktree: its
    /// files are not base files, and its directory is an overlaid nested root, so neither it nor
    /// its ancestors are clean.
    #[test]
    fn a_submodule_checkout_is_an_overlaid_nested_root() {
        let root = fixture_directory("undiscoverable-submodule");
        let submodule = root.join("submodule");
        let parent = root.join("parent");
        initialize_repository(&submodule, &[("nested.py", b"print('nested')\n")]);
        std::fs::create_dir_all(parent.join("lib")).unwrap();
        std::fs::create_dir_all(parent.join("vendor")).unwrap();
        initialize_repository(
            &parent,
            &[
                ("lib/a.py", b"print('a')\n"),
                ("vendor/other.py", b"print('other')\n"),
            ],
        );
        run_git(
            &parent,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "../submodule",
                "vendor/sub",
            ],
        );
        run_git(&parent, &["commit", "-am", "add submodule"]);

        let workspace = discover(&parent).unwrap();

        assert_eq!(
            workspace.nested_roots,
            [dunce::canonicalize(parent).unwrap().join("vendor/sub")]
        );

        assert_eq!(
            workspace.overlays,
            [WorkspaceOverlay {
                path: "vendor/sub".to_owned(),
                kind: OverlayKind::Untracked,
            }]
        );
        assert_eq!(workspace.clean_source_alias("vendor/sub/nested.py"), None);
        assert!(
            !workspace
                .base_files
                .iter()
                .any(|file| file.path.starts_with("vendor/sub/"))
        );
        assert!(workspace.clean_source_alias("vendor/other.py").is_some());
        let clean = workspace
            .clean_trees()
            .into_iter()
            .map(|(prefix, _)| prefix)
            .collect::<Vec<_>>();
        assert_eq!(clean, ["lib"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mandatory_ignore_prunes_nested_repositories_before_opening_them() {
        let root = fixture_directory("mandatory-nested-ignore");
        let submodule = root.join("submodule");
        let parent = root.join("parent");
        initialize_repository(&submodule, &[("source.py", b"print('nested')\n")]);
        std::fs::create_dir(submodule.join(".rerailignore")).unwrap();
        initialize_repository(&parent, &[("root.py", b"print('root')\n")]);
        run_git(
            &parent,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "../submodule",
                "node_modules",
            ],
        );
        run_git(&parent, &["commit", "-am", "add submodule"]);

        let workspace = discover(&parent).unwrap();
        assert!(workspace.nested_roots.is_empty());

        assert!(
            !workspace
                .index_files()
                .iter()
                .any(|(path, _)| path.starts_with("node_modules/"))
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discovers_linked_worktree_submodule_and_ignores_a_stale_gitfile() {
        static NEXT_FIXTURE: std::sync::atomic::AtomicUsize =
            std::sync::atomic::AtomicUsize::new(0);

        let fixture = std::env::temp_dir().join(format!(
            "rerail-git-worktree-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let parent = fixture.join("parent");
        let submodule = fixture.join("submodule");
        let linked = fixture.join("linked");
        initialize_repository(&submodule, &[("nested.rs", b"pub fn nested() {}\n")]);
        initialize_repository(&parent, &[("root.rs", b"pub fn root() {}\n")]);
        run_git(
            &parent,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "../submodule",
                "nested",
            ],
        );
        run_git(&parent, &["commit", "-am", "add submodule"]);
        run_git(
            &parent,
            &["worktree", "add", "--detach", linked.to_str().unwrap()],
        );
        run_git(
            &linked,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "update",
                "--init",
            ],
        );

        let linked_root = dunce::canonicalize(&linked).unwrap();
        let workspace = discover(&linked).unwrap();
        assert_eq!(workspace.worktree_root, linked_root);
        assert!(
            !workspace
                .base_files
                .iter()
                .any(|file| file.path.starts_with("nested/"))
        );
        assert_eq!(workspace.nested_roots, [linked_root.join("nested")]);
        assert!(
            workspace
                .overlays
                .iter()
                .any(|overlay| overlay.path == "nested")
        );

        std::fs::copy(parent.join("nested/.git"), linked.join("nested/.git")).unwrap();
        let workspace = discover(&linked).unwrap();
        assert_eq!(
            workspace.worktree_root,
            dunce::canonicalize(&linked).unwrap()
        );
        assert!(
            !workspace
                .base_files
                .iter()
                .any(|file| file.path == "nested/nested.rs")
        );

        std::fs::remove_dir_all(fixture).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn dirty_source_reads_reject_symlink_escapes() {
        use std::os::unix::fs::symlink;
        let root =
            std::env::temp_dir().join(format!("rerail-anchored-read-{}", std::process::id()));
        let outside =
            std::env::temp_dir().join(format!("rerail-anchored-outside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("source.py"), b"print('escaped')\n").unwrap();
        symlink(&outside, root.join("escape")).unwrap();
        let workspace = GitWorkspace {
            worktree_root: root.clone(),
            common_directory: root.clone(),
            hash_algorithm: GitHashAlgorithm::Sha1,
            root_tree: HashKey::git_sha1([0; 20]),
            trees: Vec::new(),
            base_files: Vec::new(),
            overlays: vec![WorkspaceOverlay {
                path: "escape/source.py".to_owned(),
                kind: OverlayKind::Untracked,
            }],
            nested_roots: Vec::new(),
        };
        assert!(workspace.read_source("escape/source.py", u64::MAX).is_err());
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(outside).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn dirty_source_reads_reject_fifos_without_blocking() {
        let root = fixture_directory("fifo");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("pipe.py");
        let path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        let workspace = GitWorkspace {
            worktree_root: root.clone(),
            common_directory: root.clone(),
            hash_algorithm: GitHashAlgorithm::Sha1,
            root_tree: HashKey::git_sha1([0; 20]),
            trees: Vec::new(),
            base_files: Vec::new(),
            overlays: vec![WorkspaceOverlay {
                path: "pipe.py".to_owned(),
                kind: OverlayKind::Untracked,
            }],
            nested_roots: Vec::new(),
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            sender
                .send(workspace.read_source("pipe.py", u64::MAX).is_err())
                .unwrap();
        });

        assert_eq!(
            receiver.recv_timeout(std::time::Duration::from_millis(250)),
            Ok(true)
        );
        reader.join().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rerailignore_uses_gitignore_negation() {
        let mut workspace = gix::ignore::Search::default();
        workspace
            .add_patterns_buffer(
                b"generated/\n!generated/keep.py\n!tmp/keep.py\n",
                ".rerailignore",
                None,
                gix::ignore::search::Ignore::default(),
            )
            .expect("the patterns parse");
        let mut mandatory = gix::ignore::Search::default();
        mandatory
            .add_patterns_buffer(
                BUILTIN_IGNORE,
                ".rerailignore.defaults",
                None,
                gix::ignore::search::Ignore::default(),
            )
            .expect("the patterns parse");
        let rules = IgnoreRules {
            mandatory,
            workspace,
            case: gix::ignore::glob::pattern::Case::Sensitive,
        };
        assert!(ignored(&rules, "generated/drop.py"));
        assert!(!ignored(&rules, "generated/keep.py"));
        assert!(ignored(&rules, "tmp/keep.py"));
    }
    #[test]
    fn clean_trees_comparisons_are_linear_for_many_trees_and_overlays() {
        let count = 2_000;
        let workspace = GitWorkspace {
            worktree_root: PathBuf::new(),
            common_directory: PathBuf::new(),
            hash_algorithm: GitHashAlgorithm::Sha1,
            root_tree: HashKey::git_sha1([0; 20]),
            trees: (0..count)
                .map(|index| GitTree {
                    id: HashKey::git_sha1([index as u8; 20]),
                    prefix: format!("tree-{index}"),
                    entries: Vec::new(),
                })
                .collect(),
            base_files: Vec::new(),
            // Overlays touch every even tree and as many directories without a tree. The dirty
            // set is a hash set of directories: each tree is one lookup, not a scan of overlays.
            overlays: (0..count)
                .map(|index| WorkspaceOverlay {
                    path: if index % 2 == 0 {
                        format!("tree-{index}/file.py")
                    } else {
                        format!("other-{index}/file.py")
                    },
                    kind: OverlayKind::Untracked,
                })
                .collect(),
            nested_roots: Vec::new(),
        };
        let clean = workspace
            .clean_trees()
            .into_iter()
            .map(|(prefix, _)| prefix)
            .collect::<Vec<_>>();
        let odd = (1..count)
            .step_by(2)
            .map(|index| format!("tree-{index}"))
            .collect::<Vec<_>>();
        assert_eq!(clean, odd);
    }

    #[test]
    fn a_tree_whose_prefix_an_overlay_names_is_not_clean() {
        let tree = |prefix: &str, byte: u8| GitTree {
            id: HashKey::git_sha1([byte; 20]),
            prefix: prefix.to_owned(),
            entries: Vec::new(),
        };
        let workspace = GitWorkspace {
            worktree_root: PathBuf::new(),
            common_directory: PathBuf::new(),
            hash_algorithm: GitHashAlgorithm::Sha1,
            root_tree: HashKey::git_sha1([0; 20]),
            trees: vec![
                tree("", 0),
                tree("vendor", 1),
                tree("vendor/lib", 2),
                tree("src", 3),
            ],
            base_files: Vec::new(),
            // A gitlink or an untracked nested repository overlays the directory itself.
            overlays: vec![WorkspaceOverlay {
                path: "vendor/lib".to_owned(),
                kind: OverlayKind::Untracked,
            }],
            nested_roots: Vec::new(),
        };
        let clean = workspace
            .clean_trees()
            .into_iter()
            .map(|(prefix, _)| prefix)
            .collect::<Vec<_>>();
        assert_eq!(clean, ["src"]);
    }

    fn tree_with_entries(byte: u8, prefix: &str, entries: Vec<GitTreeEntry>) -> GitTree {
        GitTree {
            id: HashKey::git_sha1([byte; 20]),
            prefix: prefix.to_owned(),
            entries,
        }
    }

    fn file_entries(byte: u8, count: u16) -> Vec<GitTreeEntry> {
        (0..count)
            .map(|index| {
                let mut target = [byte; 20];
                target[..2].copy_from_slice(&index.to_be_bytes());
                GitTreeEntry {
                    name: format!("file-{index}.py"),
                    mode: 0o100644,
                    target: HashKey::git_sha1(target),
                }
            })
            .collect()
    }

    fn directory_entry(tree: &GitTree) -> GitTreeEntry {
        GitTreeEntry {
            name: tree.prefix.clone(),
            mode: 0o040000,
            target: tree.id,
        }
    }

    /// A workspace over `trees` in publication (post-) order, the root last.
    fn tree_workspace(trees: Vec<GitTree>) -> GitWorkspace {
        GitWorkspace {
            worktree_root: PathBuf::new(),
            common_directory: PathBuf::new(),
            hash_algorithm: GitHashAlgorithm::Sha1,
            root_tree: trees.last().unwrap().id,
            trees,
            base_files: Vec::new(),
            overlays: Vec::new(),
            nested_roots: Vec::new(),
        }
    }

    /// A store on a fresh temporary database; dropping the path removes the database.
    fn temporary_store(label: &str) -> (TemporaryDatabase, ContentStore) {
        let path = TemporaryDatabase::new(&format!("rerail-gix-{label}"));
        let store = ContentStore::open(&path).unwrap();
        (path, store)
    }

    /// Another process's maintenance collects after every Git-tree write transaction.
    fn collect_after_every_tree_transaction(path: &Path) {
        let collector = ContentStore::open(path).unwrap();
        crate::content_store::AFTER_GIT_TREE_TRANSACTION.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                collector
                    .collect(0, crate::content_store::MAX_COLLECTION_BATCH)
                    .unwrap();
            }));
        });
    }

    fn stop_collecting() {
        crate::content_store::AFTER_GIT_TREE_TRANSACTION.with(|hook| *hook.borrow_mut() = None);
    }

    fn published_workspace() -> WorkspaceRegistration<'static> {
        WorkspaceRegistration {
            root: "/published",
            repository: "repository",
            owner: "owner",
            generation: 1,
            lease_seconds: i64::MAX,
        }
    }

    /// The snapshot a publication returned still exists after the collections that ran inside the
    /// publication: its workspace re-registers over it.
    fn assert_published_snapshot_attaches(
        store: &ContentStore,
        published: StoreResult<(SnapshotId, WorkspaceId)>,
    ) {
        let (snapshot, workspace) = match published {
            Ok(published) => published,
            Err(error) => panic!("a concurrent collection failed snapshot publication: {error:?}"),
        };
        let attached =
            store.open_workspace("/published", "repository", "owner", snapshot, 1, i64::MAX);
        assert!(
            attached.is_ok(),
            "a concurrent collection deleted the published snapshot before its workspace \
             registered: {:?}",
            attached.err()
        );
        assert_eq!(attached.unwrap(), workspace);
    }

    /// A republication (which repairs anything collected mid-publication) whose workspace holds
    /// the snapshot keeps the whole tree graph through a collection.
    fn assert_snapshot_graph_is_live(workspace: &GitWorkspace, store: &ContentStore) {
        let grammar = HashKey::sha256(b"grammar");
        let registration = published_workspace();
        workspace
            .publish_snapshot(store, 1, grammar, 1, &registration)
            .unwrap();
        store
            .collect(0, crate::content_store::MAX_COLLECTION_BATCH)
            .unwrap();
        store.reset_git_tree_write_count();
        workspace
            .publish_snapshot(store, 1, grammar, 1, &registration)
            .unwrap();
        assert_eq!(
            store.git_tree_write_count(),
            0,
            "collection removed trees of a published snapshot"
        );
    }

    #[test]
    fn publication_survives_a_collection_of_its_still_unreachable_trees() {
        let (path, store) = temporary_store("collect-fresh-trees");
        let child = tree_with_entries(81, "src", file_entries(81, 8));
        let root = tree_with_entries(82, "", vec![directory_entry(&child)]);
        let workspace = tree_workspace(vec![child, root]);
        collect_after_every_tree_transaction(&path);
        let published = workspace.publish_snapshot(
            &store,
            1,
            HashKey::sha256(b"grammar"),
            1,
            &published_workspace(),
        );
        stop_collecting();
        assert_published_snapshot_attaches(&store, published);
        assert_snapshot_graph_is_live(&workspace, &store);
    }

    #[test]
    fn publication_survives_a_collection_of_its_already_present_root() {
        let (path, store) = temporary_store("collect-present-root");
        // The first child alone exceeds one publication transaction's entry budget.
        let large = tree_with_entries(83, "large", file_entries(83, 600));
        let small = tree_with_entries(84, "small", file_entries(84, 4));
        let root = tree_with_entries(
            85,
            "",
            vec![directory_entry(&large), directory_entry(&small)],
        );
        // A retired snapshot left the root complete but unreachable, and its children missing.
        store
            .put_git_trees_bulk(std::slice::from_ref(&root))
            .unwrap();
        let workspace = tree_workspace(vec![large, small, root]);
        collect_after_every_tree_transaction(&path);
        let published = workspace.publish_snapshot(
            &store,
            1,
            HashKey::sha256(b"grammar"),
            1,
            &published_workspace(),
        );
        stop_collecting();
        assert_published_snapshot_attaches(&store, published);
        assert_snapshot_graph_is_live(&workspace, &store);
    }
}
