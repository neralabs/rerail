//! Apply keys and rows already evaluated by SQL. Validation precedes publication.
use super::*;

impl Version {
    pub(super) fn renamed(mut self, columns: Vec<String>) -> Result<Self> {
        if columns.len() != self.schema.columns.len()
            || columns.iter().collect::<BTreeSet<_>>().len() != columns.len()
        {
            return Err(error("invalid diagnostic alias columns"));
        }
        Arc::make_mut(&mut self.schema).columns = columns;
        Ok(self)
    }
    pub fn rows(&self) -> Vec<Vec<Option<i64>>> {
        let mut rows = vec![];
        Node::prefix(&self.full, &[], &mut rows);
        rows.into_iter().map(|r| r.values.to_vec()).collect()
    }
    pub fn patched(
        &self,
        keys: &Version,
        rows: Vec<Vec<Option<i64>>>,
        dense: bool,
    ) -> Result<Self> {
        if keys.schema.columns.len() != self.schema.keys {
            return Err(error("SQL phase patch requires the complete primary key"));
        }
        self.patched_rows(keys.rows(), rows, dense)
    }
    pub fn patched_rows(
        &self,
        keys: Vec<Vec<Option<i64>>>,
        rows: Vec<Vec<Option<i64>>>,
        dense: bool,
    ) -> Result<Self> {
        self.patched_rows_reusing(keys, rows, dense, &[])
            .map(|(version, _)| version)
    }
    pub fn patched_rows_reusing(
        &self,
        keys: Vec<Vec<Option<i64>>>,
        rows: Vec<Vec<Option<i64>>>,
        dense: bool,
        reusable: &[Self],
    ) -> Result<(Self, bool)> {
        if keys
            .iter()
            .any(|k| k.len() != self.schema.keys || k.iter().any(Option::is_none))
        {
            return Err(error("invalid SQL phase primary key"));
        }
        let selected = keys.into_iter().collect::<BTreeSet<_>>();
        if rows.iter().any(|r| {
            r.len() != self.schema.columns.len() || !selected.contains(&r[..self.schema.keys])
        }) {
            return Err(error("SQL phase row is outside its patch keys"));
        }
        if dense && selected.len() >= self.len().div_ceil(2) && self.len() >= 512 {
            let mut unchanged = self.rows();
            unchanged.retain(|row| !selected.contains(&row[..self.schema.keys]));
            unchanged.extend(rows);
            return self.loaded(unchanged, false, true).map(|v| (v, false));
        }
        // Empty destinations already have the exact validated replacement. Re-inserting
        // every incoming row would rebuild all persistent paths a second time.
        if self.len() == 0 {
            if let Some(next) = reusable
                .iter()
                .find_map(|v| self.reuse_empty_rows(&rows, v))
            {
                return Ok((next, true));
            }
            return self.loaded(rows, false, true).map(|v| (v, false));
        }
        // `loaded` checks arity, nullability, counts, and duplicate primary keys.
        // It also reuses the original row ID when a key receives a different value.
        let incoming = self.loaded(rows, false, true)?;
        let mut next = self.clone();
        for key in selected {
            if let Some(old) = Node::get(&next.primary, &key).cloned() {
                next.remove(&old);
            }
        }
        let mut rows = vec![];
        Node::prefix(&incoming.full, &[], &mut rows);
        for row in rows {
            next.insert(row);
        }
        next.next = incoming.next;
        Ok((next, false))
    }
}
impl Store {
    pub fn replace_available(&self, name: &str, version: Version) -> Result<()> {
        self.replace_at(
            if self.contains(name) {
                self.scope.get()
            } else {
                0
            },
            name,
            version,
        )
    }
}
