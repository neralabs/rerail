//! Copy-on-write integer relations for temporal rule operators. SQL keeps the evaluation logic.
use super::super::facts::vtab::Modules;
use super::*;
use parking_lot::RwLock;
use std::cell::RefCell;
pub(super) mod fork;
mod tree;
use tree::{Key, Link, Node};
#[derive(Clone)]
struct Row {
    id: i64,
    values: Key,
}
#[derive(Clone)]
pub(super) struct Schema {
    pub columns: Vec<String>,
    pub keys: usize,
    pub nullable: Vec<bool>,
    pub positive: BTreeMap<usize, bool>,
}
impl Schema {
    pub fn integers(width: usize, keys: usize) -> Self {
        Self {
            columns: (0..width).map(|i| format!("c{i}")).collect(),
            keys,
            nullable: vec![false; width],
            positive: BTreeMap::new(),
        }
    }
    /// Only engine-generated state schemas enter this parser. Authored SQL is unchanged.
    pub fn generated(definition: &str) -> Result<Self> {
        let (mut fields, mut depth, mut start) = (Vec::new(), 0usize, 0usize);
        for (i, c) in definition.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                ',' if depth == 0 => {
                    fields.push(definition[start..i].trim());
                    start = i + 1;
                }
                _ => {}
            }
        }
        fields.push(definition[start..].trim());
        let (mut columns, mut nullable, mut positive, mut primary) =
            (Vec::new(), Vec::new(), BTreeMap::new(), None);
        for field in fields {
            if let Some(keys) = field
                .strip_prefix("PRIMARY KEY(")
                .and_then(|s| s.strip_suffix(')'))
            {
                primary = Some(
                    keys.split(',')
                        .map(str::trim)
                        .map(str::to_string)
                        .collect::<Vec<_>>(),
                );
                continue;
            }
            let (name, rest) = field
                .split_once(' ')
                .ok_or_else(|| error("invalid generated state column"))?;
            if !rest.starts_with("INTEGER") {
                return Err(error("versioned state requires integer columns"));
            }
            let i = columns.len();
            columns.push(name.to_string());
            nullable.push(!rest.contains("NOT NULL"));
            if rest.contains(&format!("CHECK({name}>0)")) {
                positive.insert(i, true);
            } else if rest.contains(&format!("CHECK({name}>=0)")) {
                positive.insert(i, false);
            } else if rest.contains("CHECK(") {
                return Err(error("unsupported generated state check"));
            }
        }
        let primary = primary.ok_or_else(|| error("versioned state requires a primary key"))?;
        if primary.is_empty() || !columns.iter().take(primary.len()).eq(&primary) {
            return Err(error("versioned state key must be a column prefix"));
        }
        for flag in nullable.iter_mut().take(primary.len()) {
            *flag = false;
        }
        Ok(Self {
            columns,
            keys: primary.len(),
            nullable,
            positive,
        })
    }
}
#[derive(Clone)]
pub(super) struct Version {
    schema: Arc<Schema>,
    full: Link<Row>,
    primary: Link<Row>,
    ids: Link<Row>,
    indexes: Vec<Link<Row>>,
    distinct: Vec<usize>,
    next: i64,
}
fn key(values: impl IntoIterator<Item = Option<i64>>) -> Key {
    tree::key(values.into_iter().collect())
}
impl Version {
    pub fn empty(schema: Arc<Schema>) -> Self {
        Self {
            indexes: vec![None; schema.columns.len()],
            distinct: vec![0; schema.columns.len()],
            schema,
            full: None,
            primary: None,
            ids: None,
            next: 1,
        }
    }
    fn remove(&mut self, row: &Row) {
        self.full = Node::set(&self.full, row.values.clone(), None);
        self.primary = if self.schema.keys == self.schema.columns.len() {
            self.full.clone()
        } else {
            Node::set(
                &self.primary,
                key(row.values.iter().take(self.schema.keys).copied()),
                None,
            )
        };
        self.ids = Node::set(&self.ids, key([Some(row.id)]), None);
        for (c, &v) in row.values.iter().enumerate() {
            self.indexes[c] = Node::set(&self.indexes[c], key([v, Some(row.id)]), None);
            if !Node::has_prefix(&self.indexes[c], &[v]) {
                self.distinct[c] -= 1;
            }
        }
    }
    fn insert(&mut self, row: Row) {
        self.full = Node::set(&self.full, row.values.clone(), Some(row.clone()));
        self.primary = if self.schema.keys == self.schema.columns.len() {
            self.full.clone()
        } else {
            Node::set(
                &self.primary,
                key(row.values.iter().take(self.schema.keys).copied()),
                Some(row.clone()),
            )
        };
        self.ids = Node::set(&self.ids, key([Some(row.id)]), Some(row.clone()));
        for (c, &v) in row.values.iter().enumerate() {
            if !Node::has_prefix(&self.indexes[c], &[v]) {
                self.distinct[c] += 1;
            }
            self.indexes[c] =
                Node::set(&self.indexes[c], key([v, Some(row.id)]), Some(row.clone()));
        }
    }
    pub fn difference(&self, new: &Self) -> (Vec<Vec<Option<i64>>>, Vec<Vec<Option<i64>>>, usize) {
        let (mut plus, mut minus, mut visited) = (vec![], vec![], 0);
        Node::difference(&self.full, &new.full, &mut plus, &mut minus, &mut visited);
        (
            plus.into_iter().map(|r| r.values.to_vec()).collect(),
            minus.into_iter().map(|r| r.values.to_vec()).collect(),
            visited,
        )
    }
    pub fn same(&self, other: &Self) -> bool {
        match (&self.full, &other.full) {
            (None, None) => true,
            (Some(a), Some(b)) => a.size == b.size && a.digest == b.digest,
            _ => false,
        }
    }
}
impl Source for Version {
    fn ordered(&self, equal: &[usize], order: &[(usize, bool)]) -> bool {
        let mut wanted = 0;
        for &(column, reverse) in order {
            if reverse != order[0].1 || column >= self.schema.keys {
                return false;
            }
            while wanted < column && equal.contains(&wanted) {
                wanted += 1;
            }
            if wanted != column && !equal.contains(&column) {
                return false;
            }
            wanted = column + 1;
        }
        true
    }
    fn ordered_range(&self, equal: &[(usize, i64)]) -> Option<std::ops::Range<usize>> {
        let prefix = (0..self.schema.keys)
            .map_while(|c| {
                equal
                    .iter()
                    .find(|(column, _)| *column == c)
                    .map(|(_, v)| Some(*v))
            })
            .collect::<Vec<_>>();
        Some(Node::prefix_ranks(&self.full, &prefix))
    }
    fn columns(&self) -> Vec<String> {
        self.schema.columns.clone()
    }
    fn len(&self) -> usize {
        tree::size(&self.full)
    }
    fn estimated_matches(&self, c: usize, batch: bool) -> usize {
        (self.len() / self.distinct[c].max(1))
            .max(1)
            .saturating_mul(if batch { 8 } else { 1 })
    }
    fn indexed(&self, _: usize) -> bool {
        true
    }
    fn composite(&self) -> bool {
        true
    }
    fn lookup_all(&self, eq: &[(usize, i64)]) -> Option<Vec<usize>> {
        if (0..self.schema.keys).all(|c| eq.iter().any(|(col, _)| *col == c)) {
            let pk = (0..self.schema.keys)
                .map(|c| Some(eq.iter().find(|(col, _)| *col == c).unwrap().1))
                .collect::<Vec<_>>();
            return Some(
                Node::get(&self.primary, &pk)
                    .filter(|r| eq.iter().all(|&(c, v)| r.values[c] == Some(v)))
                    .and_then(|r| Node::rank(&self.full, &r.values))
                    .into_iter()
                    .collect(),
            );
        }
        let &(c, v) = eq
            .iter()
            .min_by_key(|(c, _)| self.estimated_matches(*c, false))?;
        Some(
            self.lookup(c, &[v])?
                .into_iter()
                .filter(|&row| {
                    eq.iter()
                        .all(|&(c, v)| Node::nth(&self.full, row).values[c] == Some(v))
                })
                .collect(),
        )
    }
    fn lookup(&self, c: usize, values: &[i64]) -> Option<Vec<usize>> {
        let mut found = Vec::new();
        for &v in values {
            Node::prefix(&self.indexes[c], &[Some(v)], &mut found);
        }
        Some(
            found
                .iter()
                .filter_map(|r| Node::rank(&self.full, &r.values))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
        )
    }
    fn value(&self, row: usize, c: usize) -> i64 {
        Node::nth(&self.full, row).values[c].unwrap_or(0)
    }
    fn cell(&self, row: usize, c: usize) -> SqlValue {
        Node::nth(&self.full, row).values[c].map_or(SqlValue::Null, SqlValue::Integer)
    }
    fn row_id(&self, row: usize) -> i64 {
        Node::nth(&self.full, row).id
    }
}
struct Mutable {
    id: usize,
    state: RwLock<Version>,
    undo: Arc<RwLock<Option<Undo>>>,
}
fn constraint(code: i32, message: &str) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(if code & 255 == rusqlite::ffi::SQLITE_CONSTRAINT {
            rusqlite::ffi::SQLITE_CONSTRAINT
        } else {
            code
        }),
        Some(message.into()),
    )
}
impl Source for Mutable {
    fn ordered(&self, equal: &[usize], order: &[(usize, bool)]) -> bool {
        self.state.read().ordered(equal, order)
    }
    fn ordered_range(&self, equal: &[(usize, i64)]) -> Option<std::ops::Range<usize>> {
        self.state.read().ordered_range(equal)
    }
    fn frozen(&self) -> Option<Arc<dyn Source>> {
        Some(Arc::new(self.state.read().clone()))
    }
    fn columns(&self) -> Vec<String> {
        self.state.read().columns()
    }
    fn len(&self) -> usize {
        self.state.read().len()
    }
    fn estimated_matches(&self, c: usize, b: bool) -> usize {
        self.state.read().estimated_matches(c, b)
    }
    fn indexed(&self, _: usize) -> bool {
        true
    }
    fn composite(&self) -> bool {
        true
    }
    fn lookup_all(&self, eq: &[(usize, i64)]) -> Option<Vec<usize>> {
        self.state.read().lookup_all(eq)
    }
    fn lookup(&self, c: usize, values: &[i64]) -> Option<Vec<usize>> {
        self.state.read().lookup(c, values)
    }
    fn value(&self, row: usize, c: usize) -> i64 {
        self.state.read().value(row, c)
    }
    fn cell(&self, row: usize, c: usize) -> SqlValue {
        self.state.read().cell(row, c)
    }
    fn row_id(&self, row: usize) -> i64 {
        self.state.read().row_id(row)
    }
    fn writable(&self) -> bool {
        true
    }
    fn write(
        &self,
        old: Option<i64>,
        requested: Option<i64>,
        values: Option<Vec<Option<i64>>>,
        replace: bool,
        _ignore: bool,
    ) -> rusqlite::Result<i64> {
        use rusqlite::ffi;
        let mut state = self.state.write();
        let previous = old.and_then(|id| Node::get(&state.ids, &[Some(id)]).cloned());
        let Some(values) = values else {
            if let Some(row) = previous {
                if let Some(log) = self.undo.write().as_mut() {
                    log.versions.entry(self.id).or_insert_with(|| state.clone());
                }
                state.remove(&row);
            }
            return Ok(old.unwrap_or(0));
        };
        if values.len() != state.schema.columns.len() {
            return Err(constraint(ffi::SQLITE_CONSTRAINT, "versioned row arity"));
        }
        for (c, v) in values.iter().enumerate() {
            if v.is_none() && !state.schema.nullable[c] {
                return Err(constraint(
                    ffi::SQLITE_CONSTRAINT_NOTNULL,
                    "null in versioned integer relation",
                ));
            }
            if let Some(&strict) = state.schema.positive.get(&c) {
                if v.is_none_or(|v| if strict { v <= 0 } else { v < 0 }) {
                    return Err(constraint(
                        ffi::SQLITE_CONSTRAINT_CHECK,
                        "invalid support count",
                    ));
                }
            }
        }
        let existing = Node::get(&state.primary, &values[..state.schema.keys]).cloned();
        if existing.as_ref().is_some_and(|r| Some(r.id) != old) && !replace {
            return Err(constraint(
                ffi::SQLITE_CONSTRAINT_PRIMARYKEY,
                "duplicate versioned key",
            ));
        }
        let id = requested
            .or(old)
            .or_else(|| existing.as_ref().map(|r| r.id))
            .unwrap_or(state.next);
        let collision = Node::get(&state.ids, &[Some(id)])
            .cloned()
            .filter(|r| Some(r.id) != old && existing.as_ref().is_none_or(|e| e.id != r.id));
        if collision.is_some() && !replace {
            return Err(constraint(
                ffi::SQLITE_CONSTRAINT_ROWID,
                "duplicate versioned rowid",
            ));
        }
        let next = state.next.max(
            id.checked_add(1)
                .ok_or_else(|| constraint(ffi::SQLITE_FULL, "versioned row ids exhausted"))?,
        );
        if previous
            .as_ref()
            .or(existing.as_ref())
            .is_some_and(|r| r.id == id && r.values.as_slice() == values)
        {
            return Ok(id);
        }
        if let Some(log) = self.undo.write().as_mut() {
            log.versions.entry(self.id).or_insert_with(|| state.clone());
        }
        if let Some(row) = previous {
            state.remove(&row);
        }
        if let Some(row) = existing.filter(|r| Some(r.id) != old) {
            state.remove(&row);
        }
        if let Some(row) = collision {
            state.remove(&row);
        }
        state.next = next;
        state.insert(Row {
            id,
            values: tree::key(values),
        });
        Ok(id)
    }
}

