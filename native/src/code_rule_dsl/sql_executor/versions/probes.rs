//! Build only requested secondary access paths. Leading prefixes use the complete-row tree.
use super::*;

impl Version {
    /// Descend only into existing prefixes; do not enumerate the IN lists' Cartesian product.
    pub fn primary_batch(&self, values: &[Vec<i64>]) -> Vec<usize> {
        assert_eq!(values.len(), self.schema.keys);
        if values.iter().any(Vec::is_empty) {
            return Vec::new();
        }
        fn descend(
            version: &Version,
            values: &[Vec<i64>],
            prefix: &mut Vec<Option<i64>>,
            range: std::ops::Range<usize>,
            found: &mut Vec<usize>,
        ) {
            let level = prefix.len();
            if level == values.len()
                || range.len() <= 16
                || values[level].len() >= range.len().div_ceil(2)
            {
                for at in range {
                    crate::code_rule_dsl::sql_profile::frame(
                        crate::code_rule_dsl::sql_profile::FrameEvent::Next,
                        1,
                    );
                    let row = Node::nth(&version.full, at);
                    if values.iter().enumerate().all(|(c, allowed)| {
                        row.values[c].is_some_and(|v| allowed.binary_search(&v).is_ok())
                    }) {
                        found.push(at);
                    }
                }
                return;
            }
            for &value in &values[level] {
                prefix.push(Some(value));
                crate::code_rule_dsl::sql_profile::frame(
                    crate::code_rule_dsl::sql_profile::FrameEvent::Lookup,
                    1,
                );
                let sub = Node::prefix_ranks(&version.full, prefix);
                if !sub.is_empty() {
                    descend(version, values, prefix, sub, found);
                }
                prefix.pop();
            }
        }
        let mut found = Vec::new();
        descend(self, values, &mut Vec::new(), 0..self.len(), &mut found);
        found
    }
    pub fn ensure_indexes(&mut self, columns: &[usize]) {
        for &c in columns {
            if c == 0 || self.secondary[c] {
                continue;
            }
            let _cpu = super::super::super::logic::profile::span(|| {
                "native secondary index construction".into()
            });
            super::super::super::logic::profile::work(
                super::super::super::logic::profile::Unit::Value,
                self.len() as u64,
            );
            let _index = crate::code_rule_dsl::sql_profile::index_build(self.len());
            let mut rows = Vec::new();
            Node::prefix(&self.full, &[], &mut rows);
            let mut entries = rows
                .into_iter()
                .map(|r| (key([r.values[c], Some(r.id)]), r))
                .collect::<Vec<_>>();
            entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
            self.distinct[c] = entries
                .iter()
                .enumerate()
                .filter(|(i, e)| *i == 0 || entries[i - 1].0[0] != e.0[0])
                .count();
            self.indexes[c] = Node::bulk(entries, false);
            Arc::make_mut(&mut self.secondary)[c] = true;
        }
    }
    pub fn restrict_indexes(&mut self, columns: &[usize]) {
        self.ensure_indexes(columns);
        let desired = columns
            .iter()
            .copied()
            .filter(|&c| c > 0)
            .collect::<BTreeSet<_>>();
        for c in 1..self.indexes.len() {
            if !desired.contains(&c) {
                self.indexes[c] = None;
                self.distinct[c] = 0;
                Arc::make_mut(&mut self.secondary)[c] = false;
            }
        }
    }
}
impl Store {
    pub fn restrict_indexes(&self, name: &str, columns: &[usize]) -> Result<()> {
        let mut version = self.version(name);
        version.restrict_indexes(columns);
        self.replace(name, version)
    }
}
