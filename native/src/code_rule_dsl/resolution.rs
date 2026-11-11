//! What an evaluation's resolutions read besides a file's own tree (ECMAScript's import targets: the files a
//! workspace or snapshot holds, over disk below the workspace's root). The language whose resolutions they are
//! registers them at load (`install`); W holds the files, makes one view per apply and installs it around each
//! evaluation, without reading either.

use crate::code_rule_dsl::model::Snapshot;
use std::any::Any;
use std::path::Path;
use std::sync::{Arc, OnceLock};

/// The files a workspace holds for resolution.
pub(crate) trait HeldForResolution: Send + Sync {
    /// Holds the file at `path` with `source`; whether that changes what a resolution reads (disk below `root`
    /// standing for a path not held before).
    fn hold(&mut self, path: &str, source: &Arc<String>, root: Option<&Path>) -> bool;
    /// Stops holding `path`; whether that changes what a resolution reads.
    fn release(&mut self, path: &str, root: Option<&Path>) -> bool;
    /// What one apply's resolutions read: the held files now, over disk below `root` as the apply finds it.
    fn view(&self, root: Option<&Path>) -> Arc<dyn ResolutionView>;
}

/// One apply's (or one snapshot's) view of the files resolutions read.
pub(crate) trait ResolutionView: Send + Sync {
    /// Installs the view for the evaluations on this thread while the guard is held.
    fn install(self: Arc<Self>) -> Box<dyn Any>;
}

/// A language's resolutions (`install`).
pub(crate) struct Resolutions {
    /// A workspace's held files, empty.
    pub(crate) held: fn() -> Box<dyn HeldForResolution>,
    /// What a snapshot's resolutions read: its files, with no disk below them.
    pub(crate) snapshot: fn(&Snapshot) -> Arc<dyn ResolutionView>,
    /// Whether a view is installed on this thread (a workspace's, around a snapshot's evaluation inside it).
    pub(crate) installed: fn() -> bool,
}

static RESOLUTIONS: OnceLock<Resolutions> = OnceLock::new();

/// Registers the resolutions (once, at load).
pub(crate) fn install(resolutions: Resolutions) {
    let _ = RESOLUTIONS.set(resolutions);
}

/// No resolutions: nothing held, nothing installed.
struct Nothing;

impl HeldForResolution for Nothing {
    fn hold(&mut self, _: &str, _: &Arc<String>, _: Option<&Path>) -> bool {
        false
    }
    fn release(&mut self, _: &str, _: Option<&Path>) -> bool {
        false
    }
    fn view(&self, _: Option<&Path>) -> Arc<dyn ResolutionView> {
        Arc::new(Nothing)
    }
}

impl ResolutionView for Nothing {
    fn install(self: Arc<Self>) -> Box<dyn Any> {
        Box::new(())
    }
}

impl Default for Box<dyn HeldForResolution> {
    fn default() -> Self {
        match RESOLUTIONS.get() {
            Some(resolutions) => (resolutions.held)(),
            None => Box::new(Nothing),
        }
    }
}

/// Installs `snapshot`'s view unless one is installed already (a workspace's evaluation installed its own).
pub(crate) fn install_snapshot_unless_present(snapshot: &Snapshot) -> Box<dyn Any> {
    match RESOLUTIONS.get() {
        Some(resolutions) if !(resolutions.installed)() => (resolutions.snapshot)(snapshot).install(),
        _ => Box::new(()),
    }
}
