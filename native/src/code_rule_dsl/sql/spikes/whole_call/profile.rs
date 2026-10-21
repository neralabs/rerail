//! Profiles generated SQL stages inside their statement. EXPLAIN runs after execution and never
//! contributes to runtime statement counts. Timers surround sets, not individual rows.
use super::graph::Graph;
use crate::code_rule_dsl::sql::timing::Started;
use rusqlite::{Connection, Result, functions::FunctionFlags};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

struct Span {
    id: usize,
    started: Started,
    vm: i32,
}
#[derive(Default)]
struct State {
    active: Option<Span>,
    stages: Vec<Value>,
    publications: Vec<Value>,
}
pub(super) struct Profile {
    pub enabled: bool,
    pub sampled_vm: AtomicUsize,
    state: Mutex<State>,
}
impl Profile {
    pub fn new(enabled: bool) -> Arc<Self> {
        Arc::new(Self {
            enabled,
            sampled_vm: AtomicUsize::new(0),
            state: Mutex::new(State::default()),
        })
    }
    fn vm(&self) -> i32 {
        self.sampled_vm.load(Ordering::Relaxed) as i32
    }
    pub fn publication(&self, name: &str, rows: usize, cells: usize, cpu: u64) {
        if self.enabled {
            self.state
                .lock()
                .unwrap()
                .publications
                .push(json!({"handle":name,"rows":rows,"cells":cells,"copyCpuNs":cpu}));
        }
    }
    pub fn register(self: &Arc<Self>, connection: &Connection) -> Result<()> {
        let this = Arc::clone(self);
        connection.create_scalar_function(
            "profile_begin",
            1,
            FunctionFlags::SQLITE_UTF8,
            move |ctx| {
                if this.enabled {
                    let mut state = this.state.lock().unwrap();
                    if state.active.is_some() {
                        return Err(rusqlite::Error::ModuleError(
                            "nested SQL stage profile".into(),
                        ));
                    }
                    state.active = Some(Span {
                        id: ctx.get::<i64>(0)? as usize,
                        started: Started::now(),
                        vm: this.vm(),
                    });
                }
                Ok(1i64)
            },
        )?;
        let this = Arc::clone(self);
        connection.create_scalar_function("profile_end",2,FunctionFlags::SQLITE_UTF8,move |ctx| {
            let token:String=ctx.get(1)?;
            if this.enabled {
                let id=ctx.get::<i64>(0)? as usize;
                let mut state=this.state.lock().unwrap();
                let span=state.active.take().ok_or_else(||rusqlite::Error::ModuleError("missing stage profile start".into()))?;
                assert_eq!(span.id,id);
                let elapsed=span.started.elapsed();
                state.stages.push(json!({"relation":id,"cpuNs":elapsed.cpu,"wallNs":elapsed.wall,"sampledVmSteps":this.vm()-span.vm}));
            }
            Ok(token)
        })
    }
    pub fn report(&self, graph: &Graph) -> Value {
        if !self.enabled {
            return Value::Null;
        }
        let mut state = self.state.lock().unwrap();
        if let Some(span) = state.active.take() {
            let elapsed = span.started.elapsed();
            state.stages.push(json!({"relation":span.id,"cpuNs":elapsed.cpu,"wallNs":elapsed.wall,"sampledVmSteps":self.vm()-span.vm,"incomplete":true}));
        }
        state
            .stages
            .sort_by_key(|v| std::cmp::Reverse(v["cpuNs"].as_u64().unwrap_or(0)));
        state
            .publications
            .sort_by_key(|v| std::cmp::Reverse(v["rows"].as_u64().unwrap_or(0)));
        for stage in &mut state.stages {
            let id = stage["relation"].as_u64().unwrap() as usize;
            if let Some(relation) = graph.relations.get(id) {
                stage["sql"] = json!(relation.body);
                stage["columns"] =
                    json!(relation.columns.iter().map(|c| &c.name).collect::<Vec<_>>());
                stage["dependencies"] = json!(graph.dependencies(&relation.body));
            }
        }
        json!({"stagesByCpu":state.stages,"publicationsByRows":state.publications,
            "sampleResolutionVmSteps":10000,"interpretation":"stage times include producer SQL, native leaf calls, publication, and release; copyCpuNs measures only final typed publication, not aggregate row boxing"})
    }
}
pub(super) fn explain(connection: &Connection, sql: &str) -> Value {
    let plan = (|| -> Result<Vec<Value>> {
        let mut statement = connection.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
        statement.query_map([],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"parent":r.get::<_,i64>(1)?,"detail":r.get::<_,String>(3)?})))?.collect()
    })();
    let bytecode = (|| -> Result<std::collections::BTreeMap<String, usize>> {
        let mut statement = connection.prepare(&format!("EXPLAIN {sql}"))?;
        let names = statement
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<Result<Vec<_>>>()?;
        let mut counts = std::collections::BTreeMap::new();
        for name in names {
            *counts.entry(name).or_default() += 1;
        }
        Ok(counts)
    })();
    match (plan, bytecode) {
        (Ok(plan), Ok(opcodes)) => {
            let flags: Vec<_> = plan
                .iter()
                .filter_map(|p| {
                    let d = p["detail"].as_str().unwrap();
                    (d.contains("CORRELATED")
                        || d.contains("TEMP B-TREE")
                        || d.contains("AUTOMATIC")
                        || d.contains("SCAN "))
                    .then(|| json!({"planId":p["id"],"detail":d}))
                })
                .collect();
            json!({"plan":plan,"opcodes":opcodes,"flags":flags})
        }
        (p, b) => json!({"error":format!("plan: {:?}; bytecode: {:?}",p.err(),b.err())}),
    }
}

