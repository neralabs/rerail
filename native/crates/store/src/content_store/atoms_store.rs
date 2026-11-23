//! The store's atoms (rerail-atoms, below the store): its calls over its arena (`intern_atoms`, the writer's
//! flush, the commit that writes the pending ones), and the kept facts installed beside them on a store's
//! connection (`install_atoms`, `attach_atoms`; decisions 0456 K8, 0439).
#[allow(unused_imports)]
use super::Cached as _;
use super::atoms::{Atoms, ProcessIds, StoreIds, mapped};
use super::{StoreError, StoreResult, Sym};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;

/// Registers the atoms on `connection`, where it is opened (`Atoms::install`: their table and commit hook), and the
/// kept facts beside them on a store's.
pub fn install_atoms(connection: &mut Connection) -> rusqlite::Result<Arc<Atoms>> {
    let atoms = Atoms::install(connection)?;
    kept_beside(connection)?;
    Ok(atoms)
}

/// Registers the atoms' table on `connection`, handed over after it was opened (`Atoms::attach`), and the kept facts
/// beside them on a store's.
pub fn attach_atoms(connection: &Connection) -> rusqlite::Result<Arc<Atoms>> {
    let atoms = Atoms::attach(connection)?;
    kept_beside(connection)?;
    Ok(atoms)
}

fn kept_beside(connection: &Connection) -> rusqlite::Result<()> {
    if let Some(path) = connection.path().filter(|path| !path.is_empty()) {
        super::kept_facts::vtab::install(connection, Path::new(path))?;
        // The prefilter's tables as its kept facts (t33 steps 6 and 7).
        super::prefilter::vtab::install(connection, Path::new(path))?;
    }
    Ok(())
}

impl super::ContentStore {
    /// The atom of each of `values`, giving out new ones. They are written with the next write
    /// transaction on the store, or in one of their own when a flush is due (decision 0049 §4).
    /// This process's atoms of the store, for a reader that interns away from the store handle (a lazily
    /// decoded source). Pending atoms are appended by the store's next flush.
    pub fn atoms_handle(&self) -> std::sync::Arc<Atoms> {
        std::sync::Arc::clone(&self.atoms)
    }

    pub fn intern_atoms(&self, values: &[&str]) -> StoreResult<Vec<Sym>> {
        let ids = self.atoms.intern_all(values)?;
        // A store with a writer has it append them (decision 0104: only its writers write); one without, now.
        if self.write_back.is_some() {
            // Due, and with atoms a flush would write (not only those a component reserves, `Atoms::pending`).
            if self.atoms.due_now() && self.atoms.flush_wanted() && !self.atoms.flush_queued.swap(true, Ordering::AcqRel) {
                self.queue_atoms_flush();
            }
        } else if self.atoms.due() && self.atoms.flush_wanted() {
            self.flush_atoms()?;
        }
        Ok(ids)
    }

    /// The pending atoms, appended by the store's writer behind the run (decision 0104).
    pub fn queue_atoms_flush(&self) {
        self.write_back_syms(crate::content_store::write_back::Syms::None, "the pending atoms", 0, Vec::new(), crate::content_store::write_back::Work::atoms(true));
    }

    /// The pending atoms, appended by the store's writer behind the run's other writes, as `what`.
    pub fn queue_atoms(&self, what: &'static str) {
        self.write_back_syms(crate::content_store::write_back::Syms::None, what, 0, Vec::new(), crate::content_store::write_back::Work::atoms(false));
    }

    /// Appends the pending atoms now, unless another writer holds the store (a later write will).
    /// The database at `path` reads and writes this store's atoms in this process (`Atoms::share`): a
    /// connection there resolves symbols as the store does. Call before opening it.
    pub fn share_atoms_with(&self, path: &Path) {
        self.atoms.share(path);
    }

    pub fn flush_atoms(&self) -> StoreResult<()> {
        if !self.atoms.pending() {
            return Ok(());
        }
        let atoms = std::sync::Arc::clone(&self.atoms);
        match mapped(|| self.immediate_transaction_no_wait(|t| atoms.begin_write(t))) {
            Ok(()) | Err(StoreError::Busy) => Ok(()),
            Err(StoreError::Database(error))
                if matches!(error.sqlite_error_code(), Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)) =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// `values` (this process's symbols) as the store numbers them, in this handle's open write transaction (the
    /// writer's batch): unchanged unless forked (decision 0170).
    pub fn symbols_to_store<'v>(&self, ids: ProcessIds<'v>) -> StoreResult<StoreIds<'v>> {
        if !self.atoms.forked() {
            return Ok(StoreIds(ids.0));
        }
        let connection = self.connection.lock().map_err(StoreError::Invalid)?;
        self.atoms.to_store(&connection, ids)
    }

    /// The atom of `value`, if the store holds it.
    pub fn atom(&self, value: &str) -> StoreResult<Option<Sym>> {
        self.atoms.lookup(value)
    }

    /// The atoms' writes so far: (appends, atoms, bytes), and the atoms pending now.
    pub fn atom_writes(&self) -> ((u64, u64, u64), usize) {
        self.atoms.writes()
    }

    /// Heap bytes the arena holds (counted against the fact pool, K1).
    pub fn atom_bytes(&self) -> usize {
        self.atoms.bytes()
    }

    /// The string of `atom`, if the store holds it.
    pub fn atom_value(&self, atom: Sym) -> StoreResult<Option<String>> {
        self.atoms.value(atom)
    }

    /// Commits `transaction`, appending the pending atoms first if it writes the store.
    /// Inside the writer's batch (`super::connection::begin`) it releases its savepoint only: the batch's commit
    /// appends the atoms.
    pub(super) fn commit(&self, transaction: super::connection::Tx<'_>) -> StoreResult<()> {
        let super::connection::Tx::Top(transaction) = transaction else {
            transaction.commit()?;
            return Ok(());
        };
        let upto = self.atoms.write_pending(&transaction)?;
        transaction.commit()?;
        self.atoms.committed(upto);
        Ok(())
    }
}

#[cfg(test)]
#[path = "atoms/tests.rs"]
mod tests;

/// The store's kept facts as the SQL tables of their names (`prefilter::vtab`: the derivation graph, the search
/// index, the prefilter's) on `connection`, a plain connection to a store's file: for checks of what a store holds on
/// disk, beside `materialize_stored`.
pub fn install_kept_tables(connection: &Connection) -> StoreResult<()> {
    let path = connection.path().filter(|path| !path.is_empty()).ok_or_else(|| StoreError::Invalid("a connection to no store's file".to_owned()))?;
    super::prefilter::vtab::install(connection, Path::new(path))?;
    Ok(())
}
