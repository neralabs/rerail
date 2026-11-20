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

const BUILTIN_IGNORE: &[u8] = b".git/\nnode_modules/\ntarget/\ntmp/\n__pycache__/\n";
const STATUS_EXCLUDES: &[&str] = &[
    ":(glob,exclude)**/.git/**",
    ":(glob,exclude)**/node_modules/**",
    ":(glob,exclude)**/target/**",
    ":(glob,exclude)**/tmp/**",
    ":(glob,exclude)**/__pycache__/**",
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
/// the size and modification time (`touch -r`) still changes the change time on Unix. On Windows
/// the stat holds the creation time in that field, so `VerifiedFiles::proof` adds the file's own
/// change time.
const RESTORE_STAT: gix::index::entry::stat::Options = gix::index::entry::stat::Options {
    trust_ctime: true,
    check_stat: true,
    use_nsec: true,
    use_stdev: false,
};
/// The checkout conversions that can change a file's bytes and keep its size. An end-of-line
/// conversion only inserts carriage returns, so a size check sees it; these need a hash.
const SIZE_PRESERVING_CONVERSIONS: [&str; 3] = ["filter", "ident", "working-tree-encoding"];

// This file compiles in rerail-store through `#[path]`, so its submodules are named by path.
#[path = "git_workspace/worktree_stat.rs"]
mod worktree_stat;

pub use worktree_stat::VerifiedFiles;
#[cfg(windows)]
pub use worktree_stat::recalls_data;
use worktree_stat::{ChangeStamp, Proof, WorktreeStat, stat_decides};

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

/// Sets `path`'s modification time to `when`, as for a file last written then and left alone
/// since. On Windows its change time goes there too: setting a time moves NTFS's change time to
/// now, which such a file does not have.
#[cfg(any(test, feature = "test-hooks"))]
pub fn backdate(path: &Path, when: std::time::SystemTime) {
    let file = std::fs::File::options().write(true).open(path).unwrap();
    #[cfg(not(windows))]
    file.set_modified(when).unwrap();
    #[cfg(windows)]
    set_written(&file, when);
}

#[cfg(all(any(test, feature = "test-hooks"), windows))]
fn set_written(file: &std::fs::File, when: std::time::SystemTime) {
    let time = file_time_of(when);
    set_basic_info(
        file,
        windows_sys::Win32::Storage::FileSystem::FILE_BASIC_INFO {
            LastWriteTime: time,
            ChangeTime: time,
            ..Default::default()
        },
    );
}

/// Sets `path`'s NTFS change time to `when` and leaves its other times, as a file whose times a
/// tool set has one that differs from its modification time.
#[cfg(all(any(test, feature = "test-hooks"), windows))]
pub fn set_change_time(path: &Path, when: std::time::SystemTime) {
    let file = std::fs::File::options().write(true).open(path).unwrap();
    set_basic_info(
        &file,
        windows_sys::Win32::Storage::FileSystem::FILE_BASIC_INFO {
            ChangeTime: file_time_of(when),
            ..Default::default()
        },
    );
}

/// Tests only: whether the case `name` of the test module `module` (its `module_path!()`) ran in a
/// process of its own with a Git setup installed, as the product's host installs one when it
/// starts. On Windows no clean file takes the restore's stat shortcut until a setup names the
/// system attributes file Git reads (`git_access::opener::system_attributes`), and a setup holds
/// for its whole process, so there the case runs again in a child process whose outcome this
/// asserts: `true`. In that child, and on every other system, `false`: the case runs here.
#[cfg(all(any(test, feature = "test-hooks"), not(windows)))]
pub fn ran_with_a_git_setup(_module: &str, _name: &str) -> bool {
    false
}

/// Tests only, on Windows: as above.
#[cfg(all(any(test, feature = "test-hooks"), windows))]
pub fn ran_with_a_git_setup(module: &str, name: &str) -> bool {
    const SYSTEM_CONFIG: &str = "RERAIL_TEST_GIT_SYSTEM_CONFIG";
    if let Some(config) = std::env::var_os(SYSTEM_CONFIG) {
        crate::git_access::setup::install(std::sync::Arc::new(crate::git_access::setup::GitSetup {
            system_config: Some(PathBuf::from(config)),
            installation_config: None,
        }));
        return false;
    }
    let folder =
        std::env::temp_dir().join(format!("rerail-git-setup-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("gitconfig"), b"").unwrap();
    let test = format!("{}::{name}", module.split_once("::").map_or(module, |(_, path)| path));
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([test.as_str(), "--exact", "--nocapture", "--test-threads=1"])
        .env(SYSTEM_CONFIG, folder.join("gitconfig"))
        .output()
        .unwrap();
    std::fs::remove_dir_all(&folder).unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "{test} in a process with a Git setup:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

/// `when` as a file time: 100 ns intervals from 1601.
#[cfg(all(any(test, feature = "test-hooks"), windows))]
fn file_time_of(when: std::time::SystemTime) -> i64 {
    let since_1601 = when
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        / 100
        + 116_444_736_000_000_000;
    i64::try_from(since_1601).unwrap()
}

/// Sets the times `info` names on `file`; a time left at 0 stays as it is.
#[cfg(all(any(test, feature = "test-hooks"), windows))]
fn set_basic_info(
    file: &std::fs::File,
    info: windows_sys::Win32::Storage::FileSystem::FILE_BASIC_INFO,
) {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_BASIC_INFO, FileBasicInfo, SetFileInformationByHandle,
    };
    // SAFETY: an open handle with write access, and a buffer of the size given.
    let set = unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileBasicInfo,
            (&raw const info).cast(),
            std::mem::size_of::<FILE_BASIC_INFO>() as u32,
        )
    };
    assert_ne!(set, 0, "{}", std::io::Error::last_os_error());
}

/// Sets `path`'s attributes to `attributes`, as a provider that moves a file's data off this disk
/// sets them; this moves its change time. `FILE_ATTRIBUTE_NORMAL` clears them.
#[cfg(all(any(test, feature = "test-hooks"), windows))]
pub fn set_attributes(path: &Path, attributes: u32) {
    let file = std::fs::File::options().write(true).open(path).unwrap();
    set_basic_info(
        &file,
        windows_sys::Win32::Storage::FileSystem::FILE_BASIC_INFO {
            FileAttributes: attributes,
            ..Default::default()
        },
    );
}

/// While it lives, everyone is denied the read of a file, as a person denies it with `icacls`:
/// the denial moves the file's change time and leaves its modification time and size.
#[cfg(all(any(test, feature = "test-hooks"), windows))]
pub struct ReadDenied(PathBuf);

#[cfg(all(any(test, feature = "test-hooks"), windows))]
impl ReadDenied {
    pub fn new(path: &Path) -> Self {
        let output = icacls(path, &["/deny", "*S-1-1-0:(R)"]).unwrap();
        assert!(
            output.status.success(),
            "icacls: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        Self(path.to_path_buf())
    }
}

#[cfg(all(any(test, feature = "test-hooks"), windows))]
impl Drop for ReadDenied {
    /// Takes the denial off, so the test's folder can be removed.
    fn drop(&mut self) {
        let _ = icacls(&self.0, &["/remove:d", "*S-1-1-0"]);
    }
}

#[cfg(all(any(test, feature = "test-hooks"), windows))]
fn icacls(path: &Path, change: &[&str]) -> std::io::Result<std::process::Output> {
    std::process::Command::new("icacls")
        .arg(path)
        .args(change)
        .output()
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

    /// What restores remember, in `store`, of this worktree's clean files whose change time moved
    /// after their last write (`holds_blob_bytes`).
    pub fn verified_files<'a>(&self, store: &'a ContentStore) -> VerifiedFiles<'a> {
        VerifiedFiles::new(Some(store), &self.worktree_root)
    }

    /// Whether the worktree file at `relative_path` holds exactly its committed blob's bytes, which
    /// are `blob_length` long, now. One `lstat` decides while the file's stat equals the stat of
    /// its index entry (`BaseFile::checkout`) and, on Windows, nothing changed the file after its
    /// last write (`VerifiedFiles::proof`); otherwise the file is hashed. The file may have changed
    /// since discovery, and a checkout conversion (`core.autocrlf`, a filter driver) keeps a file
    /// clean in Git while its bytes differ from the blob's.
    pub fn holds_blob_bytes(
        &self,
        relative_path: &str,
        blob_length: u64,
        verified: &mut VerifiedFiles<'_>,
    ) -> bool {
        let Ok(index) = self
            .base_files
            .binary_search_by(|file| file.path.as_str().cmp(relative_path))
        else {
            return false;
        };
        let Some(current) = WorktreeStat::read(&self.worktree_root.join(relative_path)) else {
            return false;
        };
        if !current.metadata.is_file() || current.metadata.len() != blob_length {
            return false;
        }
        let base = &self.base_files[index];
        if !base
            .checkout
            .is_some_and(|recorded| current.matches(&recorded))
        {
            return self.hashes_to_blob(relative_path, blob_length);
        }
        match verified.proof(relative_path, base.blob, &current) {
            Proof::Held => true,
            Proof::Read(stamp) => self.read_decides(relative_path, blob_length, stamp, verified),
        }
    }

    /// Whether the file's bytes hash to its committed blob; a file that cannot be read does not.
    fn hashes_to_blob(&self, relative_path: &str, blob_length: u64) -> bool {
        self.read_source(relative_path, blob_length)
            .is_ok_and(|bytes| self.source_alias(relative_path, &bytes).is_some())
    }

    /// Whether a file whose stat equals the stat Git recorded, and which only a read can prove,
    /// holds its blob's bytes: a hash decides, and `verified` remembers one equal to the blob
    /// under `stamp`. A file this process cannot read cannot be hashed: its stat decides, as it
    /// does for Git (`stat_decides`).
    fn read_decides(
        &self,
        relative_path: &str,
        blob_length: u64,
        stamp: Option<ChangeStamp>,
        verified: &mut VerifiedFiles<'_>,
    ) -> bool {
        let read_from = std::time::SystemTime::now();
        match self.read_source(relative_path, blob_length) {
            Err(error) => stat_decides(&error),
            Ok(bytes) => {
                let blob = self.source_alias(relative_path, &bytes);
                if let Some(blob) = blob {
                    verified.learn(relative_path, stamp, blob, read_from);
                }
                blob.is_some()
            }
        }
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

fn discover_in(root: &Path, scoped: bool) -> Result<GitWorkspace, DiscoveryError> {
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
    let case = if repository
        .filesystem_options()
        .map_err(|error| discovery_error("cannot read core.ignoreCase", error))?
        .ignore_case
    {
        gix::ignore::glob::pattern::Case::Fold
    } else {
        gix::ignore::glob::pattern::Case::Sensitive
    };
    let ignore = load_ignore(&worktree, case)?;
    let root_tree = repository
        .head_tree()
        .map_err(|error| discovery_error("cannot read HEAD tree", error))?;
    // Status compares the index with this same tree: re-reading HEAD there would miss a commit
    // that lands during the walk, leaving its files clean against blobs of the walked tree.
    let head_tree = root_tree.id().detach();
    let root_tree_id = hash_key(head_tree.as_ref())?;
    let mut trees = Vec::new();
    let mut base_files = Vec::new();
    let mut nested_paths = BTreeSet::new();
    // Scoped: the root's path in the worktree ("" at the top), its subtree, and a pathspec for status.
    let scope = if scoped {
        let canonical = dunce::canonicalize(root)
            .map_err(|error| discovery_error("cannot resolve the root", error))?;
        canonical
            .strip_prefix(&worktree)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        String::new()
    };
    if scope.is_empty() {
        visit_tree(
            &repository,
            root_tree,
            "",
            &ignore,
            &mut trees,
            &mut base_files,
            &mut nested_paths,
        )?;
    } else if let Some(entry) = root_tree
        .lookup_entry_by_path(&scope)
        .map_err(|error| discovery_error("cannot read HEAD tree", error))?
        .filter(|entry| entry.mode().is_tree())
    {
        let subtree = entry
            .object()
            .map_err(|error| discovery_error("cannot read Git subtree", error))?
            .try_into_tree()
            .map_err(|error| discovery_error("Git subtree has the wrong object kind", error))?;
        visit_tree(
            &repository,
            subtree,
            &scope,
            &ignore,
            &mut trees,
            &mut base_files,
            &mut nested_paths,
        )?;
    }
    #[cfg(any(test, feature = "test-hooks"))]
    if let Some(hook) = AFTER_HEAD_TREE_VISIT.take() {
        hook();
    }

    let mut flags: BTreeMap<String, OverlayFlags> = BTreeMap::new();
    let index = repository
        .index_or_empty()
        .map_err(|error| discovery_error("cannot read Git index", error))?;
    let status = repository
        .status(gix::progress::Discard)
        .map_err(|error| discovery_error("cannot prepare Git status", error))?
        .index(gix::worktree::IndexPersistedOrInMemory::Persisted(
            index.clone(),
        ))
        .untracked_files(gix::status::UntrackedFiles::Files)
        .index_worktree_submodules(None)
        .head_tree(head_tree)
        .into_iter(
            (!scope.is_empty())
                .then(|| gix::bstr::BString::from(format!(":(top,literal){scope}")))
                .into_iter()
                .chain(
                    STATUS_EXCLUDES
                        .iter()
                        .map(|pattern| gix::bstr::BString::from(*pattern)),
                ),
        )
        .map_err(|error| discovery_error("cannot read Git status", error))?;
    for item in status {
        let item = item.map_err(|error| discovery_error("cannot read Git status item", error))?;
        if let gix::status::Item::TreeIndex(gix::diff::index::Change::Rewrite {
            source_location,
            copy: false,
            ..
        }) = &item
        {
            let source = path_string(source_location)?;
            if !ignored(&ignore, &source) {
                flags.entry(source).or_default().staged = true;
            }
        }
        let path = path_string(item.location())?;
        if ignored(&ignore, &path) {
            continue;
        }
        match item {
            gix::status::Item::TreeIndex(_) => flags.entry(path).or_default().staged = true,
            gix::status::Item::IndexWorktree(
                gix::status::index_worktree::Item::DirectoryContents { entry, .. },
            ) => {
                if matches!(
                    entry.disk_kind,
                    Some(gix::dir::entry::Kind::Directory | gix::dir::entry::Kind::Repository)
                ) && worktree.join(&path).join(".git").exists()
                {
                    nested_paths.insert(path.clone());
                }
                flags.entry(path).or_default().untracked = true;
            }
            gix::status::Item::IndexWorktree(_) => {
                flags.entry(path).or_default().worktree = true;
            }
        }
    }
    // Status never compares these entries with the worktree (assume-unchanged, skip-worktree), so
    // nothing proves their worktree bytes are the committed ones: they are changed.
    for entry in index.entries() {
        if !entry.flags.intersects(UNCOMPARED_INDEX_ENTRY) {
            continue;
        }
        let path = path_string(entry.path(&index))?;
        if !ignored(&ignore, &path) {
            flags.entry(path).or_default().worktree = true;
        }
    }
    // The index entry stat of each base file a restore may verify by one `lstat`. A racy entry
    // (written in the same timestamp tick as the index) proves nothing about later writes, and a
    // size-preserving conversion makes the checkout bytes differ from the blob's at equal size:
    // both leave the file to be hashed when restored, as does every file where the system
    // attributes file cannot be named.
    if let Some(mut attributes) = conversion_attributes(&repository, &index)? {
        record_checkout_stats(&mut attributes, &index, &mut base_files)?;
    }

    let mut overlays: Vec<WorkspaceOverlay> = flags
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
        .collect();
    // A nested checkout (a gitlink or an untracked repository) is judged by its own repository,
    // never folded into this one; its directory is overlaid so no clean tree claims it.
    let nested_roots = nested_paths
        .into_iter()
        .filter(|path| !mandatory_nested_ignored(&ignore, path))
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
        .collect::<Vec<_>>();
    let common_directory = repository.common_dir().to_path_buf();
    let repository_hash_algorithm = hash_algorithm(repository.object_hash())?;
    overlays.sort_by(|left, right| left.path.cmp(&right.path));
    trees.sort_by_key(|tree| tree.id);
    base_files.sort_by(|left, right| left.path.cmp(&right.path));

    Ok(GitWorkspace {
        worktree_root: worktree,
        common_directory,
        hash_algorithm: repository_hash_algorithm,
        root_tree: root_tree_id,
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

    /// A repository at `root` whose first commit holds the files the case wrote at `paths`, with the
    /// times it gave them (`initialize_repository` writes its files itself).
    fn initialize_repository_with(root: &Path, paths: &[&str]) {
        run_git(root, &["init", "--initial-branch=main"]);
        run_git(root, &["config", "user.email", "rerail-tests@example.com"]);
        run_git(root, &["config", "user.name", "Rerail tests"]);
        run_git(root, &[&["add", "--"][..], paths].concat());
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

    /// A clean file's stat proves it holds its blob's bytes until something writes it. A write
    /// that keeps the size and puts the modification time back (`touch -r`) ends that proof on
    /// every system, so the file is hashed and its new bytes are not taken for the blob's.
    #[test]
    fn a_clean_files_stat_proves_its_bytes_until_a_same_size_write_puts_its_time_back() {
        if ran_with_a_git_setup(
            module_path!(),
            "a_clean_files_stat_proves_its_bytes_until_a_same_size_write_puts_its_time_back",
        ) {
            return;
        }
        let root = fixture_directory("stat-proof");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("a.py");
        std::fs::write(&path, b"pront(1)\n").unwrap();
        let written = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_577_836_800);
        backdate(&path, written);
        initialize_repository_with(&root, &["a.py"]);
        let workspace = discover(&root).unwrap();
        let recorded = workspace.base_files[0]
            .checkout
            .expect("the entry of a file written before the index is not racy");
        assert!(
            WorktreeStat::read(&path).unwrap().proves(&recorded),
            "an untouched file is proven by its stat alone, with no hash"
        );
        let mut memory = VerifiedFiles::new(None, &root);
        assert!(workspace.holds_blob_bytes("a.py", 9, &mut memory));

        std::fs::write(&path, b"print(1)\n").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(written)
            .unwrap();
        assert!(!WorktreeStat::read(&path).unwrap().proves(&recorded));
        assert!(
            !workspace.holds_blob_bytes("a.py", 9, &mut memory),
            "bytes written after Git compared the file are not its blob's"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A clean file whose read a person denied after Git compared it cannot be hashed: its stat
    /// alone decides, as it does for Git. The denial moves NTFS's change time, so the stronger
    /// proof cannot decide for it.
    #[cfg(windows)]
    #[test]
    fn a_clean_file_whose_read_is_denied_is_proven_by_its_stat_alone() {
        if ran_with_a_git_setup(
            module_path!(),
            "a_clean_file_whose_read_is_denied_is_proven_by_its_stat_alone",
        ) {
            return;
        }
        let root = fixture_directory("stat-denied");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("a.py");
        std::fs::write(&path, b"print(1)\n").unwrap();
        backdate(
            &path,
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_577_836_800),
        );
        initialize_repository_with(&root, &["a.py"]);
        let workspace = discover(&root).unwrap();
        let mut memory = VerifiedFiles::new(None, &root);
        {
            let _denied = ReadDenied::new(&path);
            assert!(
                workspace.read_source("a.py", 9).is_err(),
                "the read is denied"
            );
            assert!(
                workspace.holds_blob_bytes("a.py", 9, &mut memory),
                "an unchanged file this process may not read keeps the answer of its stat"
            );
            assert!(
                !workspace.holds_blob_bytes("a.py", 10, &mut memory),
                "a stat of another size proves nothing"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A clean file another process holds without sharing it, or locks, cannot be hashed while it
    /// does: its stat alone decides, as it does for Git, once its change time moved.
    #[cfg(windows)]
    #[test]
    fn a_clean_file_held_by_another_handle_is_proven_by_its_stat_alone() {
        use std::os::windows::fs::OpenOptionsExt;
        if ran_with_a_git_setup(
            module_path!(),
            "a_clean_file_held_by_another_handle_is_proven_by_its_stat_alone",
        ) {
            return;
        }
        let root = fixture_directory("stat-held");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("a.py");
        std::fs::write(&path, b"print(1)\n").unwrap();
        let written = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_577_836_800);
        backdate(&path, written);
        initialize_repository_with(&root, &["a.py"]);
        let workspace = discover(&root).unwrap();
        let mut memory = VerifiedFiles::new(None, &root);
        // Setting the same time again moves the change time alone.
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(written)
            .unwrap();
        {
            let _held = std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(&path)
                .unwrap();
            assert!(
                workspace.read_source("a.py", 9).is_err(),
                "the file is held"
            );
            assert!(workspace.holds_blob_bytes("a.py", 9, &mut memory));
        }
        {
            let locked = std::fs::File::open(&path).unwrap();
            locked.lock().unwrap();
            assert!(
                workspace.read_source("a.py", 9).is_err(),
                "the file is locked"
            );
            assert!(workspace.holds_blob_bytes("a.py", 9, &mut memory));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The time a file in these cases was last written: long before any index write.
    #[cfg(windows)]
    fn written_long_ago() -> std::time::SystemTime {
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_577_836_800)
    }

    /// A committed repository at a fixture folder named `name` holding `a.py`, written long ago,
    /// with its discovered workspace.
    #[cfg(windows)]
    fn committed_long_ago(name: &str, bytes: &[u8]) -> (PathBuf, PathBuf, GitWorkspace) {
        let root = fixture_directory(name);
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("a.py");
        std::fs::write(&path, bytes).unwrap();
        backdate(&path, written_long_ago());
        initialize_repository_with(&root, &["a.py"]);
        let workspace = discover(&root).unwrap();
        (root, path, workspace)
    }

    /// Sets `path`'s creation time to `when`, as the file it replaced had.
    #[cfg(windows)]
    fn set_creation_time(path: &Path, when: std::time::SystemTime) {
        use std::os::windows::fs::FileTimesExt;
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_times(std::fs::FileTimes::new().set_created(when))
            .unwrap();
    }

    /// Writes `bytes` to `path` and puts its modification time back to `written_long_ago`, as
    /// `touch -r` does: its size and stat stay those Git recorded.
    #[cfg(windows)]
    fn rewrite_keeping_its_time(path: &Path, bytes: &[u8]) {
        std::fs::write(path, bytes).unwrap();
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(written_long_ago())
            .unwrap();
    }

    /// A clean file whose change time moved after its last write (its times set again, a copy or
    /// an extraction that kept them) is read once: the hash equal to its blob is remembered in
    /// the store, and the next restore, in a workspace of its own, takes the file with no read.
    /// A write that keeps the size and puts the modification time back moves the change time,
    /// so the restore after it reads the file and finds other bytes.
    #[cfg(windows)]
    #[test]
    fn a_clean_file_whose_change_time_moved_is_read_once_until_a_write() {
        if ran_with_a_git_setup(
            module_path!(),
            "a_clean_file_whose_change_time_moved_is_read_once_until_a_write",
        ) {
            return;
        }
        let (root, path, workspace) = committed_long_ago("stat-remembered", b"print(1)\n");
        let hour = std::time::Duration::from_secs(3_600);
        set_change_time(&path, written_long_ago() + hour);
        let (_database, store) = temporary_store("stat-remembered");
        let mut first = workspace.verified_files(&store);
        assert!(workspace.holds_blob_bytes("a.py", 9, &mut first));
        assert_eq!(
            first.learned(),
            1,
            "the first restore reads and hashes the file"
        );
        first.save();
        let mut second = discover(&root).unwrap().verified_files(&store);
        assert!(workspace.holds_blob_bytes("a.py", 9, &mut second));
        assert_eq!(
            second.learned(),
            0,
            "the next restore takes it with no read"
        );
        let current = WorktreeStat::read(&path).unwrap();
        let another_blob = second.proof("a.py", HashKey::git_sha1([7; 20]), &current);
        assert!(
            matches!(another_blob, Proof::Read(Some(_))),
            "what was remembered of one blob proves no other"
        );

        let remembered = WorktreeStat::read(&path).unwrap().stamp().unwrap();
        rewrite_keeping_its_time(&path, b"pront(1)\n");
        let written = WorktreeStat::read(&path).unwrap().stamp().unwrap();
        assert_ne!(
            written.changed, remembered.changed,
            "the write moved the change time"
        );
        let mut third = workspace.verified_files(&store);
        assert!(
            !workspace.holds_blob_bytes("a.py", 9, &mut third),
            "bytes written after the hash are not the blob's"
        );
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A file a rename put in a remembered file's place is another file, whatever change time
    /// it carries: its other bytes are read, not taken for the blob's.
    #[cfg(windows)]
    #[test]
    fn a_file_a_rename_put_in_place_is_read_whatever_its_change_time() {
        if ran_with_a_git_setup(
            module_path!(),
            "a_file_a_rename_put_in_place_is_read_whatever_its_change_time",
        ) {
            return;
        }
        let (root, path, workspace) = committed_long_ago("stat-renamed", b"print(1)\n");
        let changed = written_long_ago() + std::time::Duration::from_secs(3_600);
        set_change_time(&path, changed);
        let mut memory = VerifiedFiles::new(None, &root);
        assert!(workspace.holds_blob_bytes("a.py", 9, &mut memory));
        assert_eq!(memory.learned(), 1);
        let remembered = WorktreeStat::read(&path).unwrap().stamp().unwrap();

        let created = std::fs::metadata(&path).unwrap().created().unwrap();
        let other = root.join("other.py");
        rewrite_keeping_its_time(&other, b"pront(1)\n");
        std::fs::rename(&other, &path).unwrap();
        set_creation_time(&path, created);
        set_change_time(&path, changed);
        let recorded = workspace.base_files[0].checkout.unwrap();
        let renamed = WorktreeStat::read(&path).unwrap();
        assert!(renamed.matches(&recorded), "the stat Git recorded");
        let renamed = renamed.stamp().unwrap();
        assert_eq!(renamed.changed, remembered.changed, "the same change time");
        assert_ne!(renamed, remembered, "another file");
        assert!(
            !workspace.holds_blob_bytes("a.py", 9, &mut memory),
            "the renamed file's bytes are not the blob's"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A write through a remembered file's other name moves the change time both names share,
    /// so the restore reads the file again and finds the other bytes.
    #[cfg(windows)]
    #[test]
    fn a_write_through_a_hard_link_moves_the_change_time_its_other_name_shows() {
        if ran_with_a_git_setup(
            module_path!(),
            "a_write_through_a_hard_link_moves_the_change_time_its_other_name_shows",
        ) {
            return;
        }
        let (root, path, workspace) = committed_long_ago("stat-linked", b"print(1)\n");
        let link = root.join("b.py");
        std::fs::hard_link(&path, &link).unwrap();
        set_change_time(
            &path,
            written_long_ago() + std::time::Duration::from_secs(3_600),
        );
        let mut memory = VerifiedFiles::new(None, &root);
        assert!(workspace.holds_blob_bytes("a.py", 9, &mut memory));
        assert_eq!(memory.learned(), 1);
        let remembered = WorktreeStat::read(&path).unwrap().stamp().unwrap();

        rewrite_keeping_its_time(&link, b"pront(1)\n");
        let through_link = WorktreeStat::read(&path).unwrap().stamp().unwrap();
        assert_ne!(through_link.changed, remembered.changed);
        assert_eq!(
            through_link,
            WorktreeStat::read(&link).unwrap().stamp().unwrap(),
            "one file under two names"
        );
        assert!(
            !workspace.holds_blob_bytes("a.py", 9, &mut memory),
            "bytes written through the other name are not the blob's"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A clean file whose change time moved and whose attributes say its data is not all on this
    /// disk is taken by its stat with no query beyond the stat's own call and no read: a read
    /// would make its provider download its data. The same file with its data on disk is read.
    #[cfg(windows)]
    fn not_all_on_disk_is_taken_by_its_stat(name: &str, attributes: u32) {
        let (root, path, _) = committed_long_ago(name, b"print(1)\n");
        let blob = HashKey::git_sha1([7; 20]);
        let mut memory = VerifiedFiles::new(None, &root);
        let on_disk = WorktreeStat::as_if(&path, Some((2, 1)), 0);
        assert!(
            matches!(memory.proof("a.py", blob, &on_disk), Proof::Read(Some(_))),
            "a file whose change time moved is read"
        );
        let recalled = WorktreeStat::as_if(&path, Some((2, 1)), attributes);
        worktree_stat::queries_made();
        assert!(
            matches!(memory.proof("a.py", blob, &recalled), Proof::Held),
            "a file with attributes {attributes:#x} is taken by its stat"
        );
        assert_eq!(
            worktree_stat::queries_made(),
            0,
            "no query: {attributes:#x}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn a_cloud_file_recalled_on_data_access_is_taken_by_its_stat() {
        if ran_with_a_git_setup(
            module_path!(),
            "a_cloud_file_recalled_on_data_access_is_taken_by_its_stat",
        ) {
            return;
        }
        not_all_on_disk_is_taken_by_its_stat("stat-recall-data", 0x0040_0000);
    }

    #[cfg(windows)]
    #[test]
    fn a_cloud_file_recalled_on_open_is_taken_by_its_stat() {
        if ran_with_a_git_setup(
            module_path!(),
            "a_cloud_file_recalled_on_open_is_taken_by_its_stat",
        ) {
            return;
        }
        not_all_on_disk_is_taken_by_its_stat("stat-recall-open", 0x0004_0000);
    }

    #[cfg(windows)]
    #[test]
    fn an_offline_file_is_taken_by_its_stat() {
        if ran_with_a_git_setup(module_path!(), "an_offline_file_is_taken_by_its_stat") {
            return;
        }
        not_all_on_disk_is_taken_by_its_stat("stat-offline", 0x0000_1000);
    }

    /// A volume that keeps no change time (FAT and exFAT answer 0), or a file whose stat's call
    /// answered no times, leaves each clean file to its stat with no query beyond the stat's own
    /// call, on every restore.
    #[cfg(windows)]
    #[test]
    fn a_file_with_no_change_time_is_taken_by_its_stat_with_no_query() {
        if ran_with_a_git_setup(
            module_path!(),
            "a_file_with_no_change_time_is_taken_by_its_stat_with_no_query",
        ) {
            return;
        }
        let (root, path, _) = committed_long_ago("stat-no-change-time", b"print(1)\n");
        let blob = HashKey::git_sha1([7; 20]);
        let mut memory = VerifiedFiles::new(None, &root);
        worktree_stat::queries_made();
        for times in [Some((0, 1)), None] {
            let current = WorktreeStat::as_if(&path, times, 0);
            for _ in 0..2 {
                assert!(
                    matches!(memory.proof("a.py", blob, &current), Proof::Held),
                    "{times:?}"
                );
            }
        }
        assert_eq!(worktree_stat::queries_made(), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A restore asks a volume whether it keeps change times once: for files that give their
    /// identity, once per volume, and for files that give none, once in all. Every other proof
    /// makes the file id query alone.
    #[cfg(windows)]
    #[test]
    fn a_restore_asks_a_volume_about_its_change_times_once() {
        if ran_with_a_git_setup(
            module_path!(),
            "a_restore_asks_a_volume_about_its_change_times_once",
        ) {
            return;
        }
        let (root, path, _) = committed_long_ago("stat-volume-once", b"print(1)\n");
        let blob = HashKey::git_sha1([7; 20]);
        let moved = WorktreeStat::as_if(&path, Some((2, 1)), 0);
        for (identified, proof) in [(true, "Read(Some)"), (false, "Read(None)")] {
            let mut memory = VerifiedFiles::new(None, &root);
            worktree_stat::file_ids_answer(identified);
            worktree_stat::queries_made();
            for _ in 0..3 {
                let read = match memory.proof("a.py", blob, &moved) {
                    Proof::Read(stamp) => stamp.is_some() == identified,
                    Proof::Held => false,
                };
                assert!(read, "{proof}");
            }
            worktree_stat::file_ids_answer(true);
            assert_eq!(
                worktree_stat::queries_made(),
                3 + 2,
                "three file id queries and the volume's two, once: {proof}"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A change time not older than the read that hashed the file is not remembered: a write
    /// after the read could carry the same time. Each restore reads such a file.
    #[cfg(windows)]
    #[test]
    fn a_change_time_as_new_as_the_read_is_not_remembered() {
        if ran_with_a_git_setup(
            module_path!(),
            "a_change_time_as_new_as_the_read_is_not_remembered",
        ) {
            return;
        }
        let (root, path, workspace) = committed_long_ago("stat-recent", b"print(1)\n");
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(3_600);
        set_change_time(&path, later);
        let mut memory = VerifiedFiles::new(None, &root);
        assert!(workspace.holds_blob_bytes("a.py", 9, &mut memory));
        assert!(workspace.holds_blob_bytes("a.py", 9, &mut memory));
        assert_eq!(memory.learned(), 0, "the change time was not remembered");
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
