//! What the adapters report, kept per watched root until the host takes it.

use super::TreeNotice;
use indexmap::{IndexMap, IndexSet};
use std::collections::HashSet;
use std::sync::{Mutex, MutexGuard, PoisonError};

/// The most path entries (`changed` and `renamed` together) kept for one root between two takes;
/// one more drops them and marks the root `lost`, so the host reads it whole.
pub(crate) const NOTICE_CAP: usize = 16_384;

#[derive(Default)]
struct Pending {
    changed: IndexSet<String>,
    renamed: IndexSet<String>,
    lost: bool,
    ended: bool,
}

impl Pending {
    /// Records `path` in `changed` or `renamed`, or marks the root lost past the cap.
    fn add(&mut self, path: String, renamed: bool) {
        if self.lost {
            return;
        }
        if self.full_for(&path) {
            self.lose();
            return;
        }
        let paths = if renamed {
            &mut self.renamed
        } else {
            &mut self.changed
        };
        paths.insert(path);
    }

    /// Whether `path` would be one entry past the cap.
    fn full_for(&self, path: &str) -> bool {
        let known = self.changed.contains(path) || self.renamed.contains(path);
        !known && self.changed.len() + self.renamed.len() >= NOTICE_CAP
    }

    fn lose(&mut self) {
        self.lost = true;
        self.changed = IndexSet::new();
        self.renamed = IndexSet::new();
    }
}

#[derive(Default)]
struct State {
    watched: HashSet<String>,
    pending: IndexMap<String, Pending>,
    /// The watched roots no handle reports on for now (see [`Notices::unheard`]).
    unheard: HashSet<String>,
}

impl State {
    /// The pending notice of `root` while it is watched; none for a root not watched, whose
    /// reports are dropped.
    fn watched(&mut self, root: &str) -> Option<&mut Pending> {
        if !self.watched.contains(root) {
            return None;
        }
        Some(self.pending.entry(root.to_owned()).or_default())
    }
}

/// The watched roots and their notices since the last take. Adapters' threads report here; the
/// host's thread starts, stops and takes.
#[derive(Default)]
pub(crate) struct Notices {
    state: Mutex<State>,
}

impl Notices {
    fn state(&self) -> MutexGuard<'_, State> {
        // A panic while the lock was held left at worst a half-recorded notice; a notice is a hint
        // the host checks against the files, so keep going.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Holds `root` as watched; true when it was not (never watched, or stopped or ended since).
    /// A root whose watch ended before its notice was taken, and is watched again, keeps that
    /// notice as `lost`: what happened in between went unseen.
    pub(crate) fn start(&self, root: &str) -> bool {
        let mut state = self.state();
        if !state.watched.insert(root.to_owned()) {
            return false;
        }
        if let Some(pending) = state.pending.get_mut(root).filter(|pending| pending.ended) {
            pending.ended = false;
            pending.lose();
        }
        true
    }

    /// Forgets `root` and every notice of it not taken.
    pub(crate) fn stop(&self, root: &str) {
        let mut state = self.state();
        state.watched.remove(root);
        state.unheard.remove(root);
        state.pending.shift_remove(root);
    }

    /// No handle reports on `root` for now (Windows: its handle was let go for a rename or removal
    /// of the root or a folder above it, which waits for that). It is `lost` now, before what let
    /// it go ends, and at every take until [`Notices::heard`]: a host never takes a root as unmoved
    /// while nothing watches it.
    #[cfg(any(windows, test))]
    pub(crate) fn unheard(&self, root: &str) {
        let mut state = self.state();
        let Some(pending) = state.watched(root) else {
            return;
        };
        pending.lose();
        state.unheard.insert(root.to_owned());
    }

    /// Whether no handle reports on `root` for now (see [`Notices::unheard`]).
    #[cfg(test)]
    pub(crate) fn is_unheard(&self, root: &str) -> bool {
        self.state().unheard.contains(root)
    }

    /// A handle reports on `root` again (see [`Notices::unheard`]).
    #[cfg(any(windows, test))]
    pub(crate) fn heard(&self, root: &str) {
        self.state().unheard.remove(root);
    }

    /// Whether `root` is watched: started, and neither stopped nor ended since.
    #[cfg(test)]
    pub(crate) fn watching(&self, root: &str) -> bool {
        self.state().watched.contains(root)
    }

    /// Whether `root`'s notice not taken yet is `lost`.
    #[cfg(test)]
    pub(crate) fn is_lost(&self, root: &str) -> bool {
        self.state()
            .pending
            .get(root)
            .is_some_and(|pending| pending.lost)
    }

    /// `path` (relative to `root`, `/`-separated) changed: its content, size or attributes.
    pub(crate) fn changed(&self, root: &str, path: String) {
        if let Some(pending) = self.state().watched(root) {
            pending.add(path, false);
        }
    }

    /// `path` (relative to `root`, `/`-separated) was created, deleted or renamed.
    pub(crate) fn renamed(&self, root: &str, path: String) {
        if let Some(pending) = self.state().watched(root) {
            pending.add(path, true);
        }
    }

    /// Notices of `root` were dropped.
    pub(crate) fn lost(&self, root: &str) {
        if let Some(pending) = self.state().watched(root) {
            pending.lose();
        }
    }

    /// Notices of every watched root were dropped.
    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn lost_all(&self) {
        let mut state = self.state();
        let roots: Vec<String> = state.watched.iter().cloned().collect();
        for root in roots {
            if let Some(pending) = state.watched(&root) {
                pending.lose();
            }
        }
    }

    /// The watch of `root` stopped; it is no longer watched.
    pub(crate) fn ended(&self, root: &str) {
        let mut state = self.state();
        let Some(pending) = state.watched(root) else {
            return;
        };
        pending.ended = true;
        state.watched.remove(root);
        state.unheard.remove(root);
    }

    /// Every watched root's watch stopped (the adapter's thread failed).
    #[cfg(any(target_os = "linux", windows))]
    pub(crate) fn ended_all(&self) {
        let roots: Vec<String> = self.state().watched.iter().cloned().collect();
        for root in roots {
            self.ended(&root);
        }
    }

    /// Takes every notice since the last take, at most one per root. A root no handle reports on
    /// is `lost` again at the next take.
    pub(crate) fn take(&self) -> Vec<TreeNotice> {
        let mut state = self.state();
        let pending = std::mem::take(&mut state.pending);
        let unheard: Vec<String> = state.unheard.iter().cloned().collect();
        for root in unheard {
            if let Some(next) = state.watched(&root) {
                next.lose();
            }
        }
        drop(state);
        pending
            .into_iter()
            .map(|(root, pending)| TreeNotice {
                root,
                changed: pending.changed.into_iter().collect(),
                renamed: pending.renamed.into_iter().collect(),
                lost: pending.lost,
                ended: pending.ended,
            })
            .collect()
    }

    /// Forgets every root and notice.
    pub(crate) fn clear(&self) {
        *self.state() = State::default();
    }
}
