//! The reach index (decision 0115): a transitive closure held as one compressed bitmap per start, not as its pairs.
//!
//! A relation of the shape
//!
//! ```text
//! r(x, y) :- seed(x, y).            // any rules not reading r
//! r(x, z) :- r(x, y), edge(y, z).   // the one recursive rule
//! ```
//!
//! is each start `x`'s set of nodes reachable over `edge` (none or more steps) from its seeds. `components.rr`'s
//! `rr_cx_reach` is one: 54 files' imports reach 6.3 M pairs, which as rows cost 220 MB and the cross-file part's
//! whole keep budget. Here the graph is condensed into its strongly connected components, each component's closure
//! is one bitmap (`roaring`) over the nodes, made from its successors' (sinks first), and a start holds the union of
//! its seeds' closures — shared, not copied, where it has one seed. A relation through the closure,
//! `s(x, q) :- r(x, e), map(e, q)`, is the same over `map`'s values: each component's closure of what its nodes map
//! to.
//!
//! The index is a `Source`: its rows are `(x, z)` ordered by `x`, then `z`, and row ids are a start's offset plus the
//! node's rank in its bitmap. A probe of both columns is a bitmap test; of the start, a range of rows; of the node,
//! a test per start. Statistics are counted from the bitmaps, never from rows.
use crate::code_rule_dsl::facts::vtab::Source;
use roaring::RoaringBitmap;
use std::sync::{Arc, OnceLock};

/// The edges' graph over dense node ids (values sorted, so an id's order is its value's), condensed: each node's
/// component, components numbered sinks first (a component's successors have smaller numbers).
pub struct Graph {
    nodes: Vec<i64>,
    comp: Vec<u32>,
    members: Vec<Vec<u32>>,
    succ: Vec<Vec<u32>>,
    /// The distinct edges, and those from a node to itself.
    edges: usize,
    loops: usize,
    /// Each node's weakly connected component (the edges taken both ways), numbered from 0.
    weak: Vec<u32>,
}

/// The graph's shape for the run's explain output: its cycles and its connected parts, largest first.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub nodes: usize,
    pub edges: usize,
    /// Strongly connected components, and the cycles among them: components of two or more nodes, and nodes with
    /// an edge to themselves.
    pub components: usize,
    pub cyclic: usize,
    pub self_loops: usize,
    /// The largest cycles: each one's size and its first nodes' values (at most `SAMPLE`).
    pub largest_cycles: Vec<(usize, Vec<i64>)>,
    /// Weakly connected components: how many, and the largest ones' sizes with their first nodes' values.
    pub connected: usize,
    pub largest_connected: Vec<(usize, Vec<i64>)>,
}

/// How many of the largest cycles and connected parts the shape lists, and how many nodes of each.
pub const LARGEST: usize = 5;
pub const SAMPLE: usize = 4;

