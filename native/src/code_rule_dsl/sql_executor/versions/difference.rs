//! Publish a version difference with shared immutable row cells and existing row IDs.
use super::*;

impl Version {
    /// Compare immutable indexes without decoding unchanged branches. Both results
    /// have positional, non-null complete-row keys, and retain source row IDs.
    pub fn difference_roots(&self, new: &Self) -> Result<(Self, Self, usize)> {
        if self.schema.columns.len() != new.schema.columns.len() {
            return Err(error("native difference arity"));
        }
        let empty = Self::empty(Arc::new(Schema::integers(
            self.schema.columns.len(),
            self.schema.columns.len(),
        )));
        if self.same(new) {
            return Ok((empty.clone(), empty, 0));
        }
        let (mut added, mut removed, mut visited) = (vec![], vec![], 0);
        Node::difference(
            &self.full,
            &new.full,
            &mut added,
            &mut removed,
            &mut visited,
        );
        Ok((
            empty.delta_subset(new, added)?,
            empty.delta_subset(self, removed)?,
            visited,
        ))
    }
    fn delta_subset(&self, source: &Self, rows: Vec<Row>) -> Result<Self> {
        if self.len() != 0
            || self.schema.keys != self.schema.columns.len()
            || self.schema.columns.len() != source.schema.columns.len()
            || source.schema.nullable.iter().any(|&v| v)
                && rows.iter().any(|r| r.values.iter().any(Option::is_none))
            || !self.schema.positive.is_empty()
        {
            return Err(error(
                "native difference requires an empty complete-key integer destination",
            ));
        }
        let mut next = self.empty_like();
        next.next = source.next;
        next.distinct[0] = rows
            .iter()
            .enumerate()
            .filter(|(i, r)| *i == 0 || rows[*i - 1].values[0] != r.values[0])
            .count();
        next.full = Node::bulk(
            rows.iter().map(|r| (r.values.clone(), r.clone())).collect(),
            true,
        );
        next.primary = next.full.clone();
        let mut ids = rows
            .into_iter()
            .map(|r| (key([Some(r.id)]), r))
            .collect::<Vec<_>>();
        ids.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        next.ids = Node::bulk(ids, false);
        let indexes = self
            .secondary
            .iter()
            .enumerate()
            .filter_map(|(c, &yes)| yes.then_some(c))
            .collect::<Vec<_>>();
        next.secondary = vec![false; self.schema.columns.len()].into();
        next.ensure_indexes(&indexes);
        Ok(next)
    }
}
impl Store {
    pub fn difference_scratch(
        &self,
        plus: &str,
        minus: &str,
        old: &Version,
        new: &Version,
    ) -> Result<(usize, usize, usize)> {
        if old.schema.columns.len() != new.schema.columns.len() {
            return Err(error("native difference arity"));
        }
        let (mut added, mut removed, mut visited) = (vec![], vec![], 0);
        Node::difference(&old.full, &new.full, &mut added, &mut removed, &mut visited);
        // Build both destinations before publication. The shared cells already passed
        // source validation; their digests and row IDs need no reconstruction.
        let positive = self.version_at(0, plus).delta_subset(new, added)?;
        let negative = self.version_at(0, minus).delta_subset(old, removed)?;
        let counts = (positive.len(), negative.len(), visited);
        self.replace_at(0, plus, positive)?;
        self.replace_at(0, minus, negative)?;
        Ok(counts)
    }
}
