//! Indexed, connection-local facts. Updates require an idle connection, so cursors see stable rows.
use super::*;
use parking_lot::RwLock;

#[derive(Default)]
struct Rows {
    values: Vec<Vec<i64>>,
    positions: BTreeMap<Vec<i64>, usize>,
    indexes: Vec<BTreeMap<i64, BTreeSet<usize>>>,
}
pub(super) struct Live {
    columns: Vec<String>,
    rows: RwLock<Rows>,
}
impl Rows {
    fn insert(&mut self, row: Vec<i64>) {
        if self.positions.contains_key(&row) {
            return;
        }
        let i = self.values.len();
        for (c, v) in row.iter().enumerate() {
            self.indexes[c].entry(*v).or_default().insert(i);
        }
        self.positions.insert(row.clone(), i);
        self.values.push(row);
    }
    fn delete(&mut self, row: &[i64]) {
        let Some(i) = self.positions.remove(row) else {
            return;
        };
        let last = self.values.len() - 1;
        for (c, v) in row.iter().enumerate() {
            let index = &mut self.indexes[c];
            let group = index.get_mut(v).unwrap();
            group.remove(&i);
            if group.is_empty() {
                index.remove(v);
            }
        }
        self.values.swap_remove(i);
        if i != last {
            let moved = &self.values[i];
            *self.positions.get_mut(moved).unwrap() = i;
            for (c, v) in moved.iter().enumerate() {
                let group = self.indexes[c].get_mut(v).unwrap();
                group.remove(&last);
                group.insert(i);
            }
        }
    }
}
impl Live {
    pub fn from_source(source: &dyn Source) -> Self {
        let columns = source.columns();
        let mut rows = Rows {
            indexes: vec![BTreeMap::new(); columns.len()],
            ..Rows::default()
        };
        for i in 0..source.len() {
            rows.insert((0..columns.len()).map(|c| source.value(i, c)).collect());
        }
        Self {
            columns,
            rows: RwLock::new(rows),
        }
    }
    pub fn changes(
        &self,
        insert: &BTreeSet<Vec<i64>>,
        delete: &BTreeSet<Vec<i64>>,
    ) -> (Vec<Vec<i64>>, Vec<Vec<i64>>) {
        let rows = self.rows.read();
        (
            insert
                .iter()
                .filter(|r| !rows.positions.contains_key(*r))
                .cloned()
                .collect(),
            delete
                .iter()
                .filter(|r| !insert.contains(*r) && rows.positions.contains_key(*r))
                .cloned()
                .collect(),
        )
    }
    pub fn apply(&self, insert: &[Vec<i64>], delete: &[Vec<i64>]) {
        let mut rows = self.rows.write();
        for r in delete {
            rows.delete(r);
        }
        for r in insert {
            rows.insert(r.clone());
        }
    }
}
impl Source for Live {
    fn columns(&self) -> Vec<String> {
        self.columns.clone()
    }
    fn len(&self) -> usize {
        self.rows.read().values.len()
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        let r = self.rows.read();
        let n = r.values.len() / r.indexes[column].len().max(1);
        if batch {
            n.saturating_mul(8).max(1)
        } else {
            n.max(1)
        }
    }
    fn indexed(&self, _: usize) -> bool {
        true
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        let r = self.rows.read();
        Some(
            values
                .iter()
                .filter_map(|v| r.indexes[column].get(v))
                .flatten()
                .copied()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
        )
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        self.rows.read().values[row][column]
    }
}
