//! The one opener every in-process Git read uses, so the engine reads a repository as the Git
//! Rerail runs does, or not at all.
//!
//! - It refuses where Git refuses: a repository another user owns that `safe.directory` does not
//!   name (`bail_if_untrusted`), and a config file Git cannot parse. A value gix cannot interpret
//!   (`core.autocrlf = 2`) takes gix's default for that key, as gix's lenient mode does.
//! - It names the system and installation config files Rerail's Git reported
//!   ([`super::setup::GitSetup`]), so gix never starts a Git of its own to find them: gix-path runs
//!   the first `git.exe` on `PATH` on Windows for the system file (`git config --show-origin`) and
//!   for the system attributes (`git --exec-path`). It applies `GIT_CONFIG_SYSTEM` over the named
//!   file and `GIT_CONFIG_NOSYSTEM` as Git does; gix applies `HOME`, `XDG_CONFIG_HOME` and
//!   `GIT_CONFIG_GLOBAL` for the global files.
//! - It refuses a config, exclude or attribute file it can name before the read when the file is
//!   larger than [`MAX_GIT_FILE_BYTES`], so a damaged or hostile repository cannot make the daemon
//!   read a gigabyte into memory.

use std::path::{Path, PathBuf};

use super::setup::{GitSetup, installed};

/// The largest config, exclude or attribute file a read takes: Git's own limit for an attributes
/// file since 2.39.1 (`ATTR_MAX_FILE_SIZE`). Real ones are a few kilobytes.
pub const MAX_GIT_FILE_BYTES: u64 = 100 << 20;

#[derive(Debug)]
pub enum OpenError {
    /// No Git repository holds the path.
    NoRepository(String),
    /// A repository holds the path, and Git would refuse it or it cannot be read.
    Failed(String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoRepository(message) | Self::Failed(message) => formatter.write_str(message),
        }
    }
}

/// Whether discovery failed because no repository encloses the path, rather than because one
/// could not be read.
fn is_no_repository(error: &gix::Error) -> bool {
    error
        .downcast_any_ref::<gix::discover::upwards::Error>()
        .is_some_and(no_repository)
}

fn no_repository(error: &gix::discover::upwards::Error) -> bool {
    matches!(
        error,
        gix::discover::upwards::Error::NoGitRepository { .. }
            | gix::discover::upwards::Error::NoGitRepositoryWithinCeiling { .. }
            | gix::discover::upwards::Error::NoGitRepositoryWithinFs { .. }
    )
}

/// Tests only: discovery takes every repository for one another user owns, as Git does under
/// `GIT_TEST_ASSUME_DIFFERENT_OWNER`. Set in a test's own child process, never in a shared one.
#[cfg(any(test, feature = "test-hooks"))]
pub static ASSUME_ANOTHER_OWNER: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn upwards_options() -> gix::discover::upwards::Options<'static> {
    #[allow(unused_mut, reason = "only the test build assumes another owner")]
    let mut options = gix::discover::upwards::Options::default();
    #[cfg(any(test, feature = "test-hooks"))]
    if ASSUME_ANOTHER_OWNER.load(std::sync::atomic::Ordering::Relaxed) {
        options.trust = gix::discover::upwards::TrustPolicy::Assume(gix::sec::Trust::Reduced);
    }
    options
}

/// [`discover`] on a thread of its own under the Git read time limit ([`super::pool`]), for a
/// caller that then reads the repository itself.
pub fn open(root: &Path) -> Result<gix::Repository, OpenError> {
    let owned = root.to_path_buf();
    super::pool::run(
        "opening the Git repository",
        root,
        super::pool::GIT_READ_TIME_LIMIT,
        move || discover(&owned),
    )
    .map_err(|error| OpenError::Failed(error.message))?
}

/// The options [`discover`] opens a repository with, for a caller that opens its Git directory again (a source
/// view whose object store changed).
pub fn open_options() -> gix::open::Options {
    let setup = installed();
    Sources::new(setup.as_deref()).options()
}