/// Keep observations separate from proposed remedies. Loop cycles are inclusive, so rank them
/// individually; never add them together or use them as the stage CPU denominator.
pub(super) fn summarize(metrics: &mut Value, graph: &Graph, owners: &[(usize, String)]) {
    let mut labels = vec![std::collections::BTreeSet::new(); graph.relations.len()];
    for (root, label) in owners {
        let mut pending = vec![*root];
        let mut seen = std::collections::HashSet::new();
        while let Some(id) = pending.pop() {
            if id < labels.len() && seen.insert(id) {
                labels[id].insert(label.clone());
                pending.extend(graph.dependencies(&graph.relations[id].body));
            }
        }
    }
    if let Some(stages) = metrics["profile"]["stagesByCpu"].as_array_mut() {
        for stage in stages {
            if let Some(names) = stage["relation"]
                .as_u64()
                .and_then(|i| labels.get(i as usize))
            {
                stage["clauses"] = json!(names);
            }
        }
    }
    let mut hazards = Vec::new();
    for (statement, info) in metrics["statements"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        for item in info["scans"]["loops"].as_array().into_iter().flatten() {
            let flags = item["flags"].as_array().cloned().unwrap_or_default();
            if flags.is_empty() {
                continue;
            }
            let mut entry = item.clone();
            entry["statement"] = json!(statement);
            let action = if flags.iter().any(|f| f == "many_empty_probes") {
                "Compare a narrow predicate set joined by file/node against this repeated existence probe. Check anchor cardinality before reversing the join."
            } else if flags.iter().any(|f| f == "repeated_scan") {
                "Check correlation and input grain. Materialize a reused set or add a keyed semijoin before this scan."
            } else {
                "Correct virtual-table cardinality estimates or pin the smaller input during SQL generation; verify the resulting join order."
            };
            entry["suggestedExperiment"] = json!(action);
            hazards.push(entry);
        }
    }
    hazards.sort_by_key(|v| std::cmp::Reverse(v["hardwareCycles"].as_i64().unwrap_or(0)));
    hazards.truncate(12);
    let stages: Vec<_> = metrics["profile"]["stagesByCpu"]
        .as_array()
        .into_iter()
        .flatten()
        .take(8)
        .cloned()
        .collect();
    let publications: Vec<_> = metrics["profile"]["publicationsByRows"]
        .as_array()
        .into_iter()
        .flatten()
        .take(8)
        .cloned()
        .collect();
    let prepare = metrics["prepareCpuNs"].as_u64().unwrap_or(0);
    let execution = metrics["cpuNs"].as_u64().unwrap_or(0);
    metrics["actionable"] = json!({
        "prepareDominates":prepare>execution,
        "prepareShare":prepare as f64/(prepare+execution).max(1) as f64,
        "prepareExperiment":"Compare smaller dependency-expansion budgets and a few statement phases. Reuse installed prepared programs across calls.",
        "hottestStages":stages,"largestPublications":publications,"hazards":hazards,
        "interpretation":"Suggested experiments are hypotheses. Stage CPU is measured; sampled stage VM is approximate. Loop cycles are inclusive. Full plans and counters remain in metrics."
    });
}