impl Graph {
    /// The graph of `edges` with `extra` nodes (seeds' targets that no edge touches).
    pub fn new(edges: &[(i64, i64)], extra: impl IntoIterator<Item = i64>) -> Graph {
        let mut nodes: Vec<i64> = edges.iter().flat_map(|&(a, b)| [a, b]).chain(extra).collect();
        nodes.sort_unstable();
        nodes.dedup();
        let id = |v: i64| nodes.binary_search(&v).expect("a node") as u32;
        let n = nodes.len();
        // Adjacency as one array of targets with offsets.
        let mut pairs: Vec<(u32, u32)> = edges.iter().map(|&(a, b)| (id(a), id(b))).collect();
        pairs.sort_unstable();
        pairs.dedup();
        let mut start = vec![0u32; n + 1];
        for &(a, _) in &pairs {
            start[a as usize + 1] += 1;
        }
        for i in 0..n {
            start[i + 1] += start[i];
        }
        let adj: Vec<u32> = pairs.iter().map(|&(_, b)| b).collect();
        let out = |v: usize| &adj[start[v] as usize..start[v + 1] as usize];
        // Tarjan's components, iteratively: each is emitted after every component it reaches.
        const NONE: u32 = u32::MAX;
        let (mut index, mut low, mut comp) = (vec![NONE; n], vec![0u32; n], vec![NONE; n]);
        let (mut stack, mut on) = (Vec::new(), vec![false; n]);
        let mut members: Vec<Vec<u32>> = Vec::new();
        let mut next = 0u32;
        let mut call: Vec<(u32, u32)> = Vec::new();
        for root in 0..n as u32 {
            if index[root as usize] != NONE {
                continue;
            }
            call.push((root, 0));
            while let Some(&mut (v, ref mut at)) = call.last_mut() {
                let vu = v as usize;
                if *at == 0 && index[vu] == NONE {
                    index[vu] = next;
                    low[vu] = next;
                    next += 1;
                    stack.push(v);
                    on[vu] = true;
                }
                let targets = out(vu);
                if let Some(&w) = targets.get(*at as usize) {
                    *at += 1;
                    let wu = w as usize;
                    if index[wu] == NONE {
                        call.push((w, 0));
                    } else if on[wu] {
                        low[vu] = low[vu].min(index[wu]);
                    }
                    continue;
                }
                call.pop();
                if let Some(&(p, _)) = call.last() {
                    low[p as usize] = low[p as usize].min(low[vu]);
                }
                if low[vu] == index[vu] {
                    let c = members.len() as u32;
                    let mut m = Vec::new();
                    while let Some(w) = stack.pop() {
                        on[w as usize] = false;
                        comp[w as usize] = c;
                        m.push(w);
                        if w == v {
                            break;
                        }
                    }
                    members.push(m);
                }
            }
        }
        let mut succ: Vec<Vec<u32>> = vec![Vec::new(); members.len()];
        for &(a, b) in &pairs {
            let (ca, cb) = (comp[a as usize], comp[b as usize]);
            if ca != cb {
                succ[ca as usize].push(cb);
            }
        }
        for s in &mut succ {
            s.sort_unstable();
            s.dedup();
        }
        let loops = pairs.iter().filter(|(a, b)| a == b).count();
        let weak = weak_components(n, &pairs);
        Graph { nodes, comp, members, succ, edges: pairs.len(), loops, weak }
    }

    /// Its cycles and connected parts (`Shape`).
    pub fn shape(&self) -> Shape {
        let sample = |ids: &mut Vec<u32>| {
            ids.sort_unstable();
            ids.iter().take(SAMPLE).map(|&i| self.nodes[i as usize]).collect::<Vec<i64>>()
        };
        let self_loops = self.loops;
        let mut cycles: Vec<&Vec<u32>> = self.members.iter().filter(|m| m.len() > 1).collect();
        cycles.sort_by_key(|m| std::cmp::Reverse(m.len()));
        let largest_cycles = cycles.iter().take(LARGEST).map(|m| (m.len(), sample(&mut m.to_vec()))).collect();
        let connected = self.weak.iter().map(|&w| w as usize + 1).max().unwrap_or(0);
        let mut parts: Vec<Vec<u32>> = vec![Vec::new(); connected];
        for (i, &w) in self.weak.iter().enumerate() {
            parts[w as usize].push(i as u32);
        }
        parts.sort_by_key(|p| std::cmp::Reverse(p.len()));
        let largest_connected = parts.iter_mut().take(LARGEST).map(|p| (p.len(), sample(p))).collect();
        Shape {
            nodes: self.nodes.len(),
            edges: self.edges,
            components: self.members.len(),
            cyclic: cycles.len(),
            self_loops,
            largest_cycles,
            connected,
            largest_connected,
        }
    }

    fn id(&self, v: i64) -> Option<u32> {
        self.nodes.binary_search(&v).ok().map(|i| i as u32)
    }

    /// Each component in `wanted`'s closure (and every one it reaches): the union of `base` over its nodes and its
    /// successors' closures. Components not reached from `wanted` get none.
    fn close(&self, wanted: &[u32], base: impl Fn(u32, &mut RoaringBitmap)) -> Vec<Option<Arc<RoaringBitmap>>> {
        let k = self.members.len();
        let mut need = vec![false; k];
        let mut todo: Vec<u32> = wanted.to_vec();
        while let Some(c) = todo.pop() {
            if !std::mem::replace(&mut need[c as usize], true) {
                todo.extend(self.succ[c as usize].iter().copied().filter(|&s| !need[s as usize]));
            }
        }
        let mut closed: Vec<Option<Arc<RoaringBitmap>>> = vec![None; k];
        // Sinks first: every successor of `c` is numbered below it.
        for c in 0..k {
            if !need[c] {
                continue;
            }
            let succ = &self.succ[c];
            let mut own = RoaringBitmap::new();
            for &m in &self.members[c] {
                base(m, &mut own);
            }
            // A component that adds nothing to its one successor shares that successor's closure.
            if let [s] = succ.as_slice() {
                let s = closed[*s as usize].as_ref().expect("closed before its predecessors");
                if own.is_subset(s) {
                    closed[c] = Some(Arc::clone(s));
                    continue;
                }
            }
            for &s in succ {
                own |= closed[s as usize].as_deref().expect("closed before its predecessors");
            }
            closed[c] = Some(Arc::new(own));
        }
        closed
    }
}