/// The Git repository around `root`, opened as Git would open it from there.
pub fn discover(root: &Path) -> Result<gix::Repository, OpenError> {
    let setup = installed();
    let sources = Sources::new(setup.as_deref());
    let (found, trust) = gix::discover::upwards_opts(root, upwards_options()).map_err(|error| {
        let message = format!("cannot discover Git repository: {error}");
        if is_no_repository(&error) {
            OpenError::NoRepository(message)
        } else {
            OpenError::Failed(message)
        }
    })?;
    let (git_dir, _) = found.into_repository_and_work_tree_directories();
    // gix's discovery has read `commondir` already; the opener reads it again, bounded, to name the
    // files it measures, also in a repository another user owns. There, before gix's open checks
    // `safe.directory`, it reads only that file and stats the config files gix reads to open it (the
    // common `config`, `config.worktree`); the exclude and attribute files are stat'ed once it has.
    let common_dir = common_dir(&git_dir)?;
    let [config, exclude, attributes] = repository_files(&common_dir);
    let (before, after) = match trust {
        gix::sec::Trust::Full => (vec![config, exclude, attributes], Vec::new()),
        gix::sec::Trust::Reduced => (vec![config], vec![exclude, attributes]),
    };
    for file in sources
        .config_files()
        .into_iter()
        .chain(before)
        .chain([git_dir.join("config.worktree")])
    {
        check_size(&file)?;
    }
    let repository =
        gix::discover_opts(root, upwards_options(), sources.options()).map_err(|error| {
            let message = format!("cannot open Git repository: {error}");
            if is_no_repository(&error) {
                OpenError::NoRepository(message)
            } else {
                OpenError::Failed(message)
            }
        })?;
    for file in after.into_iter().chain(global_files(&repository)) {
        check_size(&file)?;
    }
    Ok(repository)
}

/// The longest `commondir` file read: it holds one path, and no system takes a path this long.
const MAX_COMMONDIR_BYTES: u64 = 64 << 10;

/// The folder `git_dir` shares with the main worktree, as its `commondir` file names it; `git_dir`
/// itself where it has none. The file is opened once, without waiting on a FIFO, and read only if
/// that handle is a regular file within [`MAX_COMMONDIR_BYTES`]; trailing line ends are dropped, as
/// Git drops them. gix's discovery has read it already by then (whole, as it reads the `.git`
/// file); this read is the opener's own, to name the files it measures.
fn common_dir(git_dir: &Path) -> Result<PathBuf, OpenError> {
    use std::io::Read;
    let file = git_dir.join("commondir");
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(&mut options, libc::O_NONBLOCK);
    let mut handle = match options.open(&file) {
        Ok(handle) => handle,
        Err(error) if absent(&error) => return Ok(git_dir.to_path_buf()),
        Err(error) => return Err(unreadable(&file, &error)),
    };
    let metadata = handle
        .metadata()
        .map_err(|error| unreadable(&file, &error))?;
    check_metadata(&file, &metadata, MAX_COMMONDIR_BYTES)?;
    let mut bytes = Vec::new();
    handle
        .by_ref()
        .take(MAX_COMMONDIR_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| unreadable(&file, &error))?;
    while bytes
        .last()
        .is_some_and(|byte| matches!(byte, b'\n' | b'\r'))
    {
        bytes.pop();
    }
    let common = gix::path::from_byte_slice(&bytes)
        .map_err(|_| OpenError::Failed(format!("{} names no path", file.display())))?;
    Ok(git_dir.join(common))
}

/// The config, exclude and attribute files of the repository whose common folder is `common_dir`.
fn repository_files(common_dir: &Path) -> [PathBuf; 3] {
    [
        common_dir.join("config"),
        common_dir.join("info").join("exclude"),
        common_dir.join("info").join("attributes"),
    ]
}

/// Whether `error` means the file is not there, which Git reads as empty.
fn absent(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
    )
}

fn unreadable(path: &Path, error: &std::io::Error) -> OpenError {
    OpenError::Failed(format!("cannot read {}: {error}", path.display()))
}

