
/// Evaluate a standalone fact program through Rerail's planner and executor.
/// Output cells retain the types declared by the program.
pub fn evaluate_facts(source: &str, tracing: bool) -> Result<serde_json::Value, String> {
    use super::logic::{front, plan, rr};
    rr::reset_rule_sources();
    let program = front::prune(rr::parse(source).map_err(|e| e.to_string())?);
    let (outcome, events) = plan::run_facts(&program, tracing).map_err(|e| e.to_string())?;
    let mut outputs = serde_json::Map::new();
    for (name, rows) in outcome.outputs {
        let columns = &program.decls[&name];
        let rows: Result<Vec<Vec<serde_json::Value>>, String> = rows.iter().map(|row| {
            row.split('\t').zip(columns).map(|(cell, (_, ty))| {
                if matches!(ty.as_str(), "int" | "node" | "flat") {
                    cell.parse::<i64>().map(serde_json::Value::from).map_err(|e| e.to_string())
                } else {
                    Ok(serde_json::Value::from(cell))
                }
            }).collect()
        }).collect();
        outputs.insert(name, serde_json::json!(rows?));
    }
    let evaluation = if tracing {
        let mut sources = rr::take_rule_sources();
        let catalog: Vec<_> = program.rules.iter().enumerate().map(|(id, rule)| {
            let reads: Vec<_> = rule.body.iter().filter_map(|literal| {
                match literal {
                    front::Literal::Atom(atom) => Some(serde_json::json!({"relation": atom.rel, "kind": if atom.neg { "negative" } else { "positive" }})),
                    front::Literal::Agg { atom, .. } => Some(serde_json::json!({"relation": atom.rel, "kind": "aggregate"})),
                    _ => None,
                }
            }).collect();
            let source_line = sources.get_mut(&rule.to_string()).and_then(|lines| lines.pop_front());
            serde_json::json!({"id": id, "head": rule.head.rel, "text": rule.to_string(), "reads": reads, "source_line": source_line})
        }).collect();
        let seeds: serde_json::Map<String, serde_json::Value> = program.facts.iter().map(|(name, rows)| {
            let rows: Vec<Vec<serde_json::Value>> = rows.iter().map(|row| row.iter().map(|cell| match cell {
                front::Value::Int(n) => serde_json::json!(n),
                front::Value::Str(s) => serde_json::json!(s),
            }).collect()).collect();
            (name.clone(), serde_json::json!(rows))
        }).collect();
        Some(serde_json::json!({
            "backend": "Rerail SQL executor", "strategy": "memoized", "cache": "exact input fact sets",
            "rules": catalog, "seeds": seeds, "events": events,
        }))
    } else { None };
    Ok(serde_json::json!({"outputs": outputs, "evaluation": evaluation}))
}