#[derive(Default)]
struct Undo {
    versions: BTreeMap<usize, Version>,
    created: Vec<String>,
}
#[derive(Default)]
pub(super) struct Store {
    modules: Modules,
    tables: RefCell<BTreeMap<String, Arc<Mutable>>>,
    next: std::cell::Cell<usize>,
    undo: Arc<RwLock<Option<Undo>>>,
}
pub(super) struct Transaction<'a> {
    store: &'a Store,
    db: &'a rusqlite::Connection,
    committed: bool,
}
impl Store {
    pub fn close(&self, db: &rusqlite::Connection) -> Result<()> {
        let names = self.tables.borrow().keys().cloned().collect::<Vec<_>>();
        for name in names {
            self.remove(db, &name)?;
        }
        Ok(())
    }
    pub fn transaction<'a>(&'a self, db: &'a rusqlite::Connection) -> Result<Transaction<'a>> {
        let mut undo = self.undo.write();
        if undo.is_some() {
            return Err(error("nested native version transaction"));
        }
        *undo = Some(Undo::default());
        Ok(Transaction {
            store: self,
            db,
            committed: false,
        })
    }
    pub fn contains(&self, name: &str) -> bool {
        self.tables.borrow().contains_key(name)
    }
    pub fn version(&self, name: &str) -> Version {
        self.tables.borrow()[name].state.read().clone()
    }
    pub fn create(&self, db: &rusqlite::Connection, name: &str, mut schema: Schema) -> Result<()> {
        if schema.columns.is_empty()
            || schema.keys == 0
            || schema.keys > schema.columns.len()
            || schema.nullable.len() != schema.columns.len()
            || schema.columns.iter().collect::<BTreeSet<_>>().len() != schema.columns.len()
        {
            return Err(error("invalid versioned integer schema"));
        }
        for nullable in schema.nullable.iter_mut().take(schema.keys) {
            *nullable = false;
        }
        self.fork_version(db, name, Version::empty(Arc::new(schema)))
    }
    pub fn fork(&self, db: &rusqlite::Connection, old: &str, new: &str) -> Result<()> {
        self.fork_version(db, new, self.version(old))
    }
    pub fn fork_version(
        &self,
        db: &rusqlite::Connection,
        name: &str,
        version: Version,
    ) -> Result<()> {
        if self.tables.borrow().contains_key(name) {
            return Err(error("versioned table already exists"));
        }
        let id = self.next.get();
        self.next.set(id + 1);
        let table = Arc::new(Mutable {
            id,
            state: RwLock::new(version),
            undo: self.undo.clone(),
        });
        self.modules.bind(db, name, table.clone()).map_err(error)?;
        self.tables.borrow_mut().insert(name.into(), table);
        if let Some(undo) = self.undo.write().as_mut() {
            undo.created.push(name.into());
        }
        Ok(())
    }
    pub fn remove(&self, db: &rusqlite::Connection, name: &str) -> Result<()> {
        self.modules.unregister(db, name).map_err(error)?;
        self.tables.borrow_mut().remove(name);
        Ok(())
    }
}
impl Transaction<'_> {
    pub fn create(&mut self, name: &str, schema: Schema) -> Result<()> {
        self.store.create(self.db, name, schema)
    }
    pub fn fork(&mut self, old: &str, new: &str) -> Result<()> {
        self.store.fork(self.db, old, new)
    }
    pub fn commit(mut self) {
        self.store.undo.write().take();
        self.committed = true;
    }
}
impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let undo = self.store.undo.write().take();
        if let Some(undo) = undo {
            for table in self.store.tables.borrow().values() {
                if let Some(version) = undo.versions.get(&table.id) {
                    *table.state.write() = version.clone();
                }
            }
            for name in &undo.created {
                let _ = self.store.remove(self.db, name);
            }
        }
    }
}

#[cfg(test)]
mod operators;
#[cfg(test)]
mod tests;
