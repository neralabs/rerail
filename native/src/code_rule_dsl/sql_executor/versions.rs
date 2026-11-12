//! Copy-on-write integer relations for temporal rule operators. SQL keeps the evaluation logic.
use super::super::facts::vtab::Modules;
use super::*;
use parking_lot::RwLock;
use std::cell::RefCell;
pub mod bindings;
pub mod bulk;
pub mod difference;
pub mod fork;
pub mod patch;
pub mod probes;
pub mod scan;
pub mod tree;
use tree::{Key, Link, Node};
#[derive(Clone)]
struct Row {
    id: i64,
    values: Key,
}
#[derive(Clone, PartialEq, Eq)]
pub struct Schema {
    pub columns: Vec<String>,
    pub keys: usize,
    pub nullable: Vec<bool>,
    pub positive: BTreeMap<usize, bool>,
}
impl Schema {
    fn valid_row(&self, values: &[Option<i64>]) -> bool {
        values.len() == self.columns.len()
            && values.iter().enumerate().all(|(c, v)| {
                (v.is_some() || self.nullable[c])
                    && self.positive.get(&c).is_none_or(|&strict| {
                        v.is_some_and(|v| if strict { v > 0 } else { v >= 0 })
                    })
            })
    }
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
pub struct Version {
    schema: Arc<Schema>,
    full: Link<Row>,
    primary: Link<Row>,
    ids: Link<Row>,
    indexes: Vec<Link<Row>>,
    secondary: Arc<[bool]>,
    lazy: bool,
    distinct: Vec<usize>,
    next: i64,
}
fn key(values: impl IntoIterator<Item = Option<i64>>) -> Key {
    tree::key(values.into_iter().collect())
}
impl Version {
    pub fn distinct_values(&self, column: usize) -> Option<usize> {
        (column < self.distinct.len() && (column == 0 || self.secondary[column]))
            .then(|| self.distinct[column])
    }
    pub fn arity(&self) -> usize {
        self.schema.columns.len()
    }
    pub fn same_schema(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.schema, &other.schema) || self.schema == other.schema
    }
    pub fn empty_like(&self) -> Self {
        let mut empty = Self::empty(self.schema.clone());
        empty.secondary = self.secondary.clone();
        empty.lazy = self.lazy;
        empty
    }
    pub fn empty(schema: Arc<Schema>) -> Self {
        Self {
            lazy: false,
            secondary: (0..schema.columns.len()).map(|c| c > 0).collect(),
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
            Node::set_index(
                &self.primary,
                key(row.values.iter().take(self.schema.keys).copied()),
                None,
            )
        };
        self.ids = Node::set_index(&self.ids, key([Some(row.id)]), None);
        for (c, &v) in row.values.iter().enumerate() {
            if c == 0 {
                if !Node::has_prefix(&self.full, &[v]) {
                    self.distinct[0] -= 1;
                }
                continue;
            }
            if !self.secondary[c] {
                continue;
            }
            self.indexes[c] = Node::set_index(&self.indexes[c], key([v, Some(row.id)]), None);
            if !Node::has_prefix(&self.indexes[c], &[v]) {
                self.distinct[c] -= 1;
            }
        }
    }
    fn insert(&mut self, row: Row) {
        if !Node::has_prefix(&self.full, &[row.values[0]]) {
            self.distinct[0] += 1;
        }
        self.full = Node::set(&self.full, row.values.clone(), Some(row.clone()));
        self.primary = if self.schema.keys == self.schema.columns.len() {
            self.full.clone()
        } else {
            Node::set_index(
                &self.primary,
                key(row.values.iter().take(self.schema.keys).copied()),
                Some(row.clone()),
            )
        };
        self.ids = Node::set_index(&self.ids, key([Some(row.id)]), Some(row.clone()));
        for (c, &v) in row.values.iter().enumerate() {
            if c == 0 || !self.secondary[c] {
                continue;
            }
            if !Node::has_prefix(&self.indexes[c], &[v]) {
                self.distinct[c] += 1;
            }
            self.indexes[c] =
                Node::set_index(&self.indexes[c], key([v, Some(row.id)]), Some(row.clone()));
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
    pub fn shares_rows(&self, other: &Self) -> bool {
        match (&self.full, &other.full) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}
impl Source for Version {
    fn row_stream(
        &self,
        range: std::ops::Range<usize>,
        reverse: bool,
    ) -> Option<Box<dyn super::super::facts::vtab::RowStream>> {
        Some(self.stream(range, reverse))
    }
    fn direct_all(
        &self,
        equal: &[(usize, i64)],
    ) -> Option<Vec<super::super::facts::vtab::ReadRow>> {
        if !(0..self.schema.keys).all(|c| equal.iter().any(|&(k, _)| k == c)) {
            return None;
        }
        let pk = (0..self.schema.keys)
            .map(|c| Some(equal.iter().find(|(k, _)| *k == c).unwrap().1))
            .collect::<Vec<_>>();
        Some(
            Node::get(&self.primary, &pk)
                .filter(|r| equal.iter().all(|&(c, v)| r.values[c] == Some(v)))
                .map(|r| super::super::facts::vtab::ReadRow {
                    id: r.id,
                    cells: r.values.clone(),
                })
                .into_iter()
                .collect(),
        )
    }
    fn direct_one(
        &self,
        column: usize,
        values: &[i64],
    ) -> Option<Vec<super::super::facts::vtab::ReadRow>> {
        if self.schema.keys != 1 || column != 0 {
            return None;
        }
        Some(
            values
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .filter_map(|v| {
                    Node::get(&self.primary, &[Some(v)]).map(|r| {
                        super::super::facts::vtab::ReadRow {
                            id: r.id,
                            cells: r.values.clone(),
                        }
                    })
                })
                .collect(),
        )
    }
    fn caches_rows(&self) -> bool {
        true
    }
    fn row_cells(&self, row: usize) -> Option<(i64, Arc<dyn super::super::facts::vtab::RowCells>)> {
        let row = Node::nth(&self.full, row);
        Some((row.id, row.values.clone()))
    }
    fn ordered(&self, equal: &[usize], order: &[(usize, bool)]) -> bool {
        // Ordered readers seek only a leading primary-key prefix. Nonleading
        // constraints need the selective index path, then a sort of its matches.
        // Pricing an ordered full scan as a point lookup makes sparse joins scan
        // every disconnected group when UNION requests canonical row order.
        if equal.iter().any(|&c| (0..c).any(|p| !equal.contains(&p))) {
            return false;
        }
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
        if self.lazy && c > 0 && !self.secondary[c] {
            return self
                .len()
                .div_ceil(100)
                .max(1)
                .saturating_mul(if batch { 8 } else { 1 });
        }
        if !self.indexed(c) {
            return self.len().max(1);
        }
        (self.len() / self.distinct[c].max(1))
            .max(1)
            .saturating_mul(if batch { 8 } else { 1 })
    }
    fn indexed(&self, c: usize) -> bool {
        c == 0 || self.secondary[c] || self.lazy
    }
    fn primary_key_len(&self) -> Option<usize> {
        Some(self.schema.keys)
    }
    fn lookup_primary_batch(&self, values: &[Vec<i64>]) -> Option<Vec<usize>> {
        Some(self.primary_batch(values))
    }
    fn estimated_matches_all(&self, columns: &[usize]) -> usize {
        if (0..self.schema.keys).all(|c| columns.contains(&c)) {
            1
        } else {
            columns
                .iter()
                .map(|&c| self.estimated_matches(c, false))
                .min()
                .unwrap_or(self.len())
                .max(1)
        }
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
        let prefix = (0..self.schema.keys)
            .map_while(|c| eq.iter().find(|(col, _)| *col == c).map(|(_, v)| Some(*v)))
            .collect::<Vec<_>>();
        let range = Node::prefix_ranks(&self.full, &prefix);
        let secondary = eq
            .iter()
            .filter(|(c, _)| *c > 0 && self.indexed(*c))
            .min_by_key(|(c, _)| self.estimated_matches(*c, false));
        if !prefix.is_empty()
            && secondary.is_none_or(|(c, _)| range.len() <= self.estimated_matches(*c, false))
        {
            return Some(
                range
                    .filter(|&row| {
                        eq.iter()
                            .all(|&(c, v)| Node::nth(&self.full, row).values[c] == Some(v))
                    })
                    .collect(),
            );
        }
        let &(c, v) = secondary?;
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
        if c == 0 {
            let values = values.iter().copied().collect::<BTreeSet<_>>();
            return Some(
                values
                    .into_iter()
                    .flat_map(|v| Node::prefix_ranks(&self.full, &[Some(v)]))
                    .collect(),
            );
        }
        if !self.secondary[c] {
            return None;
        }
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
impl super::super::facts::vtab::RowCells for tree::Cells {
    fn cell(&self, column: usize) -> SqlValue {
        self[column].map_or(SqlValue::Null, SqlValue::Integer)
    }
}
struct Mutable {
    id: usize,
    sql_writable: bool,
    state: RwLock<Arc<Version>>,
    undo: Arc<RwLock<Vec<Undo>>>,
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
    fn prepare_lookup(&self, columns: &[usize]) {
        let lazy = self.state.read().lazy;
        if lazy {
            self.prepare_index(columns);
        }
    }
    fn ordered(&self, equal: &[usize], order: &[(usize, bool)]) -> bool {
        self.state.read().ordered(equal, order)
    }
    fn ordered_range(&self, equal: &[(usize, i64)]) -> Option<std::ops::Range<usize>> {
        self.state.read().ordered_range(equal)
    }
    fn frozen(&self) -> Option<Arc<dyn Source>> {
        Some(self.state.read().clone())
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
    fn indexed(&self, c: usize) -> bool {
        self.state.read().indexed(c)
    }
    fn prepare_index(&self, columns: &[usize]) {
        let mut state = self.state.write();
        if columns.iter().any(|&c| c > 0 && !state.secondary[c]) {
            for log in self.undo.write().iter_mut() {
                log.versions.entry(self.id).or_insert_with(|| state.clone());
            }
            Arc::make_mut(&mut state).ensure_indexes(columns);
        }
    }
    fn primary_key_len(&self) -> Option<usize> {
        self.state.read().primary_key_len()
    }
    fn lookup_primary_batch(&self, values: &[Vec<i64>]) -> Option<Vec<usize>> {
        self.state.read().lookup_primary_batch(values)
    }
    fn estimated_matches_all(&self, columns: &[usize]) -> usize {
        self.state.read().estimated_matches_all(columns)
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
        self.sql_writable
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
        if !self.sql_writable {
            return Err(constraint(
                ffi::SQLITE_READONLY,
                "native SQL slot is read-only",
            ));
        }
        let mut state = self.state.write();
        let previous = old.and_then(|id| Node::get(&state.ids, &[Some(id)]).cloned());
        let Some(values) = values else {
            if let Some(row) = previous {
                for log in self.undo.write().iter_mut() {
                    log.versions.entry(self.id).or_insert_with(|| state.clone());
                }
                Arc::make_mut(&mut state).remove(&row);
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
        for log in self.undo.write().iter_mut() {
            log.versions.entry(self.id).or_insert_with(|| state.clone());
        }
        let state = Arc::make_mut(&mut state);
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

type TableKey = (usize, String);
#[derive(Default)]
struct Undo {
    versions: BTreeMap<usize, Arc<Version>>,
    created: Vec<(TableKey, usize)>,
    removed: BTreeMap<TableKey, Arc<Mutable>>,
    bindings: BTreeMap<String, super::super::facts::vtab::Binding>,
}
struct Slot {
    schema: Arc<Schema>,
    sql_writable: bool,
    owners: BTreeSet<usize>,
}
#[derive(Default)]
pub struct Store {
    reuse: RefCell<BTreeMap<TableKey, Version>>,
    modules: Modules,
    tables: RefCell<BTreeMap<TableKey, Arc<Mutable>>>,
    slots: RefCell<BTreeMap<String, Slot>>,
    scope: std::cell::Cell<usize>,
    next_scope: std::cell::Cell<usize>,
    next: std::cell::Cell<usize>,
    undo: Arc<RwLock<Vec<Undo>>>,
}
impl Drop for Store {
    fn drop(&mut self) {
        // Removed tables retain the shared undo log. Break that ownership cycle
        // even if unwinding skips the session's explicit rollback.
        self.undo.write().clear();
    }
}
pub struct Scope<'a> {
    store: &'a Store,
    previous: usize,
}
impl Drop for Scope<'_> {
    fn drop(&mut self) {
        self.store.scope.set(self.previous);
        let _ = self.store.mount(self.previous);
    }
}
pub struct Transaction<'a> {
    store: &'a Store,
    db: &'a rusqlite::Connection,
    committed: bool,
}
impl Store {
    pub fn current_scope(&self) -> usize {
        self.scope.get()
    }
    pub fn scope_mark(&self) -> usize {
        self.next_scope.get()
    }
    pub fn discard_scopes_after(&self, db: &rusqlite::Connection, mark: usize) -> Result<()> {
        let keys = self
            .tables
            .borrow()
            .keys()
            .filter(|(s, _)| *s > mark)
            .cloned()
            .collect::<Vec<_>>();
        for key in keys {
            self.remove_key(db, &key)?;
        }
        Ok(())
    }
    pub fn discard_scope(&self, db: &rusqlite::Connection, scope: usize) -> Result<()> {
        let keys = self.tables.borrow().range((scope, String::new())..)
            .take_while(|((owner, _), _)| *owner == scope)
            .map(|(key, _)| key.clone()).collect::<Vec<_>>();
        for key in keys {
            self.remove_key(db, &key)?;
        }
        Ok(())
    }
    pub fn new_scope(&self) -> Result<usize> {
        let next = self
            .next_scope
            .get()
            .checked_add(1)
            .ok_or_else(|| error("native scope ids exhausted"))?;
        self.next_scope.set(next);
        Ok(next)
    }
    fn scope_tables(&self, scope: usize) -> Vec<(String, Arc<Mutable>)> {
        self.tables
            .borrow()
            .range((scope, String::new())..)
            .take_while(|((owner, _), _)| *owner == scope)
            .map(|((_, n), t)| (n.clone(), t.clone()))
            .collect()
    }
    fn mount(&self, scope: usize) -> Result<()> {
        for (name, table) in self.scope_tables(scope) {
            self.modules
                .get(&name)
                .ok_or_else(|| error("missing native SQL slot"))?
                .set(table)
                .map_err(error)?;
        }
        Ok(())
    }
    pub fn activate(&self, scope: usize) -> Result<Scope<'_>> {
        self.mount(scope)?;
        let previous = self.scope.replace(scope);
        Ok(Scope {
            store: self,
            previous,
        })
    }
    /// Fork roots without declaring SQL tables. All frames reuse the same SQL slots.
    pub fn fork_scope(&self, old: usize) -> Result<usize> {
        let new = self.new_scope()?;
        for (name, table) in self.scope_tables(old) {
            let version = table.state.read().as_ref().clone();
            let fork = self.allocate((new, name.clone()), version, table.sql_writable);
            self.tables.borrow_mut().insert((new, name.clone()), fork);
            self.slots
                .borrow_mut()
                .get_mut(&name)
                .unwrap()
                .owners
                .insert(new);
        }
        Ok(new)
    }
    fn allocate(&self, key: TableKey, version: Version, sql_writable: bool) -> Arc<Mutable> {
        let id = self.next.get();
        self.next.set(id + 1);
        for undo in self.undo.write().iter_mut() {
            undo.created.push((key.clone(), id));
        }
        Arc::new(Mutable {
            id,
            sql_writable,
            state: RwLock::new(Arc::new(version)),
            undo: self.undo.clone(),
        })
    }
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn columns_read(&self) -> u64 {
        self.slots
            .borrow()
            .keys()
            .filter_map(|n| self.modules.get(n))
            .map(|b| {
                b.counters()
                    .columns
                    .load(std::sync::atomic::Ordering::Relaxed)
            })
            .sum()
    }
    pub fn close(&self, db: &rusqlite::Connection) -> Result<()> {
        let names = self.slots.borrow().keys().cloned().collect::<Vec<_>>();
        for name in names {
            self.remove_all(db, &name)?;
        }
        Ok(())
    }
    pub fn transaction<'a>(&'a self, db: &'a rusqlite::Connection) -> Result<Transaction<'a>> {
        self.begin()?;
        Ok(Transaction {
            store: self,
            db,
            committed: false,
        })
    }
    pub fn begin(&self) -> Result<()> {
        let mut undo = self.undo.write();
        if !undo.is_empty() {
            return Err(error("nested native version transaction"));
        }
        undo.push(Undo::default());
        Ok(())
    }
    pub fn savepoint(&self) {
        self.undo.write().push(Undo::default());
    }
    pub fn commit(&self) {
        self.undo.write().pop();
    }
    /// Restore roots and binding cells, including after SQLite's automatic rollback.
    pub fn rollback(&self, db: &rusqlite::Connection) -> Result<()> {
        let Some(undo) = self.undo.write().pop() else {
            return Ok(());
        };
        for (key, id) in &undo.created {
            if self.tables.borrow().get(key).is_some_and(|t| t.id == *id) {
                self.remove_key(db, key)?;
            }
        }
        for (key, table) in undo.removed {
            let (scope, name) = &key;
            self.slots
                .borrow_mut()
                .entry(name.clone())
                .or_insert_with(|| Slot {
                    schema: table.state.read().schema.clone(),
                    sql_writable: table.sql_writable,
                    owners: BTreeSet::new(),
                })
                .owners
                .insert(*scope);
            self.tables.borrow_mut().insert(key, table);
        }
        for (name, binding) in undo.bindings {
            self.modules
                .restore_binding(db, &name, binding)
                .map_err(error)?;
        }
        for table in self.tables.borrow().values() {
            if let Some(version) = undo.versions.get(&table.id) {
                *table.state.write() = version.clone();
            }
        }
        self.mount(self.scope.get())?;
        Ok(())
    }
    pub fn contains(&self, name: &str) -> bool {
        self.tables
            .borrow()
            .contains_key(&(self.scope.get(), name.into()))
    }
    pub fn owns(&self, name: &str) -> bool {
        self.slots.borrow().contains_key(name)
    }
    pub fn version(&self, name: &str) -> Version {
        self.version_at(self.scope.get(), name)
    }
    pub fn version_at(&self, scope: usize, name: &str) -> Version {
        self.tables.borrow()[&(scope, name.into())]
            .state
            .read()
            .as_ref()
            .clone()
    }
    pub fn replace(&self, name: &str, version: Version) -> Result<()> {
        self.replace_at(self.scope.get(), name, version)
    }
    fn replace_at(&self, scope: usize, name: &str, version: Version) -> Result<()> {
        let tables = self.tables.borrow();
        let table = tables
            .get(&(scope, name.into()))
            .ok_or_else(|| error("missing versioned relation"))?;
        let mut state = table.state.write();
        if *state.schema != *version.schema {
            return Err(error("versioned replacement changed schema"));
        }
        for undo in self.undo.write().iter_mut() {
            undo.versions
                .entry(table.id)
                .or_insert_with(|| state.clone());
        }
        *state = Arc::new(version);
        Ok(())
    }
    pub fn remove(&self, db: &rusqlite::Connection, name: &str) -> Result<()> {
        self.remove_key(db, &(self.scope.get(), name.into()))
    }
    pub fn remove_all(&self, db: &rusqlite::Connection, name: &str) -> Result<()> {
        let scopes = self
            .slots
            .borrow()
            .get(name)
            .map(|s| s.owners.clone())
            .unwrap_or_default();
        for scope in scopes {
            self.remove_key(db, &(scope, name.into()))?;
        }
        Ok(())
    }
    fn remove_key(&self, db: &rusqlite::Connection, key: &TableKey) -> Result<()> {
        let Some(table) = self.tables.borrow().get(key).cloned() else {
            return Ok(());
        };
        let (scope, name) = key;
        let remaining = self.slots.borrow()[name].owners.iter().find(|s| *s != scope).copied();
        if let Some(remaining) = remaining {
            self.modules.get(name).unwrap()
                .set(self.tables.borrow()[&(remaining, name.clone())].clone())
                .map_err(error)?;
            self.slots.borrow_mut().get_mut(name).unwrap().owners.remove(scope);
        } else {
            for undo in self.undo.write().iter_mut() {
                let existing = !undo.created.iter().any(|(_, id)| *id == table.id)
                    || undo.removed.keys().any(|(_, n)| n == name);
                if existing {
                    if let Some(binding) = self.modules.get(name) {
                        undo.bindings
                            .entry(name.clone())
                            .or_insert_with(|| binding.clone());
                    }
                }
            }
            self.modules.unregister(db, name).map_err(error)?;
            self.slots.borrow_mut().remove(name);
        }
        self.tables.borrow_mut().remove(key);
        for undo in self.undo.write().iter_mut() {
            if !undo.created.iter().any(|(_, id)| *id == table.id) {
                undo.removed
                    .entry(key.clone())
                    .or_insert_with(|| table.clone());
            }
        }
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
        self.store.commit();
        self.committed = true;
    }
}
impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let _ = self.store.rollback(self.db);
    }
}

#[cfg(test)]
pub mod bulk_tests;
#[cfg(any(test, feature = "test-hooks"))]
pub mod operators;
#[cfg(test)]
pub mod tests;