/// Each node's weakly connected component, numbered in order of each component's smallest node (union by size).
fn weak_components(n: usize, pairs: &[(u32, u32)]) -> Vec<u32> {
    let mut parent: Vec<u32> = (0..n as u32).collect();
    let mut size = vec![1u32; n];
    fn find(parent: &mut [u32], mut x: u32) -> u32 {
        while parent[x as usize] != x {
            parent[x as usize] = parent[parent[x as usize] as usize];
            x = parent[x as usize];
        }
        x
    }
    for &(a, b) in pairs {
        let (mut ra, mut rb) = (find(&mut parent, a), find(&mut parent, b));
        if ra == rb {
            continue;
        }
        if size[ra as usize] < size[rb as usize] {
            std::mem::swap(&mut ra, &mut rb);
        }
        parent[rb as usize] = ra;
        size[ra as usize] += size[rb as usize];
    }
    let mut number = vec![u32::MAX; n];
    let mut next = 0;
    (0..n as u32)
        .map(|x| {
            let r = find(&mut parent, x) as usize;
            if number[r] == u32::MAX {
                number[r] = next;
                next += 1;
            }
            number[r]
        })
        .collect()
}

/// A relation `(x, z)` held as each start's bitmap over `domain`'s values (`z = domain[id]`).
pub struct Closed {
    xs: Vec<i64>,
    sets: Vec<Arc<RoaringBitmap>>,
    /// Each start's first row id; one past the last row at the end.
    off: Vec<usize>,
    domain: Arc<Vec<i64>>,
    /// The graph and each start's seed components, for a relation through this one (`through`).
    graph: Arc<Graph>,
    seeds: Vec<Vec<u32>>,
    stats: OnceLock<Vec<(String, i64, i64, i64, i64)>>,
}

impl Closed {
    /// `r`'s rows from its seeds' `(x, y)` and the edges `(y, z)`: each start's nodes reached from its seeds.
    pub fn reach(seed: &[(i64, i64)], edges: &[(i64, i64)]) -> Closed {
        let graph = Arc::new(Graph::new(edges, seed.iter().map(|&(_, y)| y)));
        let (xs, seeds) = Self::starts(&graph, seed);
        let wanted: Vec<u32> = seeds.iter().flatten().copied().collect();
        let closed = graph.close(&wanted, |m, out| {
            out.insert(m);
        });
        let domain = Arc::new(graph.nodes.clone());
        Self::new(xs, seeds, &closed, domain, graph)
    }

    /// `s(x, q) :- r(x, e), map(e, q)` for this relation `r`: each start's values of `map` over the nodes it reaches.
    pub fn through(&self, map: &[(i64, i64)]) -> Closed {
        let graph = &self.graph;
        let mut domain: Vec<i64> = map.iter().map(|&(_, q)| q).collect();
        domain.sort_unstable();
        domain.dedup();
        let mut of: Vec<RoaringBitmap> = vec![RoaringBitmap::new(); graph.nodes.len()];
        for &(e, q) in map {
            if let Some(e) = graph.id(e) {
                of[e as usize].insert(domain.binary_search(&q).expect("a value") as u32);
            }
        }
        let wanted: Vec<u32> = self.seeds.iter().flatten().copied().collect();
        let closed = graph.close(&wanted, |m, out| *out |= &of[m as usize]);
        Self::new(self.xs.clone(), self.seeds.clone(), &closed, Arc::new(domain), Arc::clone(graph))
    }

