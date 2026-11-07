
// Optional observer around the real SQL rule insertions. No application rules live here.
thread_local! {
    static FACT_CACHE: std::cell::RefCell<(String, std::collections::BTreeMap<String, CachedFactRule>)> = const { std::cell::RefCell::new((String::new(), std::collections::BTreeMap::new())) };
    static FACT_EVENTS: std::cell::RefCell<Option<Vec<serde_json::Value>>> = const { std::cell::RefCell::new(None) };
}

#[derive(Clone)]
struct CachedFactRule {
    inputs: Vec<(String, Vec<String>)>,
    candidates: Vec<serde_json::Value>,
}

struct FactAttempt {
    rule: usize,
    head: String,
    before: std::collections::BTreeMap<String, serde_json::Value>,
    candidates: Vec<serde_json::Value>,
    started: std::time::Instant,
    cache_hit: bool,
    previous_candidates: Vec<serde_json::Value>,
}

fn fact_rows(db: &Connection, query: &str) -> Result<Vec<serde_json::Value>> {
    let mut statement = db.prepare(query)?;
    let width = statement.column_count();
    let rows = statement.query_map([], |row| {
        let cells: rusqlite::Result<Vec<serde_json::Value>> = (0..width).map(|i| {
            Ok(match row.get::<_, SqlValue>(i)? {
                SqlValue::Integer(n) => serde_json::json!(n),
                SqlValue::Real(n) => serde_json::json!(n),
                SqlValue::Text(s) => serde_json::json!(s),
                SqlValue::Null => serde_json::Value::Null,
                SqlValue::Blob(b) => serde_json::json!(b),
            })
        }).collect();
        Ok(serde_json::json!(cells?))
    })?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn fact_row_set(rows: Vec<serde_json::Value>) -> std::collections::BTreeMap<String, serde_json::Value> {
    rows.into_iter().map(|row| (row.to_string(), row)).collect()
}

// Cache eligibility is limited to standalone, deterministic set rules. Each hit compares
// complete typed input sets, including negative and aggregate inputs; never counts alone.
fn facts_run_rule(
    db: &Connection, prog: &Program, rule_id: usize, rule: &Rule, select: &str,
    evaluate: impl FnOnce() -> Result<usize>,
) -> Result<usize> {
    let eligible = prog.inputs.is_empty() && prog.lattice.is_empty() && prog.natives.is_empty();
    let reads: BTreeSet<_> = rule.body.iter().filter_map(|literal| match literal {
        Literal::Atom(atom) | Literal::Agg { atom, .. } => Some(atom.rel.as_str()),
        _ => None,
    }).collect();
    let mut inputs = Vec::new();
    if eligible {
        for relation in reads {
            inputs.push((relation.to_owned(), fact_row_set(fact_rows(db, &format!("SELECT * FROM {relation}"))?).into_keys().collect()));
        }
    }
    let identity = rule.to_string();
    let previous = if eligible {
        FACT_CACHE.with(|cache| cache.borrow().1.get(&identity).cloned())
    } else { None };
    let tracing = FACT_EVENTS.with(|events| events.borrow().is_some());
    let previous_candidates = if tracing { previous.as_ref().map(|cached| cached.candidates.clone()).unwrap_or_default() } else { Vec::new() };
    let cached = previous.filter(|cached| cached.inputs == inputs);
    let before = if tracing { fact_row_set(fact_rows(db, &format!("SELECT * FROM {}", rule.head.rel))?) } else { Default::default() };
    let cache_hit = cached.is_some();
    let candidates = match cached {
        Some(cached) => cached.candidates,
        None if eligible || tracing => fact_row_set(fact_rows(db, select)?).into_values().collect(),
        None => Vec::new(),
    };
    let started = std::time::Instant::now();
    let inserted = if cache_hit {
        let rows = candidates.iter().map(|row| row.as_array().expect("a row").iter().map(|cell| match cell {
            serde_json::Value::Number(n) => SqlValue::Integer(n.as_i64().expect("integer fact")),
            serde_json::Value::String(s) => SqlValue::Text(s.clone()),
            _ => SqlValue::Null,
        }).collect()).collect();
        let before = db.total_changes();
        insert_many(db, &format!("INSERT OR IGNORE INTO {}", rule.head.rel), rows)?;
        (db.total_changes() - before) as usize
    } else {
        let inserted = evaluate()?;
        if eligible {
            FACT_CACHE.with(|cache| cache.borrow_mut().1.insert(identity, CachedFactRule { inputs, candidates: candidates.clone() }));
        }
        inserted
    };
    let attempt = tracing.then(|| FactAttempt { rule: rule_id, head: rule.head.rel.clone(), before, candidates, started, cache_hit, previous_candidates });
    facts_trace_end(db, attempt, inserted)?;
    Ok(inserted)
}

fn facts_trace_end(db: &Connection, attempt: Option<FactAttempt>, inserted: usize) -> Result<()> {
    let Some(attempt) = attempt else { return Ok(()) };
    let micros = attempt.started.elapsed().as_micros() as u64;
    let after = fact_row_set(fact_rows(db, &format!("SELECT * FROM {}", attempt.head))?);
    let added: Vec<_> = after.iter().filter(|(key, _)| !attempt.before.contains_key(*key)).map(|(_, value)| value).collect();
    let removed: Vec<_> = attempt.before.iter().filter(|(key, _)| !after.contains_key(*key)).map(|(_, value)| value).collect();
    let outcome = if attempt.cache_hit { "cached" } else if inserted > 0 { "changed" } else if attempt.candidates.is_empty() { "no_match" } else { "unchanged" };
    FACT_EVENTS.with(|events| {
        if let Some(events) = events.borrow_mut().as_mut() {
            events.push(serde_json::json!({
                "index": events.len(), "rule": attempt.rule, "head": attempt.head, "cache_hit": attempt.cache_hit, "evaluated": !attempt.cache_hit,
                "outcome": outcome, "inserted": inserted, "candidates": attempt.candidates,
                "added": added, "removed": removed, "micros": micros,
                "previous_candidates": attempt.previous_candidates,
            }));
        }
    });
    Ok(())
}

/// Standalone rules and literal facts, with no syntax corpus or host callbacks.
pub(in crate::code_rule_dsl) fn run_facts(prog: &Program, tracing: bool) -> Result<(Outcome, Vec<serde_json::Value>)> {
    let db = Connection::open_in_memory()?;
    db.execute_batch("PRAGMA temp_store = MEMORY;")?;
    let inputs = Inputs {
        facts: "main.".to_owned(), stats: "main.".to_owned(), atoms: None,
        native: BTreeSet::new(), batch: HashMap::new(), given: Some(Stats::empty(prog)), kept: BTreeSet::new(),
    };
    let identity = format!("{:?}\n{}", prog.decls, prog.rules.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"));
    FACT_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.0 != identity { *cache = (identity, Default::default()); }
    });
    FACT_EVENTS.with(|events| *events.borrow_mut() = tracing.then(Vec::new));
    let result = run(prog, &db, inputs, None);
    let events = FACT_EVENTS.with(|events| events.borrow_mut().take().unwrap_or_default());
    result.map(|outcome| (outcome, events))
}
