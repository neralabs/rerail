//! What Rerail's own Git reported about the configuration it reads outside any repository. The
//! host asks once per process (`src/engine/native/git-access.ts`) and installs the answer through
//! [`super::owner::GitAccess`], so every opener in the process names the files that Git reads.

use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};

/// The config files Rerail's Git reads outside any repository, as it reported them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct GitSetup {
    /// `git version`'s output, empty where Git could not be asked.
    pub(crate) version: String,
    /// The system config file this Git was built to read, where no environment variable moves it.
    pub(crate) system_config: Option<PathBuf>,
    /// A config file this Git reads besides the system one, such as Apple Git's under `/Library`,
    /// which Git lists with the scope `unknown`.
    pub(crate) installation_config: Option<PathBuf>,
}

impl GitSetup {
    /// The setup from the host's answers: `git version`; `git var GIT_CONFIG_SYSTEM`, absent on a
    /// Git before 2.42 or where it failed; and `git config --list --show-origin --show-scope -z
    /// --name-only --no-includes` outside any repository, absent where it failed. The host asks both
    /// with `GIT_CONFIG_SYSTEM` and `GIT_CONFIG_NOSYSTEM` removed, which the opener applies itself,
    /// so they name the file Git was built to read.
    pub(crate) fn from_report(
        version: &str,
        system_config: Option<&str>,
        config_origins: Option<&str>,
    ) -> Self {
        let origins = config_origins.map(file_origins).unwrap_or_default();
        let system_config = system_config
            .map(|path| path.trim_end_matches(['\r', '\n']))
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            // A Git before 2.42 names its system file only as the origin of an entry the file holds;
            // a system file with no entry changes nothing Git reads.
            .or_else(|| {
                origins
                    .iter()
                    .find(|(scope, _)| *scope == "system")
                    .map(|(_, path)| PathBuf::from(path))
            });
        let installation_config = origins
            .iter()
            .find(|(scope, path)| {
                *scope == "unknown" && system_config.as_deref() != Some(Path::new(path))
            })
            .map(|(_, path)| PathBuf::from(path));
        Self {
            version: version.trim().to_owned(),
            system_config,
            installation_config,
        }
    }

    /// The system attributes file: `gitattributes` beside the system config file, since Git takes
    /// both from one configured folder (`$(sysconfdir)`); `None` where the system file is unknown.
    pub(crate) fn system_attributes(&self) -> Option<PathBuf> {
        Some(self.system_config.as_deref()?.parent()?.join("gitattributes"))
    }
}

/// `(scope, path)` of each file-backed record of a `-z` listing with scope and origin, in Git's
/// order: records are `scope NUL origin NUL name NUL`, and an origin from a file reads `file:<path>`.
fn file_origins(listing: &str) -> Vec<(&str, &str)> {
    let mut fields = listing.split('\0');
    let mut origins = Vec::new();
    while let (Some(scope), Some(origin), Some(_name)) = (fields.next(), fields.next(), fields.next())
    {
        if let Some(path) = origin.strip_prefix("file:") {
            origins.push((scope, path));
        }
    }
    origins
}

static INSTALLED: RwLock<Option<Arc<GitSetup>>> = RwLock::new(None);

/// Makes `setup` the one every later open in this process uses.
pub(crate) fn install(setup: Arc<GitSetup>) {
    *INSTALLED.write().unwrap_or_else(PoisonError::into_inner) = Some(setup);
}

/// The setup the host installed in this process; `None` before it installed one.
pub(crate) fn installed() -> Option<Arc<GitSetup>> {
    INSTALLED
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}
