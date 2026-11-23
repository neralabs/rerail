//! Bind private roots as writable temporary tables or read-only eponymous modules.
use super::*;

impl Store {
    /// Publish a diagnostic alias without copying its rows or persistent indexes.
    pub fn alias_readonly(
        &self,
        db: &rusqlite::Connection,
        name: &str,
        version: Version,
        columns: Vec<String>,
    ) -> Result<()> {
        let mut version = version.renamed(columns)?;
        if self.contains(name) {
            let old = self.version(name);
            if old.schema != version.schema
                || self.tables.borrow()[&(self.scope.get(), name.into())].sql_writable
            {
                return Err(error("diagnostic alias changed schema or access mode"));
            }
            // An extension can request extra indexes on the public diagnostic.
            // Maintain those indexes with a sparse patch instead of rebuilding them
            // or losing them when the frame publishes its next shared root.
            if old
                .secondary
                .iter()
                .zip(version.secondary.iter())
                .any(|(&keep, &incoming)| keep && !incoming)
            {
                let (plus, minus, _) = old.difference(&version);
                let keys = plus
                    .iter()
                    .chain(&minus)
                    .map(|r| r[..old.schema.keys].to_vec())
                    .collect();
                version = old.patched_rows(keys, plus, false)?;
            }
            self.replace(name, version)
        } else {
            self.fork_access(db, name, version, false)
        }
    }
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
    pub fn declare_readonly_empty(
        &self,
        db: &rusqlite::Connection,
        name: &str,
        schema: Schema,
    ) -> Result<()> {
        if self.contains(name) {
            let old = self.version(name);
            if *old.schema != schema
                || self.tables.borrow()[&(self.scope.get(), name.into())].sql_writable
            {
                return Err(error("read-only cache slot changed schema or access mode"));
            }
            self.replace(name, old.empty_like())
        } else {
            self.create_readonly(db, name, schema)
        }
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
    pub fn fork_readonly_version(&self, db: &rusqlite::Connection, name: &str, version: Version) -> Result<()> {
        self.fork_access(db, name, version, false)
    }
    fn fork_access(
        &self,
        db: &rusqlite::Connection,
        name: &str,
        version: Version,
        writable: bool,
    ) -> Result<()> {
        if self.contains(name) {
            return Err(error(format!("versioned table {name} already exists in scope {}",self.current_scope())));
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
