//! Bind private roots as writable temporary tables or read-only eponymous modules.
use super::*;

impl Store {
    pub fn create(&self, db: &rusqlite::Connection, name: &str, schema: Schema) -> Result<()> {
        self.create_access(db, name, schema, true)
    }
    pub fn create_readonly(
        &self,
        db: &rusqlite::Connection,
        name: &str,
        schema: Schema,
    ) -> Result<()> {
        self.create_access(db, name, schema, false)
    }
    fn create_access(
        &self,
        db: &rusqlite::Connection,
        name: &str,
        mut schema: Schema,
        writable: bool,
    ) -> Result<()> {
        if schema.columns.is_empty()
            || schema.keys == 0
            || schema.keys > schema.columns.len()
            || schema.nullable.len() != schema.columns.len()
            || schema.columns.iter().collect::<BTreeSet<_>>().len() != schema.columns.len()
        {
            return Err(error("invalid versioned integer schema"));
        }
        for flag in schema.nullable.iter_mut().take(schema.keys) {
            *flag = false;
        }
        self.fork_access(db, name, Version::empty(Arc::new(schema)), writable)
    }
    pub fn fork(&self, db: &rusqlite::Connection, old: &str, new: &str) -> Result<()> {
        let writable = self.tables.borrow()[&(self.scope.get(), old.into())].sql_writable;
        self.fork_access(db, new, self.version(old), writable)
    }
    pub fn fork_version(
        &self,
        db: &rusqlite::Connection,
        name: &str,
        version: Version,
    ) -> Result<()> {
        let writable = self.slots.borrow().get(name).is_none_or(|s| s.sql_writable);
        self.fork_access(db, name, version, writable)
    }
    fn fork_access(
        &self,
        db: &rusqlite::Connection,
        name: &str,
        version: Version,
        writable: bool,
    ) -> Result<()> {
        if self.contains(name) {
            return Err(error("versioned table already exists"));
        }
        if self
            .slots
            .borrow()
            .get(name)
            .is_some_and(|s| *s.schema != *version.schema || s.sql_writable != writable)
        {
            return Err(error("native SQL slot changed schema"));
        }
        let schema = version.schema.clone();
        let scope = self.scope.get();
        let key = (scope, name.into());
        let table = self.allocate(key.clone(), version, writable);
        self.modules
            .bind(db, name, table.clone())
            .map_err(|e| error(format!("{name}: native SQL slot: {e}")))?;
        self.tables.borrow_mut().insert(key, table);
        self.slots
            .borrow_mut()
            .entry(name.into())
            .or_insert_with(|| Slot {
                schema,
                sql_writable: writable,
                owners: BTreeSet::new(),
            })
            .owners
            .insert(scope);
        Ok(())
    }
}
