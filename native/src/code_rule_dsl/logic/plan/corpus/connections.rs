//! The connections a corpus run works on, set up by their owner and passed through to the run: the tape cache's
//! store, the run's SQL connection, and the cone's connection and store handle. A session keeps them from one run to
//! the next (`Session::connections`): an edit's run opens nothing, and closes nothing. A run of its own opens them
//! and closes them beside it (its connections' close frees what the process kept for them, `close_beside`).
//!
//! A kept connection is readied for the next run (`ready`, `vtab::reuse`): what the last run set up of its own is
//! undone (`DETACH x`, only where its set-up ran), and what the process kept for that run on it is dropped; the
//! connection's functions, symbols and what its runs left in SQL (`plan::Left`) stay.

use crate::content_store::{ContentStore, StoreError};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Default)]
pub(in crate::code_rule_dsl) struct Connections {
    /// Whether they are kept from run to run (a session's).
    kept: bool,
    /// The tape cache (`tape_cache_path`), once opened: `Some(None)` where there is none.
    tapes: Option<Option<ContentStore>>,
    /// The run's connection, on its file.
    run: Option<(PathBuf, Connection)>,
    /// The cone's: it runs beside the main thread, which hands them over with the job and takes them back.
    pub(super) cone: Arc<parking_lot::Mutex<Cone>>,
}

/// The cone of requests' connection and its handle on the store (`kept_strata`), each with its file.
#[derive(Default)]
pub(in crate::code_rule_dsl) struct Cone {
    db: Option<(PathBuf, Connection)>,
    store: Option<(PathBuf, ContentStore)>,
}

impl Connections {
    /// Connections a session keeps from run to run.
    pub(in crate::code_rule_dsl) fn kept() -> Connections {
        Connections { kept: true, tapes: None, run: None, cone: Default::default() }
    }

    /// Readies the kept connections for the next run, before its scratch files are made again (`x` is detached from
    /// the last run's).
    pub(super) fn ready(&mut self) -> rusqlite::Result<()> {
        if let Some((_, db)) = &self.run {
            crate::code_rule_dsl::facts::vtab::reuse(db)?;
        }
        if let Some((_, db)) = &self.cone.lock().db {
            crate::code_rule_dsl::facts::vtab::reuse(db)?;
        }
        Ok(())
    }

    /// The tape cache's store, for the run (handed back by `finish`): kept, or opened.
    pub(super) fn take_tapes(&mut self) -> std::result::Result<Option<ContentStore>, StoreError> {
        match self.tapes.take() {
            Some(tapes) => Ok(tapes),
            None => super::tape_cache(),
        }
    }

    /// The run's connection on `path`: the kept one there, or one opened.
    pub(super) fn take_run(&mut self, path: &Path) -> rusqlite::Result<Connection> {
        match self.run.take() {
            Some((p, db)) if self.kept && p == path => Ok(db),
            other => {
                if let Some((p, db)) = other {
                    super::close_beside(&p, db);
                }
                let mut db = Connection::open(path)?;
                // Its own store's atoms, guarded at commit (decision 0049); a kept connection already is.
                crate::content_store::atoms::Atoms::install(&mut db)?;
                Ok(db)
            }
        }
    }

    /// Takes back what the run was handed: kept for the next run, or closed (the connection beside the run).
    pub(super) fn finish(&mut self, tapes: Option<ContentStore>, path: &Path, db: Connection) {
        if self.kept {
            self.tapes = Some(tapes);
            self.run = Some((path.to_path_buf(), db));
        } else {
            drop(tapes);
            super::close_beside(path, db);
        }
    }

    /// Whether the cone's connection and store are kept from run to run.
    pub(super) fn cone_kept(&self) -> bool {
        self.kept
    }
}

impl Drop for Connections {
    /// Closes what it keeps beside the caller (a session's end): a close frees what the process kept for the
    /// connection (`close_beside`).
    fn drop(&mut self) {
        if let Some((p, db)) = self.run.take() {
            super::close_beside(&p, db);
        }
        if let Some((p, db)) = self.cone.lock().db.take() {
            super::close_beside(&p, db);
        }
    }
}

impl Cone {
    /// The cone's connection on `path` (the run's file): the kept one, or one opened, its functions registered once.
    pub(super) fn take_db(slot: &parking_lot::Mutex<Cone>, path: &Path) -> super::Result<Connection> {
        if let Some((p, db)) = slot.lock().db.take() {
            if p == path {
                return Ok(db);
            }
            super::close_beside(&p, db);
        }
        let mut db = Connection::open(path)?;
        // The atoms guarded at commit, as the run's connection's (decision 0049).
        crate::content_store::atoms::Atoms::install(&mut db)?;
        crate::code_rule_dsl::logic::native::register(&db)?;
        crate::code_rule_dsl::logic::native::register_symbols(&db)?;
        Ok(db)
    }

