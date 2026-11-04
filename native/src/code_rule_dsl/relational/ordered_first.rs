//! Ordered, memoized first-match traversal of a whole typed graph batch.
//! SQL owns candidate scope, edge order, and sink selection. This producer has no rule names.
use super::{Result, Stage, invalid, source, sql_error, syntax, table};
use indexmap::IndexMap;
use rusqlite::{types::Value, vtab::Context};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, atomic::AtomicUsize},
};

#[derive(Clone, Debug)]
pub(in crate::code_rule_dsl) struct Spec {
    pub sinks: String,
    pub on_edge: bool,
    pub memo: Option<String>,
    pub visiting: Option<String>,
    pub emit_memo: bool,
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct Found {
    sink: Option<usize>,
    location: i64,
}
#[derive(Default)]
struct Graph {
    keys: Vec<String>,
    ids: HashMap<String, usize>,
    roots: Vec<usize>,
    requested: HashSet<usize>,
    edges: Vec<Vec<(usize, i64)>>,
    sinks: HashMap<usize, i64>,
    blocked: HashSet<usize>,
    initial: IndexMap<usize, Option<Found>>,
}
impl Graph {
    fn intern(&mut self, key: String) -> usize {
        if let Some(id) = self.ids.get(&key) {
            return *id;
        }
        let id = self.keys.len();
        self.keys.push(key.clone());
        self.ids.insert(key, id);
        self.edges.push(Vec::new());
        id
    }
    fn first(
        &self,
        root: usize,
        on_edge: bool,
        memo: &mut IndexMap<usize, Option<Found>>,
        work: &AtomicUsize,
    ) -> Result<Option<Found>> {
        source::charge(work, 1)?;
        if let Some(found) = memo.get(&root) {
            return Ok(*found);
        }
        if self.blocked.contains(&root) {
            memo.insert(root, None);
            return Ok(None);
        }
        if !on_edge && let Some(location) = self.sinks.get(&root) {
            let found = Found {
                sink: Some(root),
                location: *location,
            };
            memo.insert(root, Some(found));
            return Ok(Some(found));
        }
        struct Frame {
            node: usize,
            next: usize,
            via: Option<i64>,
        }
        let mut stack = vec![Frame {
            node: root,
            next: 0,
            via: None,
        }];
        let mut visiting = HashSet::from([root]);
        loop {
            source::charge(work, 1)?;
            let frame = stack.last_mut().expect("live traversal frame");
            let Some(&(child, location)) = self.edges[frame.node].get(frame.next) else {
                let frame = stack.pop().expect("live traversal frame");
                visiting.remove(&frame.node);
                memo.insert(frame.node, None);
                if stack.is_empty() {
                    return Ok(None);
                }
                continue;
            };
            frame.next += 1;
            let found = if on_edge && self.sinks.contains_key(&child) {
                Some(Found {
                    sink: Some(child),
                    location,
                })
            } else if visiting.contains(&child) || self.blocked.contains(&child) {
                continue;
            } else if let Some(cached) = memo.get(&child) {
                cached.map(|found| Found { location, ..found })
            } else if !on_edge && let Some(direct) = self.sinks.get(&child) {
                memo.insert(
                    child,
                    Some(Found {
                        sink: Some(child),
                        location: *direct,
                    }),
                );
                Some(Found {
                    sink: Some(child),
                    location,
                })
            } else {
                visiting.insert(child);
                stack.push(Frame {
                    node: child,
                    next: 0,
                    via: Some(location),
                });
                continue;
            };
            if let Some(mut found) = found {
                while let Some(frame) = stack.pop() {
                    visiting.remove(&frame.node);
                    memo.insert(frame.node, Some(found));
                    if let Some(location) = frame.via {
                        found.location = location;
                    }
                }
                return Ok(Some(found));
            }
        }
    }
}

struct Output {
    scopes: Vec<String>,
    roots: Vec<String>,
    sinks: Vec<Option<String>>,
    locations: Vec<Option<i64>>,
    ordinals: Vec<i64>,
    emit_memo: bool,
    postings: [HashMap<String, Vec<usize>>; 2],
    work: Arc<AtomicUsize>,
}
impl syntax::Source for Output {
    fn columns(&self) -> Vec<String> {
        let mut fields = vec![
            "__path".into(),
            "root".into(),
            "node".into(),
            "location".into(),
        ];
        if self.emit_memo {
            fields.push("ordinal".into());
        }
        fields
    }
    fn column_types(&self) -> Vec<String> {
        let mut fields = vec![
            "TEXT".into(),
            "TEXT".into(),
            "TEXT".into(),
            "INTEGER".into(),
        ];
        if self.emit_memo {
            fields.push("INTEGER".into());
        }
        fields
    }
    fn len(&self) -> usize {
        self.roots.len()
    }
    fn indexed(&self, column: usize) -> bool {
        column < 2
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        let distinct = self.postings.get(column).map_or(1, HashMap::len).max(1);
        self.len()
            .div_ceil(distinct)
            .saturating_mul(if batch { 256 } else { 1 })
            .min(self.len())
            .max(1)
    }
    fn lookup(&self, column: usize, values: &[Value]) -> rusqlite::Result<Option<Vec<usize>>> {
        let Some(postings) = self.postings.get(column) else {
            return Ok(None);
        };
        let mut rows = Vec::new();
        for value in values {
            self.charge(1)?;
            if let Value::Text(value) = value
                && let Some(found) = postings.get(value)
            {
                rows.extend_from_slice(found);
            }
        }
        rows.sort_unstable();
        rows.dedup();
        self.charge(rows.len())?;
        Ok(Some(rows))
    }
    fn column(&self, row: usize, column: usize, ctx: &mut Context) -> rusqlite::Result<()> {
        self.charge(1)?;
        match column {
            0 => ctx.set_result(&self.scopes[row]),
            1 => ctx.set_result(&self.roots[row]),
            2 => ctx.set_result(&self.sinks[row]),
            3 => ctx.set_result(&self.locations[row]),
            _ => ctx.set_result(&self.ordinals[row]),
        }
    }
    fn charge(&self, count: usize) -> rusqlite::Result<()> {
        source::charge(&self.work, count)
            .map_err(|e| rusqlite::Error::UserFunctionError(e.reason.into()))
    }
}
pub(super) fn prepare(
    db: &super::Database,
    request: &super::Reachability,
    spec: &Spec,
    work: &Arc<AtomicUsize>,
) -> Result<Arc<dyn syntax::Source>> {
    let mut graphs = BTreeMap::<String, Graph>::new();
    let mut roots = db
        .prepare(&format!(
            "SELECT __path,id FROM ({}) ORDER BY __path,ordinal,id",
            request.roots
        ))
        .map_err(sql_error)?;
    let rows = roots
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(sql_error)?;
    for row in rows {
        source::charge(work, 1)?;
        let (scope, key) = row.map_err(sql_error)?;
        let graph = graphs.entry(scope).or_default();
        let id = graph.intern(key);
        if graph.requested.insert(id) {
            graph.roots.push(id);
        }
    }
    if !graphs.is_empty() {
        let mut edges = db
            .prepare(&format!(
                "SELECT __path,src,dst,location FROM ({}) ORDER BY __path,src,ordinal,dst",
                request.edges
            ))
            .map_err(sql_error)?;
        let rows = edges
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })
            .map_err(sql_error)?;
        for row in rows {
            source::charge(work, 1)?;
            let (scope, src, dst, location) = row.map_err(sql_error)?;
            if let Some(graph) = graphs.get_mut(&scope) {
                let src = graph.intern(src);
                let dst = graph.intern(dst);
                graph.edges[src].push((dst, location));
            }
        }
        let mut sinks = db.prepare(&spec.sinks).map_err(sql_error)?;
        let rows = sinks
            .query_map([], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .map_err(sql_error)?;
        let mut shared = HashMap::new();
        for row in rows {
            source::charge(work, 1)?;
            let (scope, id, location) = row.map_err(sql_error)?;
            let prior = if let Some(scope) = scope {
                if let Some(graph) = graphs.get_mut(&scope) {
                    let id = graph.intern(id);
                    graph.sinks.insert(id, location)
                } else {
                    None
                }
            } else {
                shared.insert(id, location)
            };
            if let Some(prior) = prior
                && prior != location
            {
                return Err(invalid(
                    "ordered first-match sinks must be unique per scope and id",
                ));
            }
        }
        // NULL-scope sinks are shared without expanding their rows over a union of graph scopes.
        for graph in graphs.values_mut() {
            for (id, key) in graph.keys.iter().enumerate() {
                source::charge(work, 1)?;
                if let Some(location) = shared.get(key) {
                    graph.sinks.entry(id).or_insert(*location);
                }
            }
        }
    }
    if let Some(query) = &spec.memo {
        let mut statement = db
            .prepare(&format!(
                "SELECT __path,id,node,location FROM ({query}) ORDER BY __path,ordinal"
            ))
            .map_err(sql_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                ))
            })
            .map_err(sql_error)?;
        for row in rows {
            source::charge(work, 1)?;
            let (scope, key, sink, location) = row.map_err(sql_error)?;
            if let Some(graph) = graphs.get_mut(&scope) {
                let id = graph.intern(key);
                let sink = sink.map(|key| graph.intern(key));
                graph
                    .initial
                    .insert(id, location.map(|location| Found { sink, location }));
            }
        }
    }
    if let Some(query) = &spec.visiting {
        let mut statement = db.prepare(query).map_err(sql_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(sql_error)?;
        for row in rows {
            source::charge(work, 1)?;
            let (scope, key) = row.map_err(sql_error)?;
            if let Some(graph) = graphs.get_mut(&scope) {
                let id = graph.intern(key);
                graph.blocked.insert(id);
            }
        }
    }
    let mut output = Output {
        scopes: Vec::new(),
        roots: Vec::new(),
        sinks: Vec::new(),
        locations: Vec::new(),
        ordinals: Vec::new(),
        emit_memo: spec.emit_memo,
        postings: Default::default(),
        work: Arc::clone(work),
    };
    for (scope, graph) in graphs {
        let mut memo = graph.initial.clone();
        let mut selected = IndexMap::new();
        for &root in &graph.roots {
            if let Some(found) = graph.first(root, spec.on_edge, &mut memo, work)? {
                selected.insert(root, Some(found));
            }
        }
        for (ordinal, (root, found)) in if spec.emit_memo { memo } else { selected }
            .into_iter()
            .enumerate()
        {
            let row = output.roots.len();
            output.postings[0]
                .entry(scope.clone())
                .or_default()
                .push(row);
            output.postings[1]
                .entry(graph.keys[root].clone())
                .or_default()
                .push(row);
            output.scopes.push(scope.clone());
            output.roots.push(graph.keys[root].clone());
            output.sinks.push(
                found
                    .and_then(|value| value.sink)
                    .map(|sink| graph.keys[sink].clone()),
            );
            output.locations.push(found.map(|value| value.location));
            output.ordinals.push(ordinal as i64);
        }
    }
    Ok(Arc::new(output))
}