/// Refuses `path` before gix reads it when it is larger than [`MAX_GIT_FILE_BYTES`], when it is
/// neither a regular file nor a folder (a FIFO or a device, which a read would wait on or never
/// finish), or when it cannot be looked at. A missing file passes, as Git reads it as empty, and so
/// do a folder, which gix reads as no file, and the null device (`GIT_CONFIG_GLOBAL=/dev/null`,
/// `core.excludesFile=/dev/null`), which reads as empty.
pub fn check_size(path: &Path) -> Result<(), OpenError> {
    if null_device_name(path) {
        return Ok(());
    }
    match std::fs::metadata(path) {
        Err(error) if absent(&error) => Ok(()),
        Err(error) => Err(unreadable(path, &error)),
        Ok(metadata) if metadata.is_dir() || is_null_device(&metadata) => Ok(()),
        Ok(metadata) => check_metadata(path, &metadata, MAX_GIT_FILE_BYTES),
    }
}

/// Whether `path` names Windows' null device (`NUL`, `\\.\NUL`), which has no metadata to compare.
fn null_device_name(path: &Path) -> bool {
    cfg!(windows)
        && path.to_str().is_some_and(|name| {
            let name = name.strip_prefix(r"\\.\").unwrap_or(name);
            name.eq_ignore_ascii_case("nul")
        })
}

/// Whether `metadata` is the null device's, reached by name or through a link.
#[cfg(unix)]
fn is_null_device(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    metadata.file_type().is_char_device()
        && std::fs::metadata("/dev/null").is_ok_and(|null| null.rdev() == metadata.rdev())
}

#[cfg(not(unix))]
fn is_null_device(_metadata: &std::fs::Metadata) -> bool {
    false
}

fn check_metadata(path: &Path, metadata: &std::fs::Metadata, limit: u64) -> Result<(), OpenError> {
    if !metadata.is_file() {
        return Err(OpenError::Failed(format!(
            "{} is not a regular file",
            path.display()
        )));
    }
    if metadata.len() > limit {
        return Err(OpenError::Failed(format!(
            "{} holds {} bytes, more than the {limit} a Git read takes",
            path.display(),
            metadata.len()
        )));
    }
    Ok(())
}

/// The exclude and attribute files outside the repository that its config names: `core.excludesFile`
/// or `git/ignore` in the XDG folder, and `core.attributesFile` or `git/attributes` there.
fn global_files(repository: &gix::Repository) -> Vec<PathBuf> {
    let config = repository.config_snapshot();
    let mut files = Vec::new();
    match config.trusted_path("core.excludesFile") {
        Ok(Some(path)) => files.push(path),
        _ => files.extend(xdg_file("ignore")),
    }
    files.extend(repository.attribute_global_paths().unwrap_or_default());
    files
}

/// `git/<name>` in `XDG_CONFIG_HOME`, else in `.config` in the home folder, as gix finds it.
fn xdg_file(name: &str) -> Option<PathBuf> {
    gix::path::env::xdg_config(name, &mut gix::path::env::var)
}

/// Removes every filter driver from `repository`'s config in memory, so a read that converts a
/// worktree file to Git's form (a status of a file whose stat changed) runs no program the
/// repository or the user configured: a file under a driver (Git LFS) then reads as changed, and
/// the reader takes its bytes from disk as for any changed file.
pub fn without_filter_programs(repository: &mut gix::Repository) -> Result<(), String> {
    let mut config = repository.config_snapshot_mut();
    let drivers = config
        .sections_and_ids_by_name("filter")
        .map(|sections| sections.map(|(_, id)| id).collect::<Vec<_>>())
        .unwrap_or_default();
    for id in drivers {
        config.remove_section_by_id(id);
    }
    config.commit().map(|_| ()).map_err(|error| {
        format!("cannot set the Git config aside from its filter drivers: {error}")
    })
}

/// The system attributes file of Rerail's Git: `Ok(None)` where Git reads none, `Err` where it
/// cannot be named without starting a Git (Windows, with no system file reported).
#[allow(
    clippy::result_unit_err,
    reason = "main's signature: the error says only that the file cannot be named; public because its callers are in other crates"
)]
pub fn system_attributes() -> Result<Option<PathBuf>, ()> {
    if env_flag("GIT_ATTR_NOSYSTEM") {
        return Ok(None);
    }
    match installed().as_deref().and_then(GitSetup::system_attributes) {
        Some(path) => Ok(Some(path)),
        // gix's own answer where finding it starts no Git: `/etc/gitattributes`.
        None if cfg!(not(windows)) => Ok(Some(PathBuf::from("/etc/gitattributes"))),
        None => Err(()),
    }
}

