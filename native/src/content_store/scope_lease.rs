//! A run's hold on its analysis scope (decision 0494): a file beside the store, named by the run's acquisition and
//! locked while the run lives. It is what another process sees of a run that has not yet written anything: a
//! deletion or a stop of a scope a running analysis holds fails (`StoreError::InUse`), to be retried once the run
//! has finished, and the acquisition's number is taken here, not by a write to the store.
//!
//! ```text
//! store.sqlite.scopes/.gate      held by a run's start and by a deletion, one at a time: a start is wholly before
//!                                a deletion (which then sees its lease) or wholly after it (which then reads the
//!                                generation the deletion left), and two starts each see the other's lease
//! store.sqlite.scopes/17         run 17's lease: its scope, its control generation, the generation its first
//!                                write creates (none there as it started); locked by the run, removed as it ends
//! ```
//!
//! A lease is made under a temporary name, locked, then linked to its number, so a lease is never seen unlocked
//! under its number while its run lives. One whose lock is free is a run that ended without removing it (a crash):
//! whoever finds it removes it.

#[cfg(not(target_family = "wasm"))]
use super::StoreError;
use super::StoreResult;
use std::path::Path;
#[cfg(not(target_family = "wasm"))]
use std::path::PathBuf;

/// What a lease records of its run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Held {
    pub acquisition: i64,
    pub scope: String,
    pub acquired_at: i64,
    /// Whether the run's first write creates the scope's generation (none was there when it started).
    pub create: bool,
    /// The generation it creates: its acquisition, or that of an earlier live lease that creates it too.
    pub generation: i64,
}

impl Held {
    fn text(&self) -> String {
        format!("{}\n{}\n{}\n{}\n", self.scope, self.acquired_at, u8::from(self.create), self.generation)
    }

    fn parse(acquisition: i64, text: &str) -> Option<Self> {
        let mut lines = text.lines();
        let scope = lines.next()?.to_owned();
        let acquired_at = lines.next()?.parse().ok()?;
        let create = lines.next()? == "1";
        let generation = lines.next()?.parse().ok()?;
        Some(Self { acquisition, scope, acquired_at, create, generation })
    }
}

/// A run's lease: removed, then unlocked, when it is dropped.
pub struct ScopeLease {
    #[cfg(not(target_family = "wasm"))]
    file: Option<std::fs::File>,
    #[cfg(not(target_family = "wasm"))]
    path: PathBuf,
    pub(super) acquisition: i64,
}

impl Drop for ScopeLease {
    fn drop(&mut self) {
        #[cfg(not(target_family = "wasm"))]
        {
            let _ = std::fs::remove_file(&self.path);
            if let Some(file) = self.file.take() {
                let _ = file.unlock();
            }
        }
    }
}

/// The leases' directory of the store at `store`.
#[cfg(not(target_family = "wasm"))]
fn directory(store: &Path) -> StoreResult<PathBuf> {
    // Resolve aliases, as the collection lock does, so every handle and process of a store meets the same leases.
    let mut path = dunce::canonicalize(store)?.into_os_string();
    path.push(".scopes");
    let path = PathBuf::from(path);
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

#[cfg(not(target_family = "wasm"))]
fn gate(dir: &Path) -> StoreResult<std::fs::File> {
    Ok(std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(dir.join(".gate"))?)
}

/// The leases in `dir`: each live one (its run holds it), and its number. A lease whose lock is free is removed.
#[cfg(not(target_family = "wasm"))]
fn leases(dir: &Path) -> StoreResult<Vec<Held>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)?.flatten() {
        let Some(n) = entry.file_name().to_str().and_then(|s| s.parse::<i64>().ok()) else { continue };
        let Ok(file) = std::fs::File::open(entry.path()) else { continue };
        match file.try_lock() {
            Ok(()) => {
                // Its run ended without removing it.
                let _ = std::fs::remove_file(entry.path());
                let _ = file.unlock();
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
                // A lease is written before it is linked under its number: a lease with no record is no run's.
                if let Some(held) = Held::parse(n, &text) {
                    out.push(held);
                }
            }
            Err(std::fs::TryLockError::Error(e)) => return Err(e.into()),
        }
    }
    out.sort_by_key(|h| h.acquisition);
    Ok(out)
}

/// The highest acquisition a live lease of `store` holds (0 for none): what an acquisition the store numbers
/// itself (`analysis_scope::next_acquisition`) is above.
pub(super) fn highest(store: &Path) -> i64 {
    #[cfg(not(target_family = "wasm"))]
    {
        directory(store).and_then(|dir| leases(&dir)).map_or(0, |l| l.last().map_or(0, |h| h.acquisition))
    }
    #[cfg(target_family = "wasm")]
    {
        let _ = store;
        0
    }
}

