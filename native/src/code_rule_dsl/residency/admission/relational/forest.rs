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

pub(in crate::code_rule_dsl) fn materialize(
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
            "SELECT root,expand FROM {} ORDER BY root",
            name("requests")
        ))
        .map_err(sql_error)?;
    let roots = request_query
        .query_map([], |r| {
            Ok((r.get::<_, u32>(0)? as usize, r.get::<_, bool>(1)?))
        })
        .map_err(sql_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql_error)?;
    // The selected graph defines these bounds. Raw CST ranges cannot replace a forest
    // whose selected edges omit an interior node or disconnect a subtree.
    db.execute_batch(&format!("DROP TABLE {}; CREATE TEMP TABLE {}(ordinal INTEGER PRIMARY KEY,node INTEGER UNIQUE); CREATE TEMP TABLE {}(root INTEGER PRIMARY KEY,lo INTEGER,hi INTEGER)",name("pairs"),name("positions"),name("ranges"))).map_err(sql_error)?;
    insert::stage_rows(
        db,
        &format!("{prefix}_positions"),
        2,
        forest
            .preorder
            .iter()
            .enumerate()
            .map(|(pre, &node)| [Value::Integer(pre as i64), Value::Integer(node as i64 + 1)]),
        work,
    )?;
    insert::stage_rows(
        db,
        &format!("{prefix}_ranges"),
        3,
        roots.iter().map(|&(root, expand)| {
            let (lo, hi) = forest.ranges[root - 1];
            [
                Value::Integer(root as i64),
                Value::Integer(lo as i64),
                Value::Integer(if expand { hi } else { lo + 1 } as i64),
            ]
        }),
        work,
    )?;
    steps.spend(forest.preorder.len() + roots.len())?;
    db.execute_batch(&format!("CREATE INDEX {} ON {}(lo,hi,root); ANALYZE temp.{}; ANALYZE temp.{}; CREATE TEMP VIEW {} AS SELECT r.root,p.node FROM {} r JOIN {} p ON p.ordinal>=r.lo AND p.ordinal<r.hi",name("range_bounds"),name("ranges"),name("positions"),name("ranges"),name("pairs"),name("ranges"),name("positions"))).map_err(sql_error)?;
    Ok((roots.len(), roots.len().div_ceil(batch_size)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn containment_retains_linear_sql_storage_for_all_roots() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TEMP TABLE f_keys(compact INTEGER PRIMARY KEY); CREATE TEMP TABLE f_compact(src INTEGER,dst INTEGER); CREATE TEMP TABLE f_requests(root INTEGER PRIMARY KEY,expand INTEGER); CREATE TEMP TABLE f_pairs(root INTEGER,node INTEGER); WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<1000) INSERT INTO f_keys SELECT i FROM n; INSERT INTO f_requests SELECT compact,1 FROM f_keys; INSERT INTO f_compact SELECT compact,compact+1 FROM f_keys WHERE compact<1000;").unwrap();
        let work = Arc::new(AtomicUsize::new(100_000));
        materialize(&db, "f", 256, &work).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT type FROM sqlite_temp_master WHERE name='f_pairs'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "view"
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM f_positions", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1000
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM f_ranges", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1000
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM f_pairs", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            500500
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM f_pairs WHERE root=500", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            501
        );
    }
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