/// Git's reading of a boolean environment variable (`git_env_bool`): unset is false.
fn env_flag(name: &str) -> bool {
    std::env::var_os(name)
        .and_then(|value| gix::config::Boolean::try_from(value).ok())
        .is_some_and(|value| value.0)
}

/// The config files outside the repository the opener hands gix, from the setup and the
/// environment.
struct Sources {
    system: Option<PathBuf>,
    installation: Option<PathBuf>,
    /// Whether gix reads a system file it finds itself: where Rerail's Git named none (it could
    /// not be asked, or answered nothing) and finding one starts no Git, that is everywhere but
    /// Windows, where gix would start the first `git.exe` on `PATH`.
    system_found_by_gix: bool,
}

impl Sources {
    fn new(setup: Option<&GitSetup>) -> Self {
        let system = std::env::var_os("GIT_CONFIG_SYSTEM")
            .map(PathBuf::from)
            .or_else(|| setup.and_then(|setup| setup.system_config.clone()));
        Self {
            system_found_by_gix: system.is_none() && cfg!(not(windows)),
            system,
            installation: setup.and_then(|setup| setup.installation_config.clone()),
        }
    }

    fn options(&self) -> gix::open::Options {
        let mut permissions = gix::open::Permissions::default();
        // Finding the system attributes file starts `git --exec-path` on Windows; the reads that
        // need it add the file Rerail's Git reads (`system_attributes`).
        permissions.attributes.system = false;
        permissions.attributes.git_binary = false;
        permissions.config.system = self.system.is_some() || self.system_found_by_gix;
        permissions.config.git_binary = self.installation.is_some();
        // A hook may export a worktree, index or ref namespace for another repository, which gix applies even after
        // discovery found this one: with any of them set, Git's prefixed variables are not read (gix groups these
        // overrides under one permission).
        if ["GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_NAMESPACE", "GIT_SHALLOW_FILE"]
            .iter()
            .any(|name| std::env::var_os(name).is_some())
        {
            permissions.env.git_prefix = gix::sec::Permission::Deny;
        }
        let mut options = gix::open::Options::default()
            .permissions(permissions)
            .bail_if_untrusted(true);
        if let Some(path) = &self.system {
            options = options.system_config_path(path);
        }
        if let Some(path) = &self.installation {
            options = options.git_installation_config_path(path);
        }
        options
    }

    /// The config files outside any repository gix reads, as far as the opener can name them.
    fn config_files(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        if !env_flag("GIT_CONFIG_NOSYSTEM") {
            files.extend(self.system.clone());
            files.extend(self.installation.clone());
        }
        match std::env::var_os("GIT_CONFIG_GLOBAL") {
            Some(global) => files.push(PathBuf::from(global)),
            None => {
                files.extend(xdg_file("config"));
                files.extend(gix::path::env::home_dir().map(|home| home.join(".gitconfig")));
            }
        }
        files
    }
}

#[cfg(test)]
mod tests {
    use super::{GitSetup, Sources};

    /// Where Rerail's Git named no system file, gix still reads its own (`/etc/gitconfig`) where
    /// finding it starts no Git, so a system `safe.directory` or `core.excludesFile` keeps counting.
    #[test]
    fn an_empty_report_keeps_the_system_file_gix_finds_without_git() {
        let named = std::env::var_os("GIT_CONFIG_SYSTEM").is_some();
        let permissions = Sources::new(Some(&GitSetup::default()))
            .options()
            .permissions;
        assert_eq!(permissions.config.system, named || cfg!(not(windows)));
    }
}
