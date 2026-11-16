//! Watches checkouts' trees and keeps what the OS reports about them until the host takes it.
//!
//! The daemon reads every other worktree of a repository on a walk; a worktree no notice named since
//! its last read need not be read again. [`TreeWatcher`] is the one file-system event bus behind
//! that: the host watches a root, the adapter of the OS (`fsevents.rs` on macOS, `inotify.rs` on
//! Linux, `windows.rs` on Windows) accumulates what changed under it, and the host takes it on its
//! next walk. Nothing calls into JavaScript: notices wait in [`notices::Notices`] until `take`.
//!
//! A root an adapter cannot watch (a network or remote file system, a WSL path, past Linux's watch
//! budget, Windows with no oplock, an OS with no adapter) makes `watch` return false, and the host
//! reads it on every walk, as it would with no watcher at all.

mod notices;

#[cfg(target_os = "macos")]
mod fsevents;
#[cfg(target_os = "linux")]
mod inotify;
#[cfg(windows)]
mod windows;

#[cfg(test)]
mod tests;

use napi::Result;
use napi_derive::napi;
use notices::Notices;
use std::sync::Arc;

/// What changed under one watched root since the last `take`.
#[napi(object)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNotice {
    pub root: String,
    /// Changed entries (content, size, attributes), relative `/`-paths. Capped (see `lost`).
    pub changed: Vec<String>,
    /// Created, deleted or renamed entries, relative `/`-paths. Capped (see `lost`).
    pub renamed: Vec<String>,
    /// Notices were dropped (an OS overflow, or more than 16,384 entries between two takes):
    /// read the whole root.
    pub lost: bool,
    /// The watch stopped (the root or a folder above it renamed, moved or deleted, a Windows oplock
    /// broke, a watch error). The root is no longer watched: read it whole and watch it again.
    pub ended: bool,
}

/// One OS's watch of roots. Each adapter reports into the [`Notices`] it was opened with, and only
/// for roots `Notices` holds as watched; the [`TreeWatcher`] starts and stops roots there.
pub(crate) trait Backend {
    /// Watches `root` (`folders`: the folders under it to watch, relative `/`-paths with `""` for
    /// the root, used by adapters that watch folder by folder). False when the root cannot be
    /// watched here; the adapter then holds nothing for it.
    fn watch(&mut self, root: &str, folders: &[String]) -> bool;
    /// Releases every handle on `root`; a root not watched is no-op.
    fn unwatch(&mut self, root: &str);
    /// Releases every root and stops the adapter's thread, waiting for it.
    fn close(&mut self);
}

/// The adapter of this OS, or none where there is none or it cannot start.
fn open_backend(notices: &Arc<Notices>) -> Option<Box<dyn Backend>> {
    #[cfg(target_os = "macos")]
    return fsevents::FsEvents::open(Arc::clone(notices)).map(|backend| Box::new(backend) as _);
    #[cfg(target_os = "linux")]
    return inotify::Inotify::open(Arc::clone(notices)).map(|backend| Box::new(backend) as _);
    #[cfg(windows)]
    return windows::DirectoryChanges::open(Arc::clone(notices))
        .map(|backend| Box::new(backend) as _);
    #[allow(unreachable_code)]
    {
        let _ = notices;
        None
    }
}

/// The engine's file-system event bus: watched roots, and what changed under each since the last
/// `take`. One per daemon; the OS thread starts at the first `watch`, so constructing one costs
/// nothing.
#[napi]
pub struct TreeWatcher {
    notices: Arc<Notices>,
    backend: Option<Box<dyn Backend>>,
    /// No adapter opened (none on this OS, or it failed): every `watch` is false.
    unavailable: bool,
    closed: bool,
}

impl Default for TreeWatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[napi]
impl TreeWatcher {
    #[napi(constructor)]
    pub fn new() -> Self {
        Self {
            notices: Arc::new(Notices::default()),
            backend: None,
            unavailable: false,
            closed: false,
        }
    }

    /// Starts watching `root` (an absolute physical path); `folders` are the folders the last read
    /// walked into (relative `/`-paths, `""` for the root), which only the Linux adapter uses.
    /// False when this root cannot be watched here: the host then reads it on every walk.
    /// Watching a root already watched replaces its folder set.
    #[napi(catch_unwind)]
    pub fn watch(&mut self, root: String, folders: Vec<String>) -> Result<bool> {
        let notices = Arc::clone(&self.notices);
        let Some(backend) = self.backend() else {
            return Ok(false);
        };
        // A root not watched until now (or whose watch ended) starts from nothing the adapter
        // still holds for it.
        if notices.start(&root) {
            backend.unwatch(&root);
        }
        let watched = backend.watch(&root, &folders);
        if !watched {
            notices.stop(&root);
        }
        Ok(watched)
    }

    /// Stops watching `root` and releases every handle on it.
    #[napi(catch_unwind)]
    pub fn unwatch(&mut self, root: String) -> Result<()> {
        if let Some(backend) = self.backend.as_mut() {
            backend.unwatch(&root);
        }
        self.notices.stop(&root);
        Ok(())
    }

    /// Takes every notice since the last take, at most one per root. Never blocks on the OS.
    #[napi(catch_unwind)]
    pub fn take(&mut self) -> Result<Vec<TreeNotice>> {
        Ok(self.notices.take())
    }

    /// Stops the adapter's thread and releases every handle. Later calls do nothing; a later
    /// `watch` is false.
    #[napi(catch_unwind)]
    pub fn close(&mut self) -> Result<()> {
        self.shut();
        Ok(())
    }
}

impl TreeWatcher {
    /// The open adapter, opening it on first use; none once closed or where none opens.
    fn backend(&mut self) -> Option<&mut Box<dyn Backend>> {
        if self.closed || self.unavailable {
            return None;
        }
        if self.backend.is_none() {
            self.backend = open_backend(&self.notices);
            self.unavailable = self.backend.is_none();
        }
        self.backend.as_mut()
    }

    fn shut(&mut self) {
        self.closed = true;
        if let Some(mut backend) = self.backend.take() {
            backend.close();
        }
        self.notices.clear();
    }
}

impl Drop for TreeWatcher {
    fn drop(&mut self) {
        self.shut();
    }
}

/// `relative` joined under `folder` with `/` (`folder` is `""` at the root).
#[cfg(target_os = "linux")]
fn join_relative(folder: &str, relative: &str) -> String {
    if folder.is_empty() {
        relative.to_owned()
    } else {
        format!("{folder}/{relative}")
    }
}
