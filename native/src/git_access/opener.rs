//! The one opener every in-process Git read uses, so the engine reads a repository as the Git
//! Rerail runs does, or not at all.
//!
//! - It refuses where Git refuses: a repository another user owns that `safe.directory` does not
//!   name (`bail_if_untrusted`), and a config file Git cannot parse or a value it rejects
//!   (`strict_config`).
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
pub(crate) const MAX_GIT_FILE_BYTES: u64 = 100 << 20;

#[derive(Debug)]
pub(crate) enum OpenError {
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
#[cfg(test)]
pub(crate) static ASSUME_ANOTHER_OWNER: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn upwards_options() -> gix::discover::upwards::Options<'static> {
    #[allow(unused_mut, reason = "only the test build assumes another owner")]
    let mut options = gix::discover::upwards::Options::default();
    #[cfg(test)]
    if ASSUME_ANOTHER_OWNER.load(std::sync::atomic::Ordering::Relaxed) {
        options.trust =
            gix::discover::upwards::TrustPolicy::Assume(gix::sec::Trust::Reduced);
    }
    options
}

/// The Git repository around `root`, opened as Git would open it from there.
pub(crate) fn discover(root: &Path) -> Result<gix::Repository, OpenError> {
    let setup = installed();
    let sources = Sources::new(setup.as_deref());
    let (found, _) = gix::discover::upwards_opts(root, upwards_options()).map_err(|error| {
        let message = format!("cannot discover Git repository: {error}");
        if error
            .downcast_any_ref::<gix::discover::upwards::Error>()
            .is_some_and(no_repository)
        {
            OpenError::NoRepository(message)
        } else {
            OpenError::Failed(message)
        }
    })?;
    let (git_dir, _) = found.into_repository_and_work_tree_directories();
    let common_dir = match gix::discover::path::from_plain_file(&git_dir.join("commondir")) {
        Some(Ok(common)) => git_dir.join(common),
        _ => git_dir.clone(),
    };
    for file in sources
        .config_files()
        .into_iter()
        .chain([common_dir.join("config"), git_dir.join("config.worktree")])
        .chain([
            common_dir.join("info").join("exclude"),
            common_dir.join("info").join("attributes"),
        ])
    {
        check_size(&file)?;
    }
    let repository = gix::discover_opts(root, upwards_options(), sources.options()).map_err(|error| {
        let message = format!("cannot open Git repository: {error}");
        if is_no_repository(&error) {
            OpenError::NoRepository(message)
        } else {
            OpenError::Failed(message)
        }
    })?;
    for file in global_files(&repository) {
        check_size(&file)?;
    }
    Ok(repository)
}

/// Refuses `path` when it is larger than [`MAX_GIT_FILE_BYTES`]; a missing file passes, as Git
/// reads it as empty.
pub(crate) fn check_size(path: &Path) -> Result<(), OpenError> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.len() > MAX_GIT_FILE_BYTES => Err(OpenError::Failed(format!(
            "{} holds {} bytes, more than the {MAX_GIT_FILE_BYTES} a Git read takes",
            path.display(),
            metadata.len()
        ))),
        _ => Ok(()),
    }
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
pub(crate) fn without_filter_programs(repository: &mut gix::Repository) -> Result<(), String> {
    let mut config = repository.config_snapshot_mut();
    let drivers = config
        .sections_and_ids_by_name("filter")
        .map(|sections| sections.map(|(_, id)| id).collect::<Vec<_>>())
        .unwrap_or_default();
    for id in drivers {
        config.remove_section_by_id(id);
    }
    config
        .commit()
        .map(|_| ())
        .map_err(|error| format!("cannot set the Git config aside from its filter drivers: {error}"))
}

/// The system attributes file of Rerail's Git: `Ok(None)` where Git reads none, `Err` where it
/// cannot be named without starting a Git (Windows, with no system file reported).
pub(crate) fn system_attributes() -> Result<Option<PathBuf>, ()> {
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
    /// Whether gix reads a system file it finds itself: only where no setup was installed and
    /// finding it starts no Git.
    system_found_by_gix: bool,
}

impl Sources {
    fn new(setup: Option<&GitSetup>) -> Self {
        let system = std::env::var_os("GIT_CONFIG_SYSTEM")
            .map(PathBuf::from)
            .or_else(|| setup.and_then(|setup| setup.system_config.clone()));
        Self {
            system_found_by_gix: system.is_none() && setup.is_none() && cfg!(not(windows)),
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
        let mut options = gix::open::Options::default()
            .permissions(permissions)
            .bail_if_untrusted(true)
            .strict_config(true);
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