    /// Opens the cone's connection on `path` and its handle on the store at `store` before the run's first wave
    /// (decision 0104: opening one reads the store), kept in the slot for `take_db` and `take_store`.
    pub(super) fn ready(slot: &parking_lot::Mutex<Cone>, path: &Path, store: Option<&Path>) -> super::Result<()> {
        let db = Self::take_db(slot, path)?;
        let handle = store.map(|p| Self::take_store(slot, p).map(|s| (p.to_path_buf(), s))).transpose().map_err(|e| super::Error::Plan(e.to_string()))?;
        let mut cone = slot.lock();
        cone.db = Some((path.to_path_buf(), db));
        if handle.is_some() {
            cone.store = handle;
        }
        Ok(())
    }

    /// The cone's handle on the store at `path`: the kept one, or one opened.
    pub(super) fn take_store(slot: &parking_lot::Mutex<Cone>, path: &Path) -> std::result::Result<ContentStore, StoreError> {
        match slot.lock().store.take() {
            Some((p, store)) if p == path => Ok(store),
            _ => ContentStore::open(path),
        }
    }

    /// Takes back the cone's connection and store: kept, or the connection closed beside the run.
    pub(super) fn finish(slot: &parking_lot::Mutex<Cone>, kept: bool, path: &Path, db: Connection, store: Option<(PathBuf, ContentStore)>) {
        if kept {
            let mut cone = slot.lock();
            cone.db = Some((path.to_path_buf(), db));
            if store.is_some() {
                cone.store = store;
            }
        } else {
            super::close_beside(path, db);
        }
    }
}

/// What a run was handed (`Connections`), held for the whole run (what borrows the store or the connection is gone
/// before it drops) and handed back as it drops: to the session that keeps them, once the run ended well
/// (`succeeded`); otherwise closed beside the run, and the session opens new ones.
pub(super) struct Handed {
    conns: std::cell::RefCell<Connections>,
    slot: Option<Arc<parking_lot::Mutex<Connections>>>,
    pub(super) tapes: std::cell::OnceCell<Option<ContentStore>>,
    run: std::cell::OnceCell<(PathBuf, Connection)>,
    ok: std::cell::Cell<bool>,
}

impl Handed {
    /// The connections `slot` keeps (a session's), readied for this run, or a run's own.
    pub(super) fn take(slot: Option<Arc<parking_lot::Mutex<Connections>>>) -> rusqlite::Result<Handed> {
        let mut conns = match &slot {
            Some(s) => std::mem::replace(&mut *s.lock(), Connections::kept()),
            None => Connections::default(),
        };
        conns.ready()?;
        Ok(Handed { conns: conns.into(), slot, tapes: Default::default(), run: Default::default(), ok: Default::default() })
    }

    /// The tape cache's store, for the run.
    pub(super) fn tapes(&self) -> std::result::Result<&Option<ContentStore>, StoreError> {
        if self.tapes.get().is_none() {
            let tapes = self.conns.borrow_mut().take_tapes()?;
            let _ = self.tapes.set(tapes);
        }
        Ok(self.tapes.get().expect("set above"))
    }

    /// The run's connection on `path`.
    pub(super) fn run(&self, path: &Path) -> rusqlite::Result<&Connection> {
        if self.run.get().is_none() {
            let db = self.conns.borrow_mut().take_run(path)?;
            let _ = self.run.set((path.to_path_buf(), db));
        }
        Ok(&self.run.get().expect("set above").1)
    }

    /// The cone's slot, and whether what it holds is kept.
    pub(super) fn cone(&self) -> (Arc<parking_lot::Mutex<Cone>>, bool) {
        let conns = self.conns.borrow();
        (Arc::clone(&conns.cone), conns.cone_kept())
    }

    /// The run ended well: what it was handed goes back to its keeper.
    pub(super) fn succeeded(&self) {
        self.ok.set(true);
    }
}

impl Drop for Handed {
    fn drop(&mut self) {
        let tapes = self.tapes.take().flatten();
        let run = self.run.take();
        let mut conns = std::mem::take(self.conns.get_mut());
        match (self.ok.get(), &self.slot, run) {
            (true, Some(slot), Some((path, db))) => {
                conns.finish(tapes, &path, db);
                *slot.lock() = conns;
            }
            (_, _, run) => {
                drop(tapes);
                if let Some((p, db)) = run {
                    super::close_beside(&p, db);
                }
                // The cone's, when its run did not keep them: they close with it.
                drop(conns);
            }
        }
    }
}