    /// The distinct starts, and each one's seeds' components.
    fn starts(graph: &Graph, seed: &[(i64, i64)]) -> (Vec<i64>, Vec<Vec<u32>>) {
        let mut pairs: Vec<(i64, u32)> = seed.iter().map(|&(x, y)| (x, graph.comp[graph.id(y).expect("a seed's node") as usize])).collect();
        pairs.sort_unstable();
        pairs.dedup();
        let (mut xs, mut seeds): (Vec<i64>, Vec<Vec<u32>>) = (Vec::new(), Vec::new());
        for (x, c) in pairs {
            if xs.last() != Some(&x) {
                xs.push(x);
                seeds.push(Vec::new());
            }
            seeds.last_mut().unwrap().push(c);
        }
        (xs, seeds)
    }

    fn new(xs: Vec<i64>, seeds: Vec<Vec<u32>>, closed: &[Option<Arc<RoaringBitmap>>], domain: Arc<Vec<i64>>, graph: Arc<Graph>) -> Closed {
        let get = |c: u32| closed[c as usize].as_ref().expect("a seed's component is closed");
        let sets: Vec<Arc<RoaringBitmap>> = seeds
            .iter()
            .map(|cs| match cs.as_slice() {
                [c] => Arc::clone(get(*c)),
                cs => Arc::new(cs.iter().fold(RoaringBitmap::new(), |mut u, &c| {
                    u |= get(c).as_ref();
                    u
                })),
            })
            .collect();
        let mut off = Vec::with_capacity(sets.len() + 1);
        let mut at = 0usize;
        for s in &sets {
            off.push(at);
            at += s.len() as usize;
        }
        off.push(at);
        Closed { xs, sets, off, domain, graph, seeds, stats: OnceLock::new() }
    }

    /// The start of row `row`.
    fn start_of(&self, row: usize) -> usize {
        self.off.partition_point(|&o| o <= row) - 1
    }

    fn start(&self, x: i64) -> Option<usize> {
        self.xs.binary_search(&x).ok()
    }

    /// The row of `(x, z)`, if the relation holds it.
    pub fn row_of(&self, x: i64, z: i64) -> Option<usize> {
        let i = self.start(x)?;
        let id = self.domain.binary_search(&z).ok()? as u32;
        let set = &self.sets[i];
        set.contains(id).then(|| self.off[i] + set.rank(id) as usize - 1)
    }

    /// The graph it closes over: its cycles and connected parts.
    pub fn shape(&self) -> Shape {
        self.graph.shape()
    }

    /// The starts, the distinct bitmaps among them, and the widest start's row count.
    pub fn spread(&self) -> (usize, usize, usize) {
        let distinct = self.sets.iter().map(Arc::as_ptr).collect::<std::collections::HashSet<_>>().len();
        (self.xs.len(), distinct, self.sets.iter().map(|s| s.len() as usize).max().unwrap_or(0))
    }

    /// The bytes held: each distinct bitmap once, the starts, offsets and domain.
    pub fn bytes(&self) -> usize {
        let mut seen = std::collections::HashSet::new();
        let sets: usize = self.sets.iter().filter(|s| seen.insert(Arc::as_ptr(s))).map(|s| s.serialized_size()).sum();
        sets + self.xs.len() * 16 + self.off.len() * 8 + self.domain.len() * 8
    }

    fn counted(&self) -> Vec<(String, i64, i64, i64, i64)> {
        let n = self.len() as i64;
        // The start column: a start's rows are its bitmap; by document (`x >> 32`), the starts are contiguous.
        let (mut sq0, mut sqd0) = (0i64, 0i64);
        let mut i = 0;
        while i < self.xs.len() {
            let doc = self.xs[i] >> 32;
            let mut in_doc = 0i64;
            while i < self.xs.len() && self.xs[i] >> 32 == doc {
                let l = self.sets[i].len() as i64;
                sq0 += l * l;
                in_doc += l;
                i += 1;
            }
            sqd0 += in_doc * in_doc;
        }
        // The node column: each node's starts, and by the starts' documents.
        let mut count = vec![0i64; self.domain.len()];
        let mut in_doc = vec![0i64; self.domain.len()];
        let mut touched: Vec<u32> = Vec::new();
        let mut sqd1 = 0i64;
        let mut i = 0;
        while i < self.xs.len() {
            let doc = self.xs[i] >> 32;
            while i < self.xs.len() && self.xs[i] >> 32 == doc {
                for id in self.sets[i].iter() {
                    count[id as usize] += 1;
                    if in_doc[id as usize] == 0 {
                        touched.push(id);
                    }
                    in_doc[id as usize] += 1;
                }
                i += 1;
            }
            for id in touched.drain(..) {
                let c = std::mem::take(&mut in_doc[id as usize]);
                sqd1 += c * c;
            }
        }
        let ndv1 = count.iter().filter(|&&c| c > 0).count() as i64;
        let sq1 = count.iter().map(|c| c * c).sum();
        vec![("c0".into(), n, self.xs.len() as i64, sq0, sqd0), ("c1".into(), n, ndv1, sq1, sqd1)]
    }
}

