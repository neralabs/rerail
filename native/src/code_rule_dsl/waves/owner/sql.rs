//! The frontier kernel can borrow an existing connection or use the product's SQL owner.
use super::super::storage::Storage;
use super::Database;
use rusqlite::{CachedStatement, Connection};
use std::{cell::RefCell, ops::Deref};

pub(in crate::code_rule_dsl::waves) trait SqlOwner:
    Deref<Target = Connection>
{
    fn storage(&self) -> &RefCell<Storage>;
    fn prepare_step(&self, sql: &str) -> rusqlite::Result<CachedStatement<'_>> {
        self.prepare_cached(sql)
    }
}
impl SqlOwner for Database {
    fn storage(&self) -> &RefCell<Storage> {
        &self.storage
    }
    fn prepare_step(&self, sql: &str) -> rusqlite::Result<CachedStatement<'_>> {
        self.prepare_execution(sql)
    }
}

pub(in crate::code_rule_dsl::waves) struct Borrowed<'a> {
    pub db: &'a Connection,
    storage: RefCell<Storage>,
}
impl<'a> Borrowed<'a> {
    pub fn new(db: &'a Connection) -> Self {
        Self {
            db,
            storage: RefCell::default(),
        }
    }
}
impl Deref for Borrowed<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.db
    }
}
impl SqlOwner for Borrowed<'_> {
    fn storage(&self) -> &RefCell<Storage> {
        &self.storage
    }
}
