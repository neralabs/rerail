//! Bulk storage construction after SQL has evaluated a relation. No rule evaluation occurs here.
use super::*;
pub(in crate::code_rule_dsl::sql_executor) struct Reuse<'a> {
    store: &'a Store,
    previous: Vec<(TableKey, Option<Version>)>,
}
impl Drop for Reuse<'_> {
    fn drop(&mut self) {
        let mut map = self.store.reuse.borrow_mut();
        for (key, old) in self.previous.drain(..) {
            if let Some(v) = old {
                map.insert(key, v);
            } else {
                map.remove(&key);
            }
        }
    }
}

impl Version {
    /// Reuse a validated output of this SQL phase when its ordered rows and new row IDs match.
    /// A complete-tuple primary key can also read a narrower source's complete-row index.
    pub fn reuse_empty_rows(&self, rows: &[Vec<Option<i64>>], source: &Self) -> Option<Self> {
        if self.len() != 0
            || self.next != 1
            || rows.len() != source.len()
            || source.next != rows.len() as i64 + 1
            || self.schema.columns.len() != source.schema.columns.len()
            || self.schema.nullable != source.schema.nullable
            || self.schema.positive != source.schema.positive
            || self.schema.keys != source.schema.keys
                && self.schema.keys != self.schema.columns.len()
        {
            return None;
        }
        fn matches(root: &Link<Row>, rows: &[Vec<Option<i64>>], at: &mut usize) -> bool {
            let Some(n) = root else { return true };
            if !matches(&n.left, rows, at)
                || n.value.id != *at as i64 + 1
                || n.value.values.as_slice() != rows[*at]
            {
                return false;
            }
            *at += 1;
            matches(&n.right, rows, at)
        }
        if !matches(&source.full, rows, &mut 0) {
            return None;
        }
        let mut next = source.clone();
        next.schema = self.schema.clone();
        next.lazy = self.lazy;
        if self.schema.keys == self.schema.columns.len() {
            next.primary = next.full.clone();
        }
        next.restrict_indexes(
            &self
                .secondary
                .iter()
                .enumerate()
                .filter_map(|(c, &yes)| yes.then_some(c))
                .collect::<Vec<_>>(),
        );
        Some(next)
    }
    pub(in crate::code_rule_dsl::sql_executor) fn loaded(
        &self,
        rows: Vec<Vec<Option<i64>>>,
        ignore: bool,
        replacing: bool,
    ) -> Result<Self> {
        if self.len() != 0 && !replacing {
            return Err(error("bulk construction requires an empty relation"));
        }
        // SQL commonly emits the previous canonical row order unchanged. Compare
        // complete values before allocating keys, row-id indexes, or bulk trees.
        fn unchanged(root: &Link<Row>, rows: &[Vec<Option<i64>>], at: &mut usize) -> bool {
            let Some(node) = root else {
                return true;
            };
            if !unchanged(&node.left, rows, at) || node.value.values.as_slice() != rows[*at] {
                return false;
            }
            *at += 1;
            unchanged(&node.right, rows, at)
        }
        if replacing && rows.len() == self.len() && unchanged(&self.full, &rows, &mut 0) {
            return Ok(self.clone());
        }
        let mut out = self.empty_like();
        out.next = self.next;
        let mut primary = BTreeMap::new();
        let mut ids = Vec::new();
        for values in rows {
            if values.len() != self.schema.columns.len() {
                return Err(error("bulk row arity"));
            }
            let valid = values.iter().enumerate().all(|(c, v)| {
                (v.is_some() || self.schema.nullable[c])
                    && self.schema.positive.get(&c).is_none_or(|&strict| {
                        v.is_some_and(|v| if strict { v > 0 } else { v >= 0 })
                    })
            });
            if !valid {
                if ignore {
                    continue;
                }
                return Err(error("invalid bulk integer row"));
            }
            let entry = primary.entry(values[..self.schema.keys].to_vec());
            if matches!(entry, std::collections::btree_map::Entry::Occupied(_)) {
                if ignore {
                    continue;
                }
                return Err(error("duplicate bulk key"));
            }
            let old_row = Node::get(&self.primary, entry.key());
            let id = if let Some(old) = old_row {
                old.id
            } else {
                let id = out.next;
                out.next = out
                    .next
                    .checked_add(1)
                    .ok_or_else(|| error("versioned row ids exhausted"))?;
                id
            };
            let values = old_row
                .filter(|old| old.values.as_slice() == values)
                .map_or_else(|| tree::key(values), |old| old.values.clone());
            let row = Row { id, values };
            ids.push((key([Some(row.id)]), row.clone()));
            entry.or_insert(row);
        }
        let full = primary
            .values()
            .map(|r| (r.values.clone(), r.clone()))
            .collect::<Vec<_>>();
        // Distinct primary prefixes already order complete tuples. No second sort
        // is needed, including when the primary key is shorter than the row.
        out.distinct[0] = full
            .iter()
            .enumerate()
            .filter(|(i, r)| *i == 0 || full[*i - 1].0[0] != r.0[0])
            .count();
        let same = |a: &Row, b: &Row| {
            a.id == b.id && (Arc::ptr_eq(&a.values, &b.values) || a.values == b.values)
        };
        out.full = Node::bulk_reuse(full, true, &self.full, &same);
        out.primary = if self.schema.keys == self.schema.columns.len() {
            out.full.clone()
        } else {
            Node::bulk_reuse(
                primary
                    .into_iter()
                    .map(|(pk, row)| (tree::key(pk), row))
                    .collect(),
                false,
                &self.primary,
                &same,
            )
        };
        ids.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        out.ids = Node::bulk_reuse(ids, false, &self.ids, &same);
        let columns = self
            .secondary
            .iter()
            .enumerate()
            .filter_map(|(c, &yes)| yes.then_some(c))
            .collect::<Vec<_>>();
        out.secondary = vec![false; self.schema.columns.len()].into();
        out.ensure_indexes(&columns);
        Ok(out)
    }
}
impl Store {
    pub fn reuse_owned<'a>(&'a self, names: &[(String, bool)]) -> Reuse<'a> {
        let previous = names
            .iter()
            .map(|(name, _)| {
                let key = (self.scope.get(), name.clone());
                let root = self.version(name);
                let previous = self.reuse.borrow_mut().insert(key.clone(), root);
                (key, previous)
            })
            .collect();
        Reuse {
            store: self,
            previous,
        }
    }
    pub fn declare_empty(
        &self,
        db: &rusqlite::Connection,
        name: &str,
        schema: Schema,
    ) -> Result<()> {
        if self.contains(name) {
            let old = self.version(name);
            if *old.schema != schema {
                return Err(error("retained slot changed schema"));
            }
            self.replace(name, old.empty_like())
        } else {
            self.create(db, name, schema)
        }
    }
    pub fn clone_scratch(&self, name: &str, source: &Version) -> Result<bool> {
        let old = self.version_at(0, name);
        if source.schema.nullable.iter().any(|&v| v)
            || old.schema.keys != source.schema.columns.len()
            || old.schema.columns.len() != source.schema.columns.len()
        {
            return Ok(false);
        }
        let mut next = old.empty_like();
        next.secondary = vec![false; old.schema.columns.len()].into();
        next.full = source.full.clone();
        next.primary = source.full.clone();
        next.ids = source.ids.clone();
        next.next = source.next;
        next.distinct[0] = source.distinct[0];
        self.replace_at(0, name, next)?;
        Ok(true)
    }
    pub fn available(&self, name: &str) -> Option<Version> {
        self.tables
            .borrow()
            .get(&(self.scope.get(), name.into()))
            .map(|t| t.state.read().as_ref().clone())
            .or_else(|| {
                self.tables
                    .borrow()
                    .get(&(0, name.into()))
                    .map(|t| t.state.read().as_ref().clone())
            })
    }
    pub fn compose(&self, plus: &str, minus: &str, in_plus: &str, in_minus: &str) -> Result<bool> {
        let (Some(add), Some(remove)) = (self.available(in_plus), self.available(in_minus)) else {
            return Ok(false);
        };
        let (mut positive, mut negative) = (self.version(plus), self.version(minus));
        for adding in [true, false] {
            let (input, keep, cancel) = if adding {
                (&add, &mut positive, &mut negative)
            } else {
                (&remove, &mut negative, &mut positive)
            };
            let mut incoming = Vec::new();
            Node::prefix(&input.full, &[], &mut incoming);
            for row in incoming {
                if let Some(old) = Node::get(&cancel.full, &row.values).cloned() {
                    cancel.remove(&old);
                } else if Node::get(&keep.full, &row.values).is_none() {
                    let id = keep.next;
                    keep.next = keep
                        .next
                        .checked_add(1)
                        .ok_or_else(|| error("versioned row ids exhausted"))?;
                    keep.insert(Row {
                        id,
                        values: row.values,
                    });
                }
            }
        }
        self.replace(plus, positive)?;
        self.replace(minus, negative)?;
        Ok(true)
    }
    pub fn load_rows(
        &self,
        name: &str,
        rows: Vec<Vec<Option<i64>>>,
        ignore: bool,
    ) -> Result<usize> {
        let _cpu = super::super::super::logic::profile::span(|| "native bulk roots".into());
        super::super::super::logic::profile::work(
            super::super::super::logic::profile::Unit::Value,
            rows.len() as u64,
        );
        let version = self.version(name).loaded(rows, ignore, false)?;
        let count = version.len();
        self.replace(name, version)?;
        Ok(count)
    }
    pub fn load_select(
        &self,
        db: &rusqlite::Connection,
        name: &str,
        select: &str,
        ignore: bool,
    ) -> Result<usize> {
        self.select_at(db, self.scope.get(), name, select, ignore, false)
    }
    pub fn select_available(
        &self,
        db: &rusqlite::Connection,
        name: &str,
        select: &str,
        ignore: bool,
        replacing: bool,
    ) -> Result<usize> {
        self.select_at(
            db,
            if self.contains(name) {
                self.scope.get()
            } else {
                0
            },
            name,
            select,
            ignore,
            replacing,
        )
    }
    fn select_at(
        &self,
        db: &rusqlite::Connection,
        scope: usize,
        name: &str,
        select: &str,
        ignore: bool,
        replacing: bool,
    ) -> Result<usize> {
        let old = self.version_at(scope, name);
        let seed = self
            .reuse
            .borrow()
            .get(&(scope, name.into()))
            .filter(|_| old.len() == 0)
            .cloned();
        let replacing = replacing || seed.is_some();
        let mut seed = seed.unwrap_or_else(|| old.clone());
        seed.secondary = old.secondary.clone();
        seed.lazy = old.lazy;
        let mut statement = db.prepare(select).map_err(error)?;
        let width = statement.column_count();
        let rows = statement
            .query_map([], |row| {
                (0..width)
                    .map(|c| row.get(c))
                    .collect::<rusqlite::Result<Vec<Option<i64>>>>()
            })
            .map_err(error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(error)?;
        drop(statement);
        let _cpu = super::super::super::logic::profile::span(|| "native bulk roots".into());
        super::super::super::logic::profile::work(
            super::super::super::logic::profile::Unit::Value,
            rows.len() as u64,
        );
        let version = seed.loaded(rows, ignore, replacing)?;
        let count = version.len();
        self.replace_at(scope, name, version)?;
        Ok(count)
    }
    /// Scratch bindings belong to scope zero and remain mounted across frame changes.
    pub fn clear_scratch(&self, name: &str) -> Result<()> {
        let old = self.version_at(0, name);
        if old.len() > 0 {
            self.replace_at(0, name, old.empty_like())?;
        }
        Ok(())
    }
    pub fn scratch_len(&self, name: &str) -> usize {
        self.version_at(0, name).len()
    }
    pub fn load_scratch(&self, name: &str, rows: Vec<Vec<Option<i64>>>) -> Result<usize> {
        let version = self.version_at(0, name).loaded(rows, true, false)?;
        let n = version.len();
        self.replace_at(0, name, version)?;
        Ok(n)
    }
    pub fn create_scratch_access(
        &self,
        db: &rusqlite::Connection,
        name: &str,
        schema: Schema,
        writable: bool,
    ) -> Result<()> {
        struct Restore<'a>(&'a std::cell::Cell<usize>, usize);
        impl Drop for Restore<'_> {
            fn drop(&mut self) {
                self.0.set(self.1);
            }
        }
        let _restore = Restore(&self.scope, self.scope.replace(0));
        if writable {
            self.create(db, name, schema)?;
        } else {
            self.create_readonly(db, name, schema)?;
        }
        self.restrict_indexes(name, &[])?;
        let mut version = self.version(name);
        version.lazy = true;
        self.replace(name, version)
    }
}
