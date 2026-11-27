//! In-process Git and worktree discovery through `gix`.
//! Production code in this module never executes an external Git process.

use rerail_store::Canonical as _;
use crate::content_store::{GitTree, GitTreeEntry, HashKey};
#[path = "git_snapshot.rs"]
pub mod snapshot;

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

/// The folders no repository walk enters, by name: the host's `IGNORED_DIRECTORIES`
/// (`src/engine/code-rule/ignored-directories.ts`), which `npm run generate:ignored-directories` writes
/// here as data. The engine's corpus listing, its artifact inputs, its `file` facts and its server graphs
/// read it, so a `.rerail` run reads the files the host's walk reads.
pub fn ignored_directories() -> &'static [String] {
    static NAMES: std::sync::LazyLock<Vec<String>> = std::sync::LazyLock::new(|| {
        serde_json::from_str(include_str!("ignored_directories.generated.json"))
            .expect("ignored_directories.generated.json is a list of folder names")
    });
    &NAMES
}

/// Whether no repository walk enters a folder of this name (`ignored_directories`).
pub fn ignored_directory(name: &str) -> bool {
    ignored_directories().iter().any(|ignored| ignored == name)
}

/// Rerail's records folder (`records_folder`) is among them: Git status leaves a file tracked there
/// unread, which another account sharing the checkout may not read, and the walk never enters it.
pub const BUILTIN_IGNORE: &[u8] =
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
/// the size and modification time (`touch -r`) still changes the change time on Unix. On Windows
/// the stat holds the creation time in that field, so `VolumeAnswers::stat_proves` adds the file's
/// own change time.
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

