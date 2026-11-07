//! Finite-set dataflow: OUT[n] = GEN[n] union (IN[n] minus KILL[n]).
//! Both SQL interfaces share one completed, bounded native fixed point.
use crate::{
    graph::failure,
    rows::{OwnedRows, Source, register as register_rows},
};
use rusqlite::{Connection, Result, functions::FunctionFlags, types::Value, vtab::Context};
use serde_json::json;
use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
struct Flow {
    nodes: Vec<i64>,
    edges: Vec<[i64; 2]>,
    seeds: Vec<[i64; 2]>,
    kills: Vec<[i64; 2]>,
    budget: u64,
    millis: u64,
    answer: OnceLock<std::result::Result<Answer, String>>,
    builds: AtomicU64,
}
struct Answer {
    rows: OwnedRows,
    node_ids: HashMap<i64, usize>,
    label_ids: HashMap<i64, usize>,
    bits: Vec<Vec<u64>>,
    work: u64,
}
impl Flow {
    fn solve(&self) -> Result<Answer> {
        self.builds.fetch_add(1, Ordering::Relaxed);
        let started = Instant::now();
        let mut work = 0_u64;
        let mut charge = || -> Result<()> {
            work += 1;
            if work > self.budget
                || work.is_multiple_of(256)
                    && started.elapsed() > Duration::from_millis(self.millis)
            {
                Err(failure("dataflow work budget or deadline exceeded"))
            } else {
                Ok(())
            }
        };
        let mut all: BTreeSet<i64> = self.nodes.iter().copied().collect();
        for &[a, b] in &self.edges {
            charge()?;
            all.insert(a);
            all.insert(b);
        }
        for &[n, _] in self.seeds.iter().chain(&self.kills) {
            charge()?;
            all.insert(n);
        }
        let nodes = all.into_iter().collect::<Vec<_>>();
        let labels = self
            .seeds
            .iter()
            .chain(&self.kills)
            .map(|p| p[1])
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let node_ids = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (*n, i))
            .collect::<HashMap<_, _>>();
        let label_ids = labels
            .iter()
            .enumerate()
            .map(|(i, n)| (*n, i))
            .collect::<HashMap<_, _>>();
        let words = labels.len().div_ceil(64);
        if nodes.len().saturating_mul(words) > 8_000_000 {
            return Err(failure("dataflow state exceeds memory budget"));
        }
        let mut bits = vec![vec![0_u64; words]; nodes.len()];
        let mut kills = bits.clone();
        let mut generated = bits.clone();
        let mut forward = vec![Vec::new(); nodes.len()];
        for &[a, b] in &self.edges {
            charge()?;
            forward[node_ids[&a]].push(node_ids[&b]);
        }
        for &[n, label] in &self.seeds {
            charge()?;
            let j = label_ids[&label];
            bits[node_ids[&n]][j / 64] |= 1 << (j % 64);
            generated[node_ids[&n]][j / 64] |= 1 << (j % 64);
        }
        for &[n, label] in &self.kills {
            charge()?;
            let j = label_ids[&label];
            kills[node_ids[&n]][j / 64] |= 1 << (j % 64);
        }
        let mut queued = vec![false; nodes.len()];
        let mut pending = VecDeque::new();
        for (i, row) in bits.iter().enumerate() {
            charge()?;
            if row.iter().any(|v| *v != 0) {
                pending.push_back(i);
                queued[i] = true;
            }
        }
        while let Some(n) = pending.pop_front() {
            charge()?;
            queued[n] = false;
            for &dst in &forward[n] {
                let mut changed = false;
                for word in 0..words {
                    charge()?;
                    let next = bits[dst][word]
                        | (bits[n][word] & (!kills[dst][word] | generated[dst][word]));
                    if next != bits[dst][word] {
                        bits[dst][word] = next;
                        changed = true;
                    }
                }
                if changed && !queued[dst] {
                    pending.push_back(dst);
                    queued[dst] = true;
                }
            }
        }
        let mut rows = Vec::new();
        for (i, row) in bits.iter().enumerate() {
            for (j, label) in labels.iter().enumerate() {
                charge()?;
                if row[j / 64] & (1 << (j % 64)) != 0 {
                    rows.push(vec![Value::Integer(nodes[i]), Value::Integer(*label)]);
                }
            }
        }
        let rows = OwnedRows::new(
            vec!["node".into(), "label".into()],
            rows,
            vec![0, 1],
            Arc::new(|_| Ok(())),
        )?;
        Ok(Answer {
            rows,
            node_ids,
            label_ids,
            bits,
            work,
        })
    }
    fn get(&self) -> Result<&Answer> {
        self.answer
            .get_or_init(|| self.solve().map_err(|e| e.to_string()))
            .as_ref()
            .map_err(|e| failure(e))
    }
}
impl Source for Flow {
    fn columns(&self) -> Vec<String> {
        vec!["node".into(), "label".into()]
    }
    fn len(&self) -> usize {
        self.answer
            .get()
            .and_then(|v| v.as_ref().ok())
            .map_or(0, |v| v.rows.rows.len())
    }
    fn estimated_len(&self) -> usize {
        self.nodes.len()
    }
    fn initialize(&self) -> Result<()> {
        self.get().map(|_| ())
    }
    fn indexed(&self, c: usize) -> bool {
        c < 2
    }
    fn lookup(&self, c: usize, v: &[Value]) -> Result<Option<Vec<usize>>> {
        self.get()?.rows.lookup(c, v)
    }
    fn column(&self, r: usize, c: usize, ctx: &mut Context) -> Result<()> {
        self.get()?.rows.column(r, c, ctx)
    }
}
pub(crate) fn register(db: &Connection) -> Result<()> {
    let registry = Arc::new(Mutex::new(HashMap::<String, Arc<Flow>>::new()));
    let shared = Arc::clone(&registry);
    db.create_scalar_function(c"flow_load", 8, FunctionFlags::SQLITE_UTF8, move |ctx| {
        let parse =
            |i| serde_json::from_str(&ctx.get::<String>(i)?).map_err(|e| failure(&e.to_string()));
        let name = ctx.get::<String>(0)?;
        let nodes: Vec<i64> =
            serde_json::from_value(parse(1)?).map_err(|e| failure(&e.to_string()))?;
        let edges: Vec<[i64; 2]> =
            serde_json::from_value(parse(2)?).map_err(|e| failure(&e.to_string()))?;
        let seeds: Vec<[i64; 2]> =
            serde_json::from_value(parse(3)?).map_err(|e| failure(&e.to_string()))?;
        let kills: Vec<[i64; 2]> =
            serde_json::from_value(parse(4)?).map_err(|e| failure(&e.to_string()))?;
        let budget =
            u64::try_from(ctx.get::<i64>(5)?).map_err(|_| failure("negative dataflow budget"))?;
        let millis =
            u64::try_from(ctx.get::<i64>(6)?).map_err(|_| failure("negative dataflow deadline"))?;
        let eager = ctx.get::<bool>(7)?;
        if !(1..=110_000).contains(&millis) {
            return Err(failure("invalid dataflow deadline"));
        }
        let flow = Arc::new(Flow {
            nodes,
            edges,
            seeds,
            kills,
            budget,
            millis,
            answer: OnceLock::new(),
            builds: AtomicU64::new(0),
        });
        if eager {
            flow.initialize()?;
        }
        let db = unsafe { ctx.get_connection()? };
        register_rows(&db, &name, flow.clone())?;
        shared
            .lock()
            .map_err(|_| failure("flow registry poisoned"))?
            .insert(name, flow);
        Ok(1)
    })?;
    let shared = Arc::clone(&registry);
    db.create_scalar_function(c"flow_has", 3, FunctionFlags::SQLITE_UTF8, move |ctx| {
        let registry = shared
            .lock()
            .map_err(|_| failure("flow registry poisoned"))?;
        let name = ctx.get::<String>(0)?;
        let flow = registry
            .get(&name)
            .ok_or_else(|| failure("unknown flow snapshot"))?;
        let answer = flow.get()?;
        let node = ctx.get::<i64>(1)?;
        let label = ctx.get::<i64>(2)?;
        Ok(
            match (answer.node_ids.get(&node), answer.label_ids.get(&label)) {
                (Some(&n), Some(&l)) => i64::from(answer.bits[n][l / 64] & (1 << (l % 64)) != 0),
                _ => 0,
            },
        )
    })?;
    db.create_scalar_function(c"flow_stats",1,FunctionFlags::SQLITE_UTF8,move|ctx| {
        let registry=registry.lock().map_err(|_|failure("flow registry poisoned"))?;
        let name=ctx.get::<String>(0)?;let flow=registry.get(&name).ok_or_else(||failure("unknown flow snapshot"))?;
        Ok(json!({"builds":flow.builds.load(Ordering::Relaxed),"complete":flow.answer.get().is_some_and(|v|v.is_ok()),"work":flow.answer.get().and_then(|v|v.as_ref().ok()).map(|v|v.work)}).to_string())
    })?;
    Ok(())
}
