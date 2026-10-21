//! Strict forest indexing over SQL-normalized identities. No detector decisions live here.
use super::{Connection, Result, insert, source, sql_error};
use crate::code_rule_dsl::json::invalid;
use rusqlite::types::Value;
use std::sync::{Arc, atomic::AtomicUsize};

/// One relational execution unit is a bounded batch of 64 VM or native steps.
/// Reserve units before work, retaining the unused part across short loops.
struct Steps<'a> {
    work: &'a AtomicUsize,
    prepaid: usize,
}
impl<'a> Steps<'a> {
    fn new(work: &'a AtomicUsize) -> Self {
        Self { work, prepaid: 0 }
    }
    fn spend(&mut self, count: usize) -> Result<()> {
        if count <= self.prepaid {
            self.prepaid -= count;
            return Ok(());
        }
        let needed = count - self.prepaid;
        let units = needed.div_ceil(64);
        source::charge(self.work, units)?;
        self.prepaid = units * 64 - needed;
        Ok(())
    }
}

#[derive(Debug)]
struct Forest {
    preorder: Vec<usize>,
    ranges: Vec<(usize, usize)>,
}
impl Forest {
    fn build(
        count: usize,
        edges: impl IntoIterator<Item = (usize, usize)>,
        work: &AtomicUsize,
    ) -> Result<Self> {
        let mut steps = Steps::new(work);
        steps.spend(count)?;
        let mut parents = vec![None; count];
        let mut children = vec![Vec::new(); count];
        for (parent, child) in edges {
            steps.spend(1)?;
            if parent >= count || child >= count {
                return Err(invalid("forest identity is out of bounds"));
            }
            if let Some(previous) = parents[child] {
                if previous != parent {
                    return Err(invalid("forest node has multiple parents"));
                }
            } else {
                parents[child] = Some(parent);
                children[parent].push(child);
            }
        }
        let mut preorder = Vec::with_capacity(count);
        let mut ranges = vec![(0, 0); count];
        let mut pending = parents
            .iter()
            .enumerate()
            .filter(|(_, p)| p.is_none())
            .map(|(i, _)| (i, false))
            .collect::<Vec<_>>();
        while let Some((node, exiting)) = pending.pop() {
            steps.spend(1)?;
            if exiting {
                ranges[node].1 = preorder.len();
            } else {
                ranges[node].0 = preorder.len();
                preorder.push(node);
                pending.push((node, true));
                pending.extend(children[node].iter().map(|&child| (child, false)));
            }
        }
        if preorder.len() != count {
            return Err(invalid("forest contains a cycle"));
        }
        Ok(Self { preorder, ranges })
    }
}

pub(super) fn materialize(
    db: &Connection,
    prefix: &str,
    batch_size: usize,
    work: &Arc<AtomicUsize>,
) -> Result<(usize, usize)> {
    let name = |suffix: &str| super::quote(&format!("{prefix}_{suffix}"));
    let mut steps = Steps::new(work);
    let count = db
        .query_row(&format!("SELECT count(*) FROM {}", name("keys")), [], |r| {
            r.get::<_, u32>(0).map(|v| v as usize)
        })
        .map_err(sql_error)?;
    let mut edges_query = db
        .prepare(&format!("SELECT src,dst FROM {}", name("compact")))
        .map_err(sql_error)?;
    let mut rows = edges_query.query([]).map_err(sql_error)?;
    let mut edges = Vec::new();
    while let Some(row) = rows.next().map_err(sql_error)? {
        steps.spend(1)?;
        edges.push((
            row.get::<_, u32>(0)
                .map(|v| v as usize)
                .map_err(sql_error)?
                - 1,
            row.get::<_, u32>(1)
                .map(|v| v as usize)
                .map_err(sql_error)?
                - 1,
        ));
    }
    let forest = Forest::build(count, edges, work)?;
    let mut request_query = db
        .prepare(&format!(
            "SELECT root FROM {} ORDER BY root",
            name("requests")
        ))
        .map_err(sql_error)?;
    let roots = request_query
        .query_map([], |r| r.get::<_, u32>(0).map(|v| v as usize))
        .map_err(sql_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql_error)?;
    for batch in roots.chunks(batch_size) {
        let mut output = Vec::new();
        for &root in batch {
            steps.spend(1)?;
            let (start, end) = forest.ranges[root - 1];
            // Bound the native copy and sort work, separately from SQL INSERT VM work.
            let length = end - start;
            steps.spend(length.saturating_mul(length.max(1).ilog2() as usize + 2))?;
            let mut nodes = forest.preorder[start..end].to_vec();
            nodes.sort_unstable();
            for node in nodes {
                output.push([
                    Value::Integer(root as i64),
                    Value::Integer((node + 1) as i64),
                ]);
                if output.len() == 256 {
                    insert::stage_rows(db, &format!("{prefix}_pairs"), 2, output.drain(..), work)?;
                }
            }
        }
        if !output.is_empty() {
            insert::stage_rows(db, &format!("{prefix}_pairs"), 2, output, work)?;
        }
    }
    Ok((roots.len(), roots.len().div_ceil(batch_size)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deep_forests_and_interruptions_are_bounded() {
        let work = AtomicUsize::new(1_000_000);
        let forest = Forest::build(30_000, (1..30_000).map(|i| (i - 1, i)), &work).unwrap();
        assert_eq!(forest.ranges[0], (0, 30_000));
        assert_eq!(forest.preorder[forest.ranges[29_999].0], 29_999);
        assert!(
            Forest::build(2, [(0, 1)], &AtomicUsize::new(0))
                .unwrap_err()
                .reason
                .contains("interrupted")
        );
        assert!(
            Forest::build(2, [(0, 1), (1, 0)], &work)
                .unwrap_err()
                .reason
                .contains("cycle")
        );
        assert!(
            Forest::build(3, [(0, 2), (1, 2)], &work)
                .unwrap_err()
                .reason
                .contains("multiple parents")
        );
    }
}
