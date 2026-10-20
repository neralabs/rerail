use rusqlite::{Error, Result};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub type Shared = Arc<Mutex<Registry>>;
pub type SharedGraph = Arc<Mutex<Graph>>;

#[derive(Default)]
pub struct Registry {
    pub next: i64,
    pub graphs: HashMap<i64, SharedGraph>,
}

pub fn failure(message: &str) -> Error {
    Error::UserFunctionError(message.to_owned().into())
}

pub fn lookup(registry: &Shared, id: i64) -> Result<SharedGraph> {
    registry
        .lock()
        .map_err(|_| failure("registry poisoned"))?
        .graphs
        .get(&id)
        .cloned()
        .ok_or_else(|| failure("unknown graph snapshot"))
}

pub struct Graph {
    nodes: HashSet<i64>,
    forward: HashMap<i64, Vec<i64>>,
    reverse: HashMap<i64, Vec<i64>>,
    cache: HashMap<(i64, bool), Arc<Vec<i64>>>,
    cache_entries: usize,
    remaining: i64,
    deadline: Instant,
    work: u64,
    traversals: u64,
    cache_hits: u64,
    pub scalar_calls: u64,
    pub filters: u64,
    pub batch_filters: u64,
    pub requested_roots: u64,
    pub emitted: u64,
}

impl Graph {
    pub fn new(nodes: Vec<i64>, edges: Vec<[i64; 2]>, budget: i64, millis: i64) -> Result<Self> {
        if budget < 0 || !(1..=110_000).contains(&millis) {
            return Err(failure("invalid graph work budget or deadline"));
        }
        let mut graph = Self {
            nodes: HashSet::new(),
            forward: HashMap::new(),
            reverse: HashMap::new(),
            cache: HashMap::new(),
            cache_entries: 0,
            remaining: budget,
            deadline: Instant::now() + Duration::from_millis(millis as u64),
            work: 0,
            traversals: 0,
            cache_hits: 0,
            scalar_calls: 0,
            filters: 0,
            batch_filters: 0,
            requested_roots: 0,
            emitted: 0,
        };
        for node in nodes {
            graph.charge()?;
            graph.nodes.insert(node);
        }
        for [a, b] in edges {
            graph.charge()?;
            graph.nodes.insert(a);
            graph.nodes.insert(b);
            graph.forward.entry(a).or_default().push(b);
            graph.reverse.entry(b).or_default().push(a);
        }
        Ok(graph)
    }

    pub fn charge(&mut self) -> Result<()> {
        Self::charge_parts(&mut self.remaining, &mut self.work, self.deadline)
    }

    fn charge_parts(remaining: &mut i64, work: &mut u64, deadline: Instant) -> Result<()> {
        if *remaining == 0 || (work.is_multiple_of(256) && Instant::now() >= deadline) {
            return Err(failure("reachability work budget or deadline exceeded"));
        }
        *remaining -= 1;
        *work += 1;
        Ok(())
    }

    pub fn reachable(&mut self, root: i64, reverse: bool) -> Result<Arc<Vec<i64>>> {
        self.charge()?;
        if let Some(values) = self.cache.get(&(root, reverse)) {
            self.cache_hits += 1;
            return Ok(Arc::clone(values));
        }
        self.traversals += 1;
        // Do not accumulate unbounded empty entries for arbitrary unknown IDs.
        if !self.nodes.contains(&root) {
            return Ok(Arc::new(vec![]));
        }
        let mut seen = HashSet::new();
        let mut pending = Vec::new();
        seen.insert(root);
        pending.push(root);
        while let Some(node) = pending.pop() {
            let neighbors = if reverse {
                &self.reverse
            } else {
                &self.forward
            }
            .get(&node)
            .map(Vec::as_slice)
            .unwrap_or_default();
            for &child in neighbors {
                Self::charge_parts(&mut self.remaining, &mut self.work, self.deadline)?;
                if seen.insert(child) {
                    pending.push(child);
                }
            }
        }
        let mut values: Vec<_> = seen.into_iter().collect();
        values.sort_unstable();
        let values = Arc::new(values);
        // Bound cached members across roots. Large single traversals are not retained.
        const CACHE_MEMBERS: usize = 250_000;
        if values.len() <= CACHE_MEMBERS {
            if self.cache_entries + values.len() > CACHE_MEMBERS {
                self.cache.clear();
                self.cache_entries = 0;
            }
            self.cache_entries += values.len();
            self.cache.insert((root, reverse), Arc::clone(&values));
        }
        Ok(values)
    }

    pub fn stats(&self) -> Value {
        json!({"nodes":self.nodes.len(), "work":self.work, "traversals":self.traversals,
            "cacheHits":self.cache_hits, "cacheMembers":self.cache_entries,
            "scalarCalls":self.scalar_calls, "filters":self.filters,
            "batchFilters":self.batch_filters, "requestedRoots":self.requested_roots,
            "emitted":self.emitted})
    }
}