impl Source for Closed {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn columns(&self) -> Vec<String> {
        vec!["c0".into(), "c1".into()]
    }
    fn len(&self) -> usize {
        *self.off.last().unwrap_or(&0)
    }
    fn row_at(&self, row: usize, columns: &[usize], out: &mut [i64]) {
        let i = self.start_of(row);
        for (o, &c) in out.iter_mut().zip(columns) {
            *o = if c == 0 { self.xs[i] } else { self.domain[self.sets[i].select((row - self.off[i]) as u32).expect("a row of the start") as usize] };
        }
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        let mut v = [0i64];
        self.row_at(row, &[column], &mut v);
        v[0]
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        let stats = self.statistics().unwrap_or_default();
        let ndv = stats.get(column).map_or(1, |s| s.2).max(1) as usize;
        let one = self.len().div_ceil(ndv).max(1);
        if batch { one * 8 } else { one }
    }
    fn estimated_matches_all(&self, columns: &[usize]) -> usize {
        if columns.contains(&0) && columns.contains(&1) { 1 } else { columns.first().map_or(self.len(), |&c| self.estimated_matches(c, false)) }
    }
    fn composite(&self) -> bool {
        true
    }
    fn lookup_all(&self, equal: &[(usize, i64)]) -> Option<Vec<usize>> {
        let x = equal.iter().filter(|e| e.0 == 0).map(|e| e.1).collect::<Vec<_>>();
        let z = equal.iter().filter(|e| e.0 == 1).map(|e| e.1).collect::<Vec<_>>();
        if x.windows(2).any(|w| w[0] != w[1]) || z.windows(2).any(|w| w[0] != w[1]) {
            return Some(Vec::new());
        }
        match (x.first(), z.first()) {
            (Some(&x), Some(&z)) => Some(self.row_of(x, z).into_iter().collect()),
            (Some(&x), None) => self.lookup(0, &[x]),
            (None, Some(&z)) => self.lookup(1, &[z]),
            (None, None) => None,
        }
    }
    fn filter_eq(&self, eqs: &[(usize, i64)]) -> Vec<u32> {
        match self.lookup_all(eqs) {
            Some(rows) => rows.into_iter().map(|r| r as u32).collect(),
            None => (0..self.len() as u32).collect(),
        }
    }
    fn indexed(&self, column: usize) -> bool {
        column < 2
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        let mut out = Vec::new();
        match column {
            0 => {
                for &v in values {
                    if let Some(i) = self.start(v) {
                        out.extend(self.off[i]..self.off[i + 1]);
                    }
                }
            }
            1 => {
                let ids: Vec<u32> = values.iter().filter_map(|v| self.domain.binary_search(v).ok().map(|i| i as u32)).collect();
                for (i, set) in self.sets.iter().enumerate() {
                    for &id in &ids {
                        if set.contains(id) {
                            out.push(self.off[i] + set.rank(id) as usize - 1);
                        }
                    }
                }
            }
            _ => return None,
        }
        out.sort_unstable();
        out.dedup();
        Some(out)
    }
    fn value_counts(&self, wanted: &[(usize, i64)]) -> Option<Vec<i64>> {
        Some(
            wanted
                .iter()
                .map(|&(c, v)| match c {
                    0 => self.start(v).map_or(0, |i| self.sets[i].len() as i64),
                    1 => self.domain.binary_search(&v).map_or(0, |id| self.sets.iter().filter(|s| s.contains(id as u32)).count() as i64),
                    _ => 0,
                })
                .collect(),
        )
    }
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        Some(self.stats.get_or_init(|| self.counted()).clone())
    }
}