/// Whether a live lease of `store` holds `acquisition` (`None`: any).
pub(crate) fn live(store: &Path, acquisition: Option<i64>) -> bool {
    #[cfg(not(target_family = "wasm"))]
    {
        directory(store).and_then(|dir| leases(&dir)).is_ok_and(|l| l.iter().any(|h| acquisition.is_none_or(|a| h.acquisition == a)))
    }
    #[cfg(target_family = "wasm")]
    {
        let _ = (store, acquisition);
        false
    }
}

/// Takes a lease on `scope` for a run of `store`: `counter` reads the store's last acquisition, `plan` whether the
/// scope has no generation (the run's first write creates one); both are reads, made while the gate holds deletions
/// and other starts off. The acquisition is above the counter and above every live lease. Returns the lease and the
/// generation the run's first write creates (`None`: the scope has one).
pub(super) fn take(
    store: &Path,
    scope: &str,
    acquired_at: i64,
    counter: impl FnOnce() -> StoreResult<i64>,
    plan: impl FnOnce() -> StoreResult<bool>,
) -> StoreResult<(ScopeLease, Option<i64>)> {
    #[cfg(not(target_family = "wasm"))]
    {
        let dir = directory(store)?;
        let gate = gate(&dir)?;
        gate.lock()?;
        let result = (|| {
            let live = leases(&dir)?;
            let mut n = counter()?.max(live.last().map_or(0, |h| h.acquisition)) + 1;
            let create = plan()?;
            // A generation the store has not yet seen: a number of the acquisitions' sequence, above every earlier
            // generation (a deleted scope's raises the counter). Two runs that create one create the same.
            let shared = live.iter().filter(|h| h.create && h.scope == scope).map(|h| h.generation).min();
            let temp = dir.join(format!(".lease-{}-{n}", std::process::id()));
            loop {
                let generation = shared.unwrap_or(n);
                let held = Held { acquisition: n, scope: scope.to_owned(), acquired_at, create, generation };
                std::fs::write(&temp, held.text())?;
                let file = std::fs::File::open(&temp)?;
                file.lock()?;
                let path = dir.join(n.to_string());
                match std::fs::hard_link(&temp, &path) {
                    Ok(()) => {
                        let _ = std::fs::remove_file(&temp);
                        return Ok((ScopeLease { file: Some(file), path, acquisition: n }, create.then_some(generation)));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        // Another run took this number between the listing and the link.
                        let _ = file.unlock();
                        n += 1;
                    }
                    Err(e) => {
                        let _ = std::fs::remove_file(&temp);
                        return Err(e.into());
                    }
                }
            }
        })();
        let _ = gate.unlock();
        result
    }
    #[cfg(target_family = "wasm")]
    {
        let _ = (store, scope, acquired_at);
        let n = counter()? + 1;
        let create = plan()?;
        Ok((ScopeLease { acquisition: n }, create.then_some(n)))
    }
}

/// A deletion's hold: no run starts while it is held. `others` are the live leases the deletion does not touch.
pub(super) struct Deleting {
    #[cfg(not(target_family = "wasm"))]
    gate: Option<std::fs::File>,
    pub others: Vec<Held>,
}

impl Drop for Deleting {
    fn drop(&mut self) {
        #[cfg(not(target_family = "wasm"))]
        if let Some(gate) = self.gate.take() {
            let _ = gate.unlock();
        }
    }
}

/// Holds runs' starts off for a deletion of `store`'s scopes, unless a live lease is one `touches` names: then the
/// deletion fails (`StoreError::InUse`), to be retried once that run has finished.
pub(super) fn deleting(store: &Path, what: &str, touches: impl Fn(&Held) -> bool) -> StoreResult<Deleting> {
    #[cfg(not(target_family = "wasm"))]
    {
        let dir = directory(store)?;
        let gate = gate(&dir)?;
        // A start holds the gate for a few reads and a link: the wait is short.
        gate.lock()?;
        let live = match leases(&dir) {
            Ok(live) => live,
            Err(e) => {
                let _ = gate.unlock();
                return Err(e);
            }
        };
        let (touched, others): (Vec<Held>, Vec<Held>) = live.into_iter().partition(|h| touches(h));
        if let Some(h) = touched.first() {
            let _ = gate.unlock();
            return Err(StoreError::InUse(format!(
                "{what}: analysis {} of {} is running; retry once it has finished",
                h.acquisition, h.scope
            )));
        }
        Ok(Deleting { gate: Some(gate), others })
    }
    #[cfg(target_family = "wasm")]
    {
        let _ = (store, what, touches);
        Ok(Deleting { others: Vec::new() })
    }
}
