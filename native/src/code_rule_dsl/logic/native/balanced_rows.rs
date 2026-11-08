//! Greedy size-balanced partitions over a whole batch of independent tasks.
//! Keys define stable tie order; occurrence IDs keep equal keys distinct.
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashSet};

pub(in crate::code_rule_dsl) struct Row {
    pub task: i64,
    pub occurrence: i64,
    pub key: String,
    pub weight: i64,
}

pub(in crate::code_rule_dsl) fn partition(rows: Vec<Row>, limits: &[(i64,i64)]) -> Result<Vec<(i64,i64,i64,i64)>,String> {
    let mut requested = BTreeMap::new();
    for &(task,count) in limits {
        if count <= 0 { return Err("balanced_rows: bin counts must be positive".into()); }
        if requested.insert(task,count).is_some_and(|old| old != count) {
            return Err("balanced_rows: each task must have one bin count".into());
        }
    }
    let mut groups: BTreeMap<i64,Vec<Row>> = BTreeMap::new();
    for row in rows {
        if requested.contains_key(&row.task) { groups.entry(row.task).or_default().push(row); }
    }
    let mut result = Vec::new();
    for (task,mut rows) in groups {
        let mut seen = HashSet::new();
        for row in &rows {
            if row.weight < 0 { return Err("balanced_rows: weights must be nonnegative".into()); }
            if !seen.insert(row.occurrence) { return Err("balanced_rows: occurrence IDs must be unique within each task".into()); }
        }
        rows.sort_by(|a,b| b.weight.cmp(&a.weight).then_with(|| a.key.cmp(&b.key)).then_with(|| a.occurrence.cmp(&b.occurrence)));
        let count = (requested[&task] as u128).min(rows.len() as u128) as usize;
        let mut bins = vec![Vec::new();count];
        let mut loads = BinaryHeap::from((0..count).map(|i| Reverse((0u128,i))).collect::<Vec<_>>());
        for row in rows {
            let Reverse((weight,bin)) = loads.pop().expect("a nonempty task has bins");
            bins[bin].push(row.occurrence);
            loads.push(Reverse((weight + row.weight as u128,bin)));
        }
        let mut ordinal = 0;
        for (bin,occurrences) in bins.into_iter().enumerate() {
            for occurrence in occurrences {
                result.push((task,occurrence,bin as i64,ordinal));
                ordinal += 1;
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_partition_demands_do_not_silently_change_the_work() {
        let row = |id,weight| Row { task: 1, occurrence: id, key: "same".into(), weight };
        assert!(partition(vec![], &[(1,0)]).unwrap_err().contains("positive"));
        assert!(partition(vec![], &[(1,1),(1,2)]).unwrap_err().contains("one bin count"));
        assert!(partition(vec![row(0,-1)], &[(1,2)]).unwrap_err().contains("nonnegative"));
        assert!(partition(vec![row(0,1),row(0,2)], &[(1,2)]).unwrap_err().contains("unique"));
        assert!(partition(vec![row(0,-1)], &[]).unwrap().is_empty());
    }
}