#[cfg(windows)]
pub use worktree_stat::recalls_data;
pub use worktree_stat::VolumeAnswers;
use worktree_stat::{WorktreeStat, stat_decides};
pub use worktree_stat::{changed_before, file_clock};

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
        crate::git_access::setup::install(std::sync::Arc::new(
            crate::git_access::setup::GitSetup {
                system_config: Some(PathBuf::from(config)),
                installation_config: None,
            },
        ));
        return false;
    }
    let folder =
        std::env::temp_dir().join(format!("rerail-git-setup-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("gitconfig"), b"").unwrap();
    let test = format!(
        "{}::{name}",
        module.split_once("::").map_or(module, |(_, path)| path)
    );
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

/// A blob, and what a test does just before status reads the tracked file whose index entry names
/// it.
#[cfg(test)]
type TrackedFileReadHook = (gix::ObjectId, Box<dyn FnOnce() + Send>);

/// Each hook runs once, inside a discovery, just before status reads the tracked file whose index
/// entry names the hook's blob: a test rewrites the file there, after status took its size. Status
/// compares on threads of its own, so the hooks are the process's, keyed by a blob only one test
/// commits.
#[cfg(test)]
static BEFORE_TRACKED_FILE_READ: std::sync::Mutex<Vec<TrackedFileReadHook>> =
    std::sync::Mutex::new(Vec::new());

/// Runs, once, the hook a test set for the blob `entry` names.
#[cfg(test)]
fn before_tracked_file_read(entry: &gix::index::Entry) {
    let hook = {
        let mut hooks = BEFORE_TRACKED_FILE_READ.lock().unwrap();
        hooks
            .iter()
            .position(|(blob, _)| *blob == entry.id)
            .map(|at| hooks.swap_remove(at).1)
    };
    if let Some(hook) = hook {
        hook();
    }
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
    pub snapshot: Option<std::sync::Arc<snapshot::Snapshot>>,
    pub worktree_root: PathBuf,
    pub common_directory: PathBuf,
    pub hash_algorithm: GitHashAlgorithm,
    pub root_tree: HashKey,
    pub trees: Vec<GitTree>,
    pub base_files: Vec<BaseFile>,
    pub overlays: Vec<WorkspaceOverlay>,
    pub nested_roots: Vec<PathBuf>,
    /// Each volume's answer to whether it keeps a change time to trust, which restores
    /// (`holds_blob_bytes`) and looks at clean blobs (`clean_blob_unread`) ask, kept across their
    /// calls and this workspace's clones, so each volume is asked once, or once by each call that
    /// misses it while another asks. No call waits for another's queries (`VolumeAnswers`).
    pub volume_answers: std::sync::Arc<VolumeAnswers>,
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
    let root = root.canonical()?;
    let path = root.join(relative_path).canonical()?;
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

pub fn discovery_error(context: &str, error: impl fmt::Display) -> DiscoveryError {
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

pub struct IgnoreRules {
    pub mandatory: gix::ignore::Search,
    pub workspace: gix::ignore::Search,
    /// The repository's `core.ignoreCase` folding: Git status applies it to `STATUS_EXCLUDES`,
    /// so a path status hides is always ignored here too, never a clean base file.
    pub case: gix::ignore::glob::pattern::Case,
}

fn load_ignore(
    worktree: &Path,
    case: gix::ignore::glob::pattern::Case,
) -> Result<IgnoreRules, DiscoveryError> {
    let path = worktree.join(".rerailignore");
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(discovery_error("cannot read .rerailignore", error)),
    };
    ignore_patterns(worktree, case, &bytes)
}

fn ignore_patterns(
    worktree: &Path,
    case: gix::ignore::glob::pattern::Case,
    bytes: &[u8],
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
    workspace
        .add_patterns_buffer(
            bytes,
            worktree.join(".rerailignore"),
            Some(worktree),
            gix::ignore::search::Ignore::default(),
        )
        .map_err(|error| discovery_error("cannot load .rerailignore", error))?;
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
    search_ignored_path(search, path.as_bytes(), case, false)
}

fn search_ignored_path(
    search: &gix::ignore::Search,
    bytes: &[u8],
    case: gix::ignore::glob::pattern::Case,
    directory: bool,
) -> bool {
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
    if let Some(found) = search.pattern_matching_relative_path(bytes.as_bstr(), Some(directory), case) {
        ignored = !found.pattern.is_negative();
    }
    ignored
}

pub fn ignored(rules: &IgnoreRules, path: &str) -> bool {
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
    /// its index entry (`BaseFile::checkout`) and, on Windows, nothing changed the file after its
    /// last write, or the stat decides there as it does for Git (`VolumeAnswers::stat_proves`);
    /// otherwise the file is hashed. The file may have changed since discovery, and a checkout
    /// conversion (`core.autocrlf`, a filter driver) keeps a file clean in Git while its bytes
    /// differ from the blob's.
    pub fn holds_blob_bytes(&self, relative_path: &str, blob_length: u64) -> bool {
        if let Some(snapshot) = &self.snapshot {
            return snapshot.owns_blob(relative_path, blob_length);
        }
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
        let proven = self.volume_answers.stat_proves(&current);
        proven || self.read_decides(relative_path, blob_length)
    }

    /// Whether the file's bytes hash to its committed blob; a file that cannot be read does not.
    fn hashes_to_blob(&self, relative_path: &str, blob_length: u64) -> bool {
        self.read_source(relative_path, blob_length)
            .is_ok_and(|bytes| self.source_alias(relative_path, &bytes).is_some())
    }

    /// Whether a file whose stat equals the stat Git recorded, and which only a read can prove,
    /// holds its blob's bytes: a hash decides. A file this process cannot read cannot be hashed:
    /// its stat decides, as it does for Git (`stat_decides`).
    fn read_decides(&self, relative_path: &str, blob_length: u64) -> bool {
        match self.read_source(relative_path, blob_length) {
            Err(error) => stat_decides(&error),
            Ok(bytes) => self.source_alias(relative_path, &bytes).is_some(),
        }
    }

    /// A clean base file's blob, unread: the path has no overlay, its worktree stat is exactly its index entry's
    /// (`BaseFile::checkout`) and, on Windows, nothing changed the file after its last write, or the stat decides
    /// there as it does for Git (`VolumeAnswers::stat_proves`), so its bytes are the blob's (phase 0, t23). `None`
    /// where only a hash could tell. The volumes' answers are this workspace's own (`volume_answers`), so each volume
    /// is asked once, or once by each call that misses it while another asks.
    pub fn clean_blob_unread(&self, relative_path: &str) -> Option<HashKey> {
        let file = self.unedited_base_file(relative_path)?;
        let recorded = file.checkout?;
        let current = WorktreeStat::read(&self.worktree_root.join(relative_path))?;
        let proven = current.metadata.is_file()
            && current.matches(&recorded)
            && self.volume_answers.stat_proves(&current);
        proven.then_some(file.blob)
    }

    /// The base file at `relative_path` where no overlay replaces it.
    fn unedited_base_file(&self, relative_path: &str) -> Option<&BaseFile> {
        if self
            .overlays
            .binary_search_by(|o| o.path.as_str().cmp(relative_path))
            .is_ok()
        {
            return None;
        }
        let index = self
            .base_files
            .binary_search_by(|f| f.path.as_str().cmp(relative_path))
            .ok()?;
        Some(&self.base_files[index])
    }

    /// Every file of the worktree as Git sees it, relative and sorted: HEAD's (less those deleted), the staged new
    /// ones and the untracked ones Git does not ignore. Ignored directories are not entered; nested repositories are
    /// their own.
    pub fn worktree_files(&self) -> Vec<String> {
        const TREE: u16 = 0o040000;
        const COMMIT: u16 = 0o160000;
        let mut files: BTreeSet<String> = BTreeSet::new();
        for tree in &self.trees {
            for entry in tree.entries.iter().filter(|e| e.mode & 0o170000 != TREE && e.mode & 0o170000 != COMMIT) {
                files.insert(if tree.prefix.is_empty() { entry.name.clone() } else { format!("{}/{}", tree.prefix, entry.name) });
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

    /// Read the exact bytes that code-rule evaluation uses, refusing a file larger than `max_bytes`
    /// with `ErrorKind::FileTooLarge`. The bytes carry no Git identity: publication hashes them
    /// against the committed blob only when the path is clean (`source_alias`).
    pub fn read_source(&self, relative_path: &str, max_bytes: u64) -> std::io::Result<Vec<u8>> {
        check_source_path(relative_path)?;
        if let Some(snapshot) = &self.snapshot {
            return snapshot.read_source(relative_path, max_bytes);
        }
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
        if let Some(snapshot) = &self.snapshot {
            return snapshot.read_source_prefix(relative_path, prefix_bytes);
        }
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
    let canonical = |p: &Path| p.canonical().ok();
    Some((canonical(repository.workdir()?)?, canonical(repository.git_dir())?, canonical(repository.common_dir())?))
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
    let worktree = repository
        .workdir()
        .ok_or_else(|| {
            DiscoveryError::Failed("bare Git repositories have no workspace".to_owned())
        })?
        .canonical()
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
    let canonical = root
        .canonical()
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

/// Whether `path` is a records folder ([`RECORDS_FOLDER`]), its case folded as `case` folds it,
/// as the built-in ignore and status's pathspec fold theirs. Rerail writes a `.gitignore` of `*`
/// there, so Git lists nothing in it. On a checkout several accounts share, the folder lets each
/// other account pass through it and make its own folder there, but not list it, and that
/// `.gitignore` is its maker's alone: a walk by another account can neither list the folder nor
/// read that Git ignores what it holds. So no walk enters it, for any account.
pub fn records_folder(path: &[u8], case: gix::ignore::glob::pattern::Case) -> bool {
    let Some(start) = path.len().checked_sub(RECORDS_FOLDER.len()) else {
        return false;
    };
    let (parent, folder) = path.split_at(start);
    let named = match case {
        gix::ignore::glob::pattern::Case::Fold => {
            folder.eq_ignore_ascii_case(RECORDS_FOLDER.as_bytes())
        }
        gix::ignore::glob::pattern::Case::Sensitive => folder == RECORDS_FOLDER.as_bytes(),
    };
    named && (parent.is_empty() || parent.ends_with(b"/"))
}

/// What status's walk for untracked files emits, kept as gix's own status keeps it: never an entry
/// of a type Git cannot track (a socket, a FIFO). Unlike that walk, it never enters a records
/// folder ([`records_folder`]) nor names one: gix stops a walk at the first folder it cannot list,
/// where Git warns and goes on.
struct UntrackedWalk {
    entries: Vec<gix::dir::Entry>,
    /// The repository's `core.ignoreCase` folding, which [`records_folder`] applies.
    case: gix::ignore::glob::pattern::Case,
}

impl gix::dir::walk::Delegate for UntrackedWalk {
    fn emit(
        &mut self,
        entry: gix::dir::EntryRef<'_>,
        _collapsed_directory_status: Option<gix::dir::entry::Status>,
    ) -> gix::dir::walk::Action {
        if entry.disk_kind != Some(gix::dir::entry::Kind::Untrackable)
            && !records_folder(&entry.rela_path, self.case)
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
        !records_folder(&entry.rela_path, self.case)
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
/// gix's walk with status's own options and [`UntrackedWalk`] as its delegate, folding case as
/// `case` does.
fn untracked_entries(
    repository: &gix::Repository,
    index: &gix::index::State,
    patterns: &[gix::bstr::BString],
    case: gix::ignore::glob::pattern::Case,
) -> Result<Vec<gix::dir::Entry>, DiscoveryError> {
    let options = repository
        .dirwalk_options()
        .map_err(|error| discovery_error("cannot prepare Git status", error))?
        .emit_untracked(gix::dir::walk::EmissionMode::Matching);
    let mut walk = UntrackedWalk {
        entries: Vec::new(),
        case,
    };
    let interrupt = std::sync::atomic::AtomicBool::new(false);
    repository
        .dirwalk(index, patterns, &interrupt, options, &mut walk)
        .map_err(|error| discovery_error("cannot read Git status item", error))?;
    Ok(walk.entries)
}

/// Whether a comparison of a tracked file failed because the file changed after status took its
/// `lstat`: cut short before the read, which hashes exactly the size the `lstat` gave
/// (`UnexpectedEof`), or gone before the open (`NotFound`).
fn rewritten_while_read(error: &gix::Error) -> bool {
    error
        .downcast_any_ref::<std::io::Error>()
        .is_some_and(|error| {
            matches!(
                error.kind(),
                std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::NotFound
            )
        })
}

/// Compares a tracked file with its index entry as gix's own status does (`FastEq`), and takes for
/// changed a file another process rewrites or removes while status reads it
/// ([`rewritten_while_read`]). gix's status ends at the first comparison that fails, without
/// naming the file, so one log or lock file written at the wrong moment would fail the whole
/// discovery. A file taken for changed costs only its reuse: its bytes are read where they are
/// used.
#[derive(Clone)]
struct ChangedWhenRewritten;

impl gix::status::plumbing::index_as_worktree::traits::CompareBlobs for ChangedWhenRewritten {
    type Output = ();

    fn compare_blobs<'a, 'b>(
        &mut self,
        entry: &gix::index::Entry,
        worktree_blob_size: u64,
        data: impl gix::status::plumbing::index_as_worktree::traits::ReadData<'a>,
        buf: &mut Vec<u8>,
    ) -> gix::Result<Option<()>> {
        #[cfg(test)]
        before_tracked_file_read(entry);
        let compared = gix::status::plumbing::index_as_worktree::traits::FastEq.compare_blobs(
            entry,
            worktree_blob_size,
            data,
            buf,
        );
        match compared {
            Err(error) if rewritten_while_read(&error) => Ok(Some(())),
            compared => compared,
        }
    }
}

/// The index entries status finds changed in the worktree. An entry whose stat alone is stale
/// holds its blob's bytes: it is clean and left out, as gix's status iterator leaves it out.
struct WorktreeItems(Vec<gix::status::Item>);

impl<'index> gix::status::plumbing::index_as_worktree_with_renames::VisitEntry<'index>
    for WorktreeItems
{
    type ContentChange = ();
    type SubmoduleStatus = gix::submodule::Status;

    fn visit_entry(
        &mut self,
        entry: gix::status::plumbing::index_as_worktree_with_renames::Entry<
            'index,
            Self::ContentChange,
            Self::SubmoduleStatus,
        >,
    ) {
        use gix::status::plumbing::index_as_worktree::EntryStatus;
        use gix::status::plumbing::index_as_worktree_with_renames::Entry;
        if !matches!(
            entry,
            Entry::Modification {
                status: EntryStatus::NeedsUpdate(_),
                ..
            }
        ) {
            self.0.push(gix::status::Item::IndexWorktree(entry.into()));
        }
    }
}

/// What status finds staged under `patterns`: the index against HEAD's tree, the one the walk
/// read, with renames as the repository configures them. It stops at the next change once
/// `interrupt` is set.
fn staged_items(
    repository: &gix::Repository,
    index: &gix::index::State,
    head_tree: gix::ObjectId,
    patterns: &[gix::bstr::BString],
    interrupt: &std::sync::atomic::AtomicBool,
) -> Result<Vec<gix::status::Item>, DiscoveryError> {
    let mut pathspec = repository
        .pathspec(
            false,
            patterns,
            true,
            index,
            gix::worktree::stack::state::attributes::Source::WorktreeThenIdMapping,
        )
        .map_err(|error| discovery_error("cannot read Git status", error))?;
    let mut items = Vec::new();
    repository
        .tree_index_status(
            &head_tree,
            index,
            Some(&mut pathspec),
            gix::status::tree_index::TrackRenames::AsConfigured,
            |change, _, _| {
                items.push(gix::status::Item::TreeIndex(change.into_owned()));
                Ok(if interrupt.load(Ordering::Acquire) {
                    std::ops::ControlFlow::Break(())
                } else {
                    std::ops::ControlFlow::Continue(())
                })
            },
        )
        .map_err(|error| discovery_error("cannot read Git status item", error))?;
    Ok(items)
}

/// What status finds changed in the worktree under `patterns`: each tracked file against its index
/// entry, compared by [`ChangedWhenRewritten`]. No walk for untracked files, no rename tracking,
/// and no look inside a submodule. It stops once `interrupt` is set.
fn worktree_items(
    repository: &gix::Repository,
    index: &gix::index::State,
    patterns: &[gix::bstr::BString],
    interrupt: &std::sync::atomic::AtomicBool,
) -> Result<Vec<gix::status::Item>, DiscoveryError> {
    let submodules = gix::status::index_worktree::BuiltinSubmoduleStatus::new(
        repository.clone().into_sync(),
        gix::status::Submodule::Given {
            ignore: gix::submodule::config::Ignore::All,
            check_dirty: false,
        },
    )
    .map_err(|error| discovery_error("cannot read Git status", error))?;
    let mut items = WorktreeItems(Vec::new());
    repository
        .index_worktree_status(
            index,
            patterns,
            &mut items,
            ChangedWhenRewritten,
            submodules,
            &mut gix::progress::Discard,
            interrupt,
            gix::status::index_worktree::Options::default(),
        )
        .map_err(|error| discovery_error("cannot read Git status item", error))?;
    Ok(items.0)
}

/// What a scoped thread returned. Its panic goes on in this thread, as it would have had its work
/// run here.
fn joined<T>(thread: std::thread::ScopedJoinHandle<'_, T>) -> T {
    match thread.join() {
        Ok(returned) => returned,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

/// Each path Git status finds staged or changed in the worktree, with how: the two comparisons of
/// gix's own status ([`staged_items`] on a thread beside [`worktree_items`], as it runs them, the
/// first to fail stopping the other). Its walk for untracked files is [`untracked_entries`]'s.
fn changed_items(
    found: &Discovery,
    index: &gix::worktree::Index,
    head_tree: gix::ObjectId,
    patterns: &[gix::bstr::BString],
) -> Result<BTreeMap<String, OverlayFlags>, DiscoveryError> {
    let reader = found.repository.clone().into_sync();
    let interrupt = std::sync::atomic::AtomicBool::new(false);
    let interrupt_on_error = |result: &Result<_, _>| {
        if result.is_err() {
            interrupt.store(true, Ordering::Release);
        }
    };
    let (staged, changed) = rerail_accel::exec::scoped(|scope| {
        let staged = scope.spawn(|| {
            let staged = staged_items(
                &reader.to_thread_local(),
                index,
                head_tree,
                patterns,
                &interrupt,
            );
            interrupt_on_error(&staged);
            staged
        });
        let changed = worktree_items(&found.repository, index, patterns, &interrupt);
        interrupt_on_error(&changed);
        (joined(staged), changed)
    });
    let mut flags = BTreeMap::new();
    for item in staged?.iter().chain(&changed?) {
        flag_item(&found.ignore, item, &mut flags)?;
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
    let case = found.ignore.case;
    let (changes, untracked) = rerail_accel::exec::scoped(|scope| {
        let walk =
            scope.spawn(|| untracked_entries(&walker.to_thread_local(), index, &patterns, case));
        let changes = changed_items(found, index, head.tree, &patterns);
        (changes, joined(walk))
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
        snapshot: None,
        worktree_root: found.worktree,
        common_directory,
        hash_algorithm,
        root_tree,
        trees,
        base_files,
        overlays,
        nested_roots,
        volume_answers: Default::default(),
    })
}

// The tests that publish to a `ContentStore` are rerail-store's (`git_workspace_tests.rs`); a clean file's stat proof's
// are here, where the stat's own hooks are (`worktree_stat`).
#[cfg(test)]
mod tests {
    use super::*;

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
        for (path, contents) in files {
            std::fs::write(root.join(path), contents).unwrap();
        }
        let paths: Vec<&str> = files.iter().map(|(path, _)| *path).collect();
        initialize_repository_with(root, &paths);
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
        assert!(workspace.holds_blob_bytes("a.py", 9));

        std::fs::write(&path, b"print(1)\n").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(written)
            .unwrap();
        assert!(!WorktreeStat::read(&path).unwrap().proves(&recorded));
        assert!(
            !workspace.holds_blob_bytes("a.py", 9),
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
        {
            let _denied = ReadDenied::new(&path);
            assert!(
                workspace.read_source("a.py", 9).is_err(),
                "the read is denied"
            );
            assert!(
                workspace.holds_blob_bytes("a.py", 9),
                "an unchanged file this process may not read keeps the answer of its stat"
            );
            assert!(
                !workspace.holds_blob_bytes("a.py", 10),
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
            assert!(workspace.holds_blob_bytes("a.py", 9));
        }
        {
            let locked = std::fs::File::open(&path).unwrap();
            locked.lock().unwrap();
            assert!(
                workspace.read_source("a.py", 9).is_err(),
                "the file is locked"
            );
            assert!(workspace.holds_blob_bytes("a.py", 9));
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
    /// an extraction that kept them) is read and hashed at every restore, in this workspace and in
    /// the next: nothing is kept of an earlier hash. A write that keeps the size and puts the
    /// modification time back is read the same way, and its other bytes are not the blob's.
    #[cfg(windows)]
    #[test]
    fn a_clean_file_whose_change_time_moved_is_read_at_every_restore() {
        if ran_with_a_git_setup(
            module_path!(),
            "a_clean_file_whose_change_time_moved_is_read_at_every_restore",
        ) {
            return;
        }
        let (root, path, workspace) = committed_long_ago("stat-read-again", b"print(1)\n");
        let hour = std::time::Duration::from_secs(3_600);
        set_change_time(&path, written_long_ago() + hour);
        let next = discover(&root).unwrap();
        for (restore, restoring) in [(1, &workspace), (2, &workspace), (3, &next)] {
            reset_source_alias_hashes();
            assert!(restoring.holds_blob_bytes("a.py", 9), "restore {restore}");
            assert_eq!(
                source_alias_hashes(),
                1,
                "restore {restore} hashes the file"
            );
        }

        rewrite_keeping_its_time(&path, b"pront(1)\n");
        assert!(
            !workspace.holds_blob_bytes("a.py", 9),
            "bytes written after Git compared the file are not the blob's"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A write through a clean file's other name, at its size with the modification time put back,
    /// moves the change time both names share, which the restore reads from the file through a
    /// handle: under this name the stat no longer proves the file, and the read finds the other
    /// bytes.
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
        // The link moved the change time; give the file the times of one written long ago again.
        backdate(&path, written_long_ago());
        let recorded = workspace.base_files[0].checkout.unwrap();
        assert!(WorktreeStat::read(&path).unwrap().proves(&recorded));
        reset_source_alias_hashes();
        assert!(workspace.holds_blob_bytes("a.py", 9));
        assert_eq!(source_alias_hashes(), 0, "proven by its stat, unread");

        rewrite_keeping_its_time(&link, b"pront(1)\n");
        let through_link = WorktreeStat::read(&path).unwrap();
        assert!(through_link.matches(&recorded), "the stat Git recorded");
        assert!(!through_link.proves(&recorded), "the change time moved");
        assert!(
            !workspace.holds_blob_bytes("a.py", 9),
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
        let answers = VolumeAnswers::default();
        let on_disk = WorktreeStat::as_if(&path, Some((2, 1)), 0);
        assert!(
            !answers.stat_proves(&on_disk),
            "a file whose change time moved is read"
        );
        let recalled = WorktreeStat::as_if(&path, Some((2, 1)), attributes);
        worktree_stat::queries_made();
        assert!(
            answers.stat_proves(&recalled),
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
        let answers = VolumeAnswers::default();
        worktree_stat::queries_made();
        for times in [Some((0, 1)), None] {
            let current = WorktreeStat::as_if(&path, times, 0);
            for _ in 0..2 {
                assert!(answers.stat_proves(&current), "{times:?}");
            }
        }
        assert_eq!(worktree_stat::queries_made(), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Whether a volume keeps change times is asked once by proofs made one after another: for
    /// files that give their volume's serial number, once per volume, and for files that give none,
    /// once in all. Every other proof makes the serial number query alone.
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
        let moved = WorktreeStat::as_if(&path, Some((2, 1)), 0);
        for serial in [true, false] {
            let answers = VolumeAnswers::default();
            worktree_stat::volume_serials_answer(serial);
            worktree_stat::queries_made();
            for _ in 0..3 {
                assert!(
                    !answers.stat_proves(&moved),
                    "read; a serial number: {serial}"
                );
            }
            worktree_stat::volume_serials_answer(true);
            assert_eq!(
                worktree_stat::queries_made(),
                3 + 2,
                "three serial number queries and the volume's two, once; a serial number: {serial}"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A file on a network share whose change time moved keeps the stat's answer with no read: a
    /// share's change time is kept by the server's file system, so the stat decides there, as for
    /// Git.
    #[cfg(windows)]
    #[test]
    fn a_file_on_a_share_whose_change_time_moved_is_taken_by_its_stat() {
        if ran_with_a_git_setup(
            module_path!(),
            "a_file_on_a_share_whose_change_time_moved_is_taken_by_its_stat",
        ) {
            return;
        }
        let (root, path, _) = committed_long_ago("stat-share", b"print(1)\n");
        let moved = WorktreeStat::as_if(&path, Some((2, 1)), 0);
        let held = |on_a_share: bool| {
            worktree_stat::files_are_on_a_share(on_a_share);
            let proven = VolumeAnswers::default().stat_proves(&moved);
            worktree_stat::files_are_on_a_share(false);
            proven
        };
        assert!(held(true), "on a share");
        assert!(!held(false), "on this machine");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A clean file's blob is taken unread only where the restore's proof holds with no read: a
    /// same-size write that puts the modification time back keeps the stat Git recorded and moves
    /// NTFS's change time, so only a hash can tell.
    #[cfg(windows)]
    #[test]
    fn a_clean_blob_is_unread_only_while_nothing_changed_the_file_after_its_last_write() {
        if ran_with_a_git_setup(
            module_path!(),
            "a_clean_blob_is_unread_only_while_nothing_changed_the_file_after_its_last_write",
        ) {
            return;
        }
        let (root, path, workspace) = committed_long_ago("unread-blob", b"print(1)\n");
        let base = &workspace.base_files[0];
        assert_eq!(workspace.clean_blob_unread("a.py"), Some(base.blob));
        rewrite_keeping_its_time(&path, b"pront(1)\n");
        assert!(
            WorktreeStat::read(&path)
                .unwrap()
                .matches(&base.checkout.unwrap()),
            "the stat Git recorded"
        );
        assert_eq!(workspace.clean_blob_unread("a.py"), None, "other bytes");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A workspace asks a volume whether it keeps change times once across its restores and its
    /// looks at clean blobs made one after another: after the first, each one of a file whose
    /// change time moved makes the serial number query alone.
    #[cfg(windows)]
    #[test]
    fn a_workspaces_restores_and_looks_ask_a_volume_about_its_change_times_once() {
        if ran_with_a_git_setup(
            module_path!(),
            "a_workspaces_restores_and_looks_ask_a_volume_about_its_change_times_once",
        ) {
            return;
        }
        let (root, path, workspace) = committed_long_ago("unread-volume-once", b"print(1)\n");
        rewrite_keeping_its_time(&path, b"print(1)\n");
        worktree_stat::queries_made();
        assert!(
            workspace.holds_blob_bytes("a.py", 9),
            "read, the blob's bytes"
        );
        for _ in 0..3 {
            assert_eq!(workspace.clean_blob_unread("a.py"), None);
        }
        assert_eq!(
            worktree_stat::queries_made(),
            4 + 2,
            "four serial number queries and the volume's two, once"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A look at a clean blob answers while another look's volume query is held: the workspace's
    /// volume answers are locked only to read or keep an answer, never across a query. The second
    /// look misses the volume too, so it asks the volume itself; its answer is kept, and the next
    /// look makes the serial number query alone.
    #[cfg(windows)]
    #[test]
    fn a_look_answers_while_another_looks_volume_query_is_held() {
        if ran_with_a_git_setup(
            module_path!(),
            "a_look_answers_while_another_looks_volume_query_is_held",
        ) {
            return;
        }
        let (root, path, workspace) = committed_long_ago("unread-held-query", b"print(1)\n");
        rewrite_keeping_its_time(&path, b"print(1)\n");
        let workspace = &workspace;
        let (reached, held) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let (answered, answer) = std::sync::mpsc::channel();
        let (second, first) = std::thread::scope(|scope| {
            let first = scope.spawn(move || {
                worktree_stat::hold_the_next_volume_query(reached, released);
                workspace.clean_blob_unread("a.py")
            });
            held.recv().unwrap();
            scope.spawn(move || {
                let blob = workspace.clean_blob_unread("a.py");
                answered
                    .send((blob, worktree_stat::queries_made()))
                    .unwrap();
            });
            // Released whatever the second look did, so neither thread is left waiting.
            let second = answer.recv_timeout(std::time::Duration::from_secs(30));
            release.send(()).unwrap();
            (second, first.join().unwrap())
        });
        assert_eq!(
            second,
            Ok((None, 3)),
            "the second look asks the volume and answers while the first one's query is held: \
             a serial number query and the volume's two"
        );
        assert_eq!(first, None, "the first look, once released");
        worktree_stat::queries_made();
        assert_eq!(workspace.clean_blob_unread("a.py"), None);
        assert_eq!(
            worktree_stat::queries_made(),
            1,
            "the kept answer: the serial number query alone"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A file written again with the bytes Git recorded has another stat and is still clean:
    /// status reads it, finds its blob, and says only that the index entry's stat is stale.
    #[test]
    fn a_file_rewritten_with_its_committed_bytes_stays_clean() {
        let root = fixture_directory("rewritten-same-bytes");
        initialize_repository(&root, &[("source.py", b"print('same')\n")]);
        let path = root.join("source.py");
        std::fs::write(&path, b"print('same')\n").unwrap();
        backdate(
            &path,
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_577_836_800),
        );

        let workspace = discover(&root).unwrap();

        assert_eq!(workspace.overlays, []);
        assert!(workspace.clean_source_alias("source.py").is_some());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A repository at a fixture folder named `name` whose commit holds `racing.py` and `kept.py`.
    /// `racing.py` is then written with other bytes of the same size, so status has to read it,
    /// and `during_its_read` runs on its path just before status does. Returns the folder and the
    /// blob the hook waits for ([`ran_during_its_read`]).
    fn rewritten_while_status_reads_it(
        name: &str,
        during_its_read: impl FnOnce(&Path) + Send + 'static,
    ) -> (PathBuf, gix::ObjectId) {
        let root = fixture_directory(name);
        let committed = format!("print('{name}, committed')\n");
        initialize_repository(
            &root,
            &[
                ("racing.py", committed.as_bytes()),
                ("kept.py", b"print('kept')\n"),
            ],
        );
        let racing = root.join("racing.py");
        std::fs::write(&racing, committed.replace("committed", "rewritten")).unwrap();
        let blob = gix::objs::compute_hash(
            gix::hash::Kind::Sha1,
            gix::objs::Kind::Blob,
            committed.as_bytes(),
        )
        .unwrap();
        BEFORE_TRACKED_FILE_READ
            .lock()
            .unwrap()
            .push((blob, Box::new(move || during_its_read(&racing))));
        (root, blob)
    }

    /// Whether the hook set for `blob` ran: status read the file, so the case raced it.
    fn ran_during_its_read(blob: gix::ObjectId) -> bool {
        !BEFORE_TRACKED_FILE_READ
            .lock()
            .unwrap()
            .iter()
            .any(|(waiting, _)| *waiting == blob)
    }

    /// Status hashes a tracked file by reading the size its `lstat` gave. A file another process
    /// cuts short in between (a log, a lock file, a file a formatter is writing) is changed, and
    /// the rest of the discovery stands.
    #[test]
    fn a_tracked_file_cut_short_while_status_reads_it_is_changed() {
        let (root, blob) = rewritten_while_status_reads_it("cut-short", |racing| {
            std::fs::write(racing, b"short\n").unwrap();
        });

        let workspace = discover(&root).unwrap();

        assert!(
            ran_during_its_read(blob),
            "the file was cut short inside status"
        );
        assert_eq!(
            workspace.overlays,
            [WorkspaceOverlay {
                path: "racing.py".to_owned(),
                kind: OverlayKind::Unstaged,
            }]
        );
        assert_eq!(workspace.clean_source_alias("racing.py"), None);
        assert!(workspace.clean_source_alias("kept.py").is_some());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A tracked file removed between status's `lstat` and its read, as a writer that replaces a
    /// file by removing it first leaves it for a moment, is deleted.
    #[test]
    fn a_tracked_file_removed_while_status_reads_it_is_deleted() {
        let (root, blob) = rewritten_while_status_reads_it("removed", |racing| {
            std::fs::remove_file(racing).unwrap();
        });

        let workspace = discover(&root).unwrap();

        assert!(
            ran_during_its_read(blob),
            "the file was removed inside status"
        );
        assert_eq!(
            workspace.overlays,
            [WorkspaceOverlay {
                path: "racing.py".to_owned(),
                kind: OverlayKind::Deleted,
            }]
        );
        assert!(workspace.clean_source_alias("kept.py").is_some());
        std::fs::remove_dir_all(root).unwrap();
    }
}
