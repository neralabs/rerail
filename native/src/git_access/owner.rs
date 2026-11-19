//! The one Node-API object per process that owns the engine's in-process Git reads. The host
//! makes it once, from what Rerail's Git reported, before the first read; it installs that report
//! for every opener in the process and hands out the readers that use it. It keeps no answers yet.

use std::path::Path;
use std::sync::Arc;

use napi::{Error, Result, Status};
use napi_derive::napi;

use super::setup::{GitSetup, install};
use crate::git_ignore::GitIgnoreMatcher;

/// What Rerail's Git reported, as the host asked it: `git var GIT_CONFIG_SYSTEM`, absent on a Git
/// before 2.42 or where it failed; and `git config --list --show-origin --show-scope -z
/// --name-only --no-includes` outside any repository, absent where it failed. Both asks run with
/// `GIT_CONFIG_SYSTEM` and `GIT_CONFIG_NOSYSTEM` removed and `GIT_CONFIG_GLOBAL` on the null
/// device.
#[napi(object)]
pub struct GitInstallation {
    pub system_config: Option<String>,
    pub config_origins: Option<String>,
}

/// The process's owner of in-process Git reads; see the module documentation.
#[napi]
pub struct GitAccess {
    setup: Arc<GitSetup>,
}

#[napi]
impl GitAccess {
    /// Installs `installation` for every in-process Git read in this process, in place of any
    /// earlier one.
    #[napi(constructor)]
    pub fn new(installation: GitInstallation) -> Self {
        let setup = Arc::new(GitSetup::from_report(
            installation.system_config.as_deref(),
            installation.config_origins.as_deref(),
        ));
        install(Arc::clone(&setup));
        Self { setup }
    }

    /// The system config file the reads take, as Rerail's Git named it; null where it named none.
    #[napi(getter)]
    pub fn system_config(&self) -> Option<String> {
        path_text(self.setup.system_config.as_deref())
    }

    /// The installation config file the reads take besides the system one; null where there is none.
    #[napi(getter)]
    pub fn installation_config(&self) -> Option<String> {
        path_text(self.setup.installation_config.as_deref())
    }

    /// Git's exclude stack for paths below `root`, re-reading the index as it changes; null when
    /// `root` is in no Git worktree. Throws where Git would refuse the repository; the error's
    /// `code` is `GitReadTimedOut` where the open, or an earlier one of `root`, outran the time
    /// limit, so a later open may answer.
    #[napi]
    pub fn open_git_ignore_matcher(
        &self,
        root: String,
    ) -> Result<Option<GitIgnoreMatcher>, GitReadStatus> {
        crate::panic_boundary::contain_panic(std::any::type_name::<Self>(), || {
            Ok(GitIgnoreMatcher::open(root))
        })
        .map_err(|error| Error::new(GitReadStatus::Failed, error.reason))?
        .map_err(|error| {
            let status = if error.timed_out {
                GitReadStatus::TimedOut
            } else {
                GitReadStatus::Failed
            };
            Error::new(status, error.message)
        })
    }
}

/// The `code` of an error from a Git read: `GitReadTimedOut` for an open past its time limit.
pub enum GitReadStatus {
    Failed,
    TimedOut,
}

impl AsRef<str> for GitReadStatus {
    fn as_ref(&self) -> &str {
        match self {
            Self::Failed => Status::GenericFailure.as_ref(),
            Self::TimedOut => "GitReadTimedOut",
        }
    }
}

fn path_text(path: Option<&Path>) -> Option<String> {
    path.map(|path| path.to_string_lossy().into_owned())
}
