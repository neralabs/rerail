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
        let mut next = self.clone();
        // Validate and assign IDs without building a second set of temporary
        // indexes. All mutations remain local until the entire phase succeeds.
        let mut incoming = BTreeMap::new();
        for values in rows {
            if !self.schema.valid_row(&values) {
                return Err(error("invalid bulk integer row"));
            }
            let pk = values[..self.schema.keys].to_vec();
            let entry = incoming.entry(pk);
            if matches!(entry, std::collections::btree_map::Entry::Occupied(_)) {
                return Err(error("duplicate bulk key"));
            }
            let old = Node::get(&self.primary, entry.key());
            let row = if let Some(old) = old.filter(|old| old.values.as_slice() == values) {
                old.clone()
            } else {
                let id = if let Some(old) = old {
                    old.id
                } else {
                    let id = next.next;
                    next.next = next
                        .next
                        .checked_add(1)
                        .ok_or_else(|| error("versioned row ids exhausted"))?;
                    id
                };
                Row {
                    id,
                    values: tree::key(values),
                }
            };
            entry.or_insert(row);
        }
        for key in selected {
            let old = Node::get(&self.primary, &key);
            match (old, incoming.remove(&key)) {
                (Some(old), Some(row)) if old.values == row.values => {}
                (Some(old), Some(row)) => next.replace_row(old, row),
                (Some(old), None) => next.remove(old),
                (None, Some(row)) => next.insert(row),
                (None, None) => {}
            }
        }
        Ok((next, false))
    }

    /// Replace a value at the same primary key and row ID. Index entries whose
    /// keys stay equal need one value update rather than deletion and insertion.
    fn replace_row(&mut self, old: &Row, row: Row) {
        debug_assert_eq!(old.id, row.id);
        debug_assert_eq!(
            &old.values[..self.schema.keys],
            &row.values[..self.schema.keys]
        );
        self.full = Node::set(&self.full, old.values.clone(), None);
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
        // The leading value is part of the unchanged primary key, so its
        // distinct count is unchanged even while the complete-row key changes.
        for (c, &value) in row.values.iter().enumerate().skip(1) {
            if !self.secondary[c] {
                continue;
            }
            if old.values[c] != value {
                self.indexes[c] =
                    Node::set_index(&self.indexes[c], key([old.values[c], Some(row.id)]), None);
                if !Node::has_prefix(&self.indexes[c], &[old.values[c]]) {
                    self.distinct[c] -= 1;
                }
                if !Node::has_prefix(&self.indexes[c], &[value]) {
                    self.distinct[c] += 1;
                }
            }
            self.indexes[c] = Node::set_index(
                &self.indexes[c],
                key([value, Some(row.id)]),
                Some(row.clone()),
            );
        }
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