pub(super) fn create(
    db: &super::Database,
    stage: &Stage,
    request: &super::Reachability,
    spec: &Spec,
    work: &Arc<AtomicUsize>,
) -> Result<()> {
    table::bind(db, &stage.name, prepare(db, request, spec, work)?).map_err(sql_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supplied_memo_and_visiting_preserve_cached_results_and_insertion_order() {
        let work = AtomicUsize::new(1000);
        let mut graph = Graph::default();
        let root = graph.intern("root".into());
        let child = graph.intern("child".into());
        let target = graph.intern("target".into());
        graph.edges[root] = vec![(child, 4), (target, 10)];
        graph.sinks.insert(target, 99);
        let mut memo = IndexMap::from([
            (child, None),
            (
                root,
                Some(Found {
                    sink: None,
                    location: 77,
                }),
            ),
        ]);
        assert_eq!(
            graph
                .first(root, true, &mut memo, &work)
                .unwrap()
                .unwrap()
                .location,
            77
        );
        assert_eq!(memo.keys().copied().collect::<Vec<_>>(), vec![child, root]);
        memo.shift_remove(&root);
        graph.blocked.insert(target);
        // A direct target is checked before the initial visiting set.
        assert_eq!(
            graph.first(root, true, &mut memo, &work).unwrap(),
            Some(Found {
                sink: Some(target),
                location: 10
            })
        );
        graph.blocked.insert(root);
        assert!(
            graph
                .first(root, true, &mut IndexMap::new(), &work)
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn cycles_preserve_negative_memo_entries_and_edge_order() {
        let work = AtomicUsize::new(1000);
        let mut graph = Graph::default();
        let a = graph.intern("a".into());
        let b = graph.intern("b".into());
        let sink = graph.intern("sink".into());
        graph.edges[a] = vec![(b, 90), (sink, 12)];
        graph.edges[b] = vec![(a, 11)];
        graph.sinks.insert(sink, 100);
        let mut memo = IndexMap::new();
        assert_eq!(
            graph.first(a, false, &mut memo, &work).unwrap(),
            Some(Found {
                sink: Some(sink),
                location: 12
            })
        );
        assert_eq!(graph.first(b, false, &mut memo, &work).unwrap(), None);
        let mut memo = IndexMap::new();
        graph.edges[a] = vec![(sink, 90), (b, 12)];
        assert_eq!(
            graph.first(a, false, &mut memo, &work).unwrap(),
            Some(Found {
                sink: Some(sink),
                location: 90
            })
        );
        assert_eq!(
            graph.first(b, false, &mut memo, &work).unwrap(),
            Some(Found {
                sink: Some(sink),
                location: 11
            })
        );
    }
    #[test]
    fn node_matches_and_edge_matches_have_distinct_root_semantics() {
        let work = AtomicUsize::new(1000);
        let mut graph = Graph::default();
        let a = graph.intern("a".into());
        let b = graph.intern("b".into());
        graph.edges[a] = vec![(b, 7)];
        graph.sinks.insert(a, 2);
        graph.sinks.insert(b, 10);
        assert_eq!(
            graph.first(a, false, &mut IndexMap::new(), &work).unwrap(),
            Some(Found {
                sink: Some(a),
                location: 2
            })
        );
        assert_eq!(
            graph.first(a, true, &mut IndexMap::new(), &work).unwrap(),
            Some(Found {
                sink: Some(b),
                location: 7
            })
        );
        assert!(
            graph
                .first(a, false, &mut IndexMap::new(), &AtomicUsize::new(0))
                .is_err()
        );
    }
}
