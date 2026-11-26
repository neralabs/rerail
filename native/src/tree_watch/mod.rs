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
    /// Notices were dropped (an OS overflow, more than 16,384 entries between two takes, or a
    /// Windows handle let go for a rename of the root or above it, at every take until it is open
    /// again): read the whole root.
    pub lost: bool,
    /// The watch stopped (the root or a folder above it renamed, moved or deleted, a watch error).
    /// The root is no longer watched: read it whole and watch it again.
    pub ended: bool,
}

/// One OS's watch of roots. Each adapter reports into the [`Notices`] it was opened with, and only
/// for roots `Notices` holds as watched; the [`TreeWatcher`] starts and stops roots there.
pub(crate) trait Backend {
    /// Watches `root` (`folders`: the folders under it to watch, relative `/`-paths with `""` for
    /// the root, used by adapters that watch folder by folder). False when the root cannot be
    /// watched here; the adapter then holds nothing for it.
    fn watch(&mut self, root: &str, folders: &[String]) -> bool;
    /// Watches `root` as [`Backend::watch`] does, but only with room a watch budget has left
    /// (Linux), giving that room up, its watch ended, to any root watched with `watch` that
    /// needs it; it stays spare until it is let go of or watched with `watch`. Adapters with no
    /// budget watch it as any root.
    fn watch_spare(&mut self, root: &str, folders: &[String]) -> bool {
        self.watch(root, folders)
    }
    /// Releases every handle on `root`; a root not watched is no-op.
    fn unwatch(&mut self, root: &str);
    /// Releases every root and stops the adapter's thread, waiting for it.
    fn close(&mut self);
    /// How many roots the adapter holds a handle on.
    #[cfg(test)]
    fn roots(&self) -> usize;
    /// Reports what the OS queued for the adapter and its thread has not read yet, so a take holds
    /// every change the OS reported before it. A no-op where the OS hands nothing over that way:
    /// on Windows the thread is woken by the kernel, and the move of a watched root waits for it.
    fn flush(&mut self) {}
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
        Ok(self.watch_as(&root, &folders, false))
    }

    /// Watches `root` as `watch` does, but only with room the watch budget has left (Linux): a
    /// root watched with `watch` that needs that room takes it, and this root's watch ends
    /// ([`Backend::watch_spare`]). A root watched so stays spare until it is let go of or watched
    /// with `watch`.
    #[napi(catch_unwind)]
    pub fn watch_spare(&mut self, root: String, folders: Vec<String>) -> Result<bool> {
        Ok(self.watch_as(&root, &folders, true))
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

    /// Takes every notice since the last take, at most one per root, with what the OS held for the
    /// adapter when the take began that its thread had not read yet. It never waits for a change to
    /// come: on Linux it reads at most the events the kernel held at its start, and waits for the
    /// adapter's thread only while that thread holds its buffer, for one read of 64 KiB at most and
    /// the reports of what it read.
    /// A root whose watch ended is let go of here: the adapter that ended it may still hold it
    /// (an FSEvents stream cannot stop itself from its callback).
    #[napi(catch_unwind)]
    pub fn take(&mut self) -> Result<Vec<TreeNotice>> {
        if let Some(backend) = self.backend.as_mut() {
            backend.flush();
        }
        let taken = self.notices.take();
        if let Some(backend) = self.backend.as_mut() {
            for notice in taken.iter().filter(|notice| notice.ended) {
                backend.unwatch(&notice.root);
            }
        }
        Ok(taken)
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
    /// `watch`, or with `spare` `watch_spare`.
    fn watch_as(&mut self, root: &str, folders: &[String], spare: bool) -> bool {
        let notices = Arc::clone(&self.notices);
        let Some(backend) = self.backend() else {
            return false;
        };
        // A root not watched until now (or whose watch ended) starts from nothing the adapter
        // still holds for it.
        if notices.start(root) {
            backend.unwatch(root);
        }
        let watched = if spare {
            backend.watch_spare(root, folders)
        } else {
            backend.watch(root, folders)
        };
        if !watched {
            notices.stop(root);
        }
        watched
    }

    /// A watcher over `backend`, which reports into `notices`.
    #[cfg(all(test, target_os = "linux"))]
    fn over(notices: Arc<Notices>, backend: Box<dyn Backend>) -> Self {
        Self {
            notices,
            backend: Some(backend),
            unavailable: false,
            closed: false,
        }
    }

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

    /// How many roots the adapter holds a handle on.
    #[cfg(test)]
    fn held_roots(&self) -> usize {
        self.backend.as_ref().map_or(0, |backend| backend.roots())
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
