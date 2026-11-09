//! Complete SQL-session observations for the independent replay verifier.
//! Build with --features headless. The oracle runs separately without SQL capture.
use rerail_engine::tools::CodeRuleSqlExecutor;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

#[path = "sql_session_verify/documents.rs"]
mod documents;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

fn snapshot(
    session: &CodeRuleSqlExecutor,
    execution: &Value,
    facts: &BTreeMap<String, Value>,
    digest: &str,
    check: usize,
) -> Result<Value> {
    let documents = session.verification_files().is_some();
    let types = session.verification_types();
    let mut tables = BTreeMap::<String, Vec<String>>::new();
    for (name, fact) in facts {
        tables.insert(
            name.clone(),
            fact["columns"]
                .as_array()
                .ok_or("missing fact columns")?
                .iter()
                .map(|c| c.as_str().unwrap().into())
                .collect(),
        );
    }
    for relation in execution["program"]["relations"]
        .as_array()
        .ok_or("missing derived relations")?
    {
        let name = relation[0].as_str().ok_or("missing relation name")?;
        let width = relation[1].as_u64().ok_or("missing relation arity")? as usize;
        tables.insert(name.into(), (0..width).map(|i| format!("c{i}")).collect());
    }
    for lattice in execution["program"]["lattices"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let name = lattice[0].as_str().unwrap();
        let ops = &execution["program"]["lattice_operations"][name];
        if !ops["after"].is_null() && !ops["widen"].is_null() {
            let width = lattice[1].as_u64().unwrap() as usize + 1;
            tables.insert(
                format!("__sql_changes_{name}"),
                (0..width - 1)
                    .map(|i| format!("c{i}"))
                    .chain(["changes".into()])
                    .collect(),
            );
        }
    }
    if documents {
        tables.extend(session.verification_inputs());
    }
    let width = tables.values().map(Vec::len).max().unwrap_or(1);
    let mut result = BTreeMap::<String, Value>::new();
    let mut arms = vec![];
    for (id, (name, columns)) in tables.iter().enumerate() {
        let values = (0..width)
            .map(|c| columns.get(c).map_or_else(|| "NULL".into(), |c| quote(c)))
            .collect::<Vec<_>>()
            .join(",");
        arms.push(format!("SELECT {id},{values} FROM {}", quote(name)));
        result.insert(name.clone(), json!({"columns":columns,"rows":[]}));
    }
    if !arms.is_empty() {
        let sql = format!(
            "{} /* rerail-verification:{digest}:{check} */",
            arms.join(" UNION ALL ")
        );
        let names = tables.keys().collect::<Vec<_>>();
        for row in session.verification_rows(&sql)? {
            let name = names[usize::try_from(row[0].ok_or("missing relation tag")?)?];
            let cells = row[1..=tables[name].len()]
                .iter()
                .enumerate()
                .map(|(column, v)| {
                    let ty = types
                        .get(name)
                        .and_then(|c| c.get(column))
                        .map_or("int", String::as_str);
                    if documents || matches!(ty, "sym" | "symbol") {
                        session.verification_cell(ty, *v).map_err(|e| e.into())
                    } else {
                        Ok(v.map(|n| json!(n.to_string())).unwrap_or(Value::Null))
                    }
                })
                .collect::<Result<Vec<_>>>()?;
            result.get_mut(name).unwrap()["rows"]
                .as_array_mut()
                .unwrap()
                .push(json!(cells));
        }
    } else {
        session.verification_rows(&format!(
            "SELECT 1 /* rerail-verification:{digest}:{check} */"
        ))?;
    }
    Ok(json!(result))
}

fn error_kind(error: &str) -> &'static str {
    let error = error.to_lowercase();
    if error.contains("fixpoint") {
        "wave-limit"
    } else if error.contains("interrupt")
        || error.contains("execution limit")
        || error.contains("step limit")
    {
        "execution-limit"
    } else if error.contains("overflow") || error.contains("out of range") {
        "integer-overflow"
    } else {
        "engine-error"
    }
}

fn fresh(
    path: &Path,
    facts: &BTreeMap<String, Value>,
    documents: Option<&Value>,
    request: &Value,
    waves: Option<u32>,
    limits: &Value,
    oracle: bool,
) -> Result<(CodeRuleSqlExecutor, std::result::Result<Value, String>)> {
    let mut session = CodeRuleSqlExecutor::new(path.to_string_lossy().into())?;
    if oracle {
        session.use_cold_oracle();
    }
    if let Some(snapshot) = documents {
        session.bind_snapshot(snapshot.to_string())?;
    }
    for fact in facts.values() {
        session.bind_relation(fact.to_string())?;
    }
    session.set_limits(limits.to_string())?;
    let result = session
        .run(
            request["program"].as_str().ok_or("missing program")?.into(),
            request["dialect"].as_str().unwrap_or("python").into(),
            waves,
        )
        .map_err(|e| e.to_string())
        .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()));
    Ok((session, result))
}

fn changed(facts: &BTreeMap<String, Value>, patches: &Value) -> Result<BTreeMap<String, Value>> {
    let mut facts = facts.clone();
    for patch in patches.as_array().ok_or("changes must be an array")? {
        let name = patch["name"].as_str().ok_or("missing patch name")?;
        let fact = facts
            .get_mut(name)
            .ok_or("patch names an unbound relation")?;
        let mut rows = fact["rows"].as_array().ok_or("missing rows")?.clone();
        for row in patch["delete"].as_array().into_iter().flatten() {
            rows.retain(|r| r != row);
        }
        for row in patch["insert"].as_array().into_iter().flatten() {
            if !rows.contains(row) {
                rows.push(row.clone());
            }
        }
        fact["rows"] = json!(rows);
    }
    Ok(facts)
}

fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 3 || !matches!(args[0].as_str(), "retained" | "oracle") {
        return Err(
            "usage: sql-session-verify retained|oracle STORE_DIRECTORY REQUEST.json".into(),
        );
    }
    let oracle = args[0] == "oracle";
    if oracle && std::env::var_os("RERAIL_SQL_PROFILE_DIR").is_some() {
        return Err("the independent oracle must run without SQL capture".into());
    }
    let bytes = std::fs::read(&args[2])?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let request: Value = serde_json::from_slice(&bytes)?;
    // Every requested action must have replay coverage. Unknown fields cannot
    // silently turn a partial execution into a complete parity claim.
    let allowed = [
        "program",
        "dialect",
        "relations",
        "snapshot",
        "steps",
        "changes",
        "max_waves",
        "queries",
    ];
    for name in request
        .as_object()
        .ok_or("request must be an object")?
        .keys()
    {
        if !allowed.contains(&name.as_str()) {
            return Err(format!("unsupported verification input: {name}").into());
        }
    }
    let directory = Path::new(&args[1]);
    std::fs::create_dir_all(directory)?;
    let mut facts = BTreeMap::new();
    for fact in request["relations"].as_array().into_iter().flatten() {
        let name = fact["name"]
            .as_str()
            .ok_or("missing fact name")?
            .to_string();
        if facts.insert(name, fact.clone()).is_some() {
            return Err("duplicate input relation".into());
        }
    }
    let waves = request["max_waves"]
        .as_u64()
        .map(|n| u32::try_from(n))
        .transpose()?;
    let mut documents = request.get("snapshot").cloned();
    let (mut session, execution) = fresh(
        &directory.join("initial.sqlite"),
        &facts,
        documents.as_ref(),
        &request,
        waves,
        &json!({}),
        oracle,
    )?;
    let mut execution = execution.map_err(std::io::Error::other)?;
    execution["program"]["relations"] = json!(session.verification_relations()?);
    let mut checks = vec![
        json!({"id":0,"outcome":"ok","state":snapshot(&session,&execution,&facts,&digest,0)?,"documents":session.verification_files()}),
    ];
    let steps = request["steps"].as_array().cloned().unwrap_or_else(|| {
        request["changes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| json!({"changes":c}))
            .collect()
    });
    if request.get("steps").is_some() && request.get("changes").is_some() {
        return Err("use either steps or changes".into());
    }
    for (at, step) in steps.iter().enumerate() {
        for name in step.as_object().ok_or("step must be an object")?.keys() {
            if ![
                "changes",
                "snapshot",
                "file_changes",
                "limits",
                "max_waves",
                "expected_error",
            ]
            .contains(&name.as_str())
            {
                return Err(format!("unsupported verification step: {name}").into());
            }
        }
        let id = at + 1;
        let limits = step.get("limits").cloned().unwrap_or_else(|| json!({}));
        let limit = step["max_waves"]
            .as_u64()
            .map(|n| u32::try_from(n))
            .transpose()?
            .or(waves);
        if ["changes", "snapshot", "file_changes"]
            .iter()
            .filter(|k| step.get(**k).is_some())
            .count()
            != 1
        {
            return Err("each step requires exactly one publication action".into());
        }
        let candidate = if step.get("changes").is_some() {
            changed(&facts, &step["changes"])?
        } else {
            facts.clone()
        };
        let candidate_documents = documents::changed(&documents, step)?;
        let (result, new) = if oracle {
            match fresh(
                &directory.join(format!("step-{id}.sqlite")),
                &candidate,
                candidate_documents.as_ref(),
                &request,
                limit,
                &limits,
                true,
            ) {
                Ok((next, result)) => (result, Some(next)),
                Err(e) => (Err(e.to_string()), None),
            }
        } else {
            session.set_limits(limits.to_string())?;
            let result = if let Some(snapshot) = step.get("snapshot") {
                session.apply_snapshot(snapshot.to_string(), limit)
            } else if let Some(files) = step.get("file_changes") {
                session.apply_file_changes(files.to_string(), limit)
            } else {
                session.apply_changes(step["changes"].to_string(), limit)
            };
            (
                result
                    .map_err(|e| e.to_string())
                    .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string())),
                None,
            )
        };
        // Snapshot after the budget guard drops, including the old facts on failure.
        let (outcome, message, report) = match result {
            Ok(report) => {
                facts = candidate;
                documents = candidate_documents;
                if let Some(next) = new {
                    session = next;
                }
                ("ok", None, Some(report))
            }
            Err(error) => (error_kind(&error), Some(error), None),
        };
        session.set_limits("{}".into())?;
        checks.push(json!({"id":id,"outcome":outcome,"error":message,"report":if oracle {None}else{report},"state":snapshot(&session,&execution,&facts,&digest,id)?,"documents":session.verification_files()}));
    }
    let mut queries = vec![];
    for query in request["queries"].as_array().into_iter().flatten() {
        if let Some(object) = query.as_object() {
            if object
                .keys()
                .any(|name| !["sql", "parameters"].contains(&name.as_str()))
            {
                return Err("unsupported verification query field".into());
            }
        }
        let sql = query
            .as_str()
            .or_else(|| query["sql"].as_str())
            .ok_or("missing query text")?;
        let result = session.query(
            sql.into(),
            query.get("parameters").map(Value::to_string),
            Some(u32::MAX),
        )?;
        queries.push(serde_json::from_str::<Value>(&result)?);
    }
    println!(
        "{}",
        json!({"format":"rerail.sql-session-observation/v1","requestSha256":digest,"mode":args[0],"runnerSha256":format!("{:x}",Sha256::digest(std::fs::read(std::env::current_exe()?)?)),"inputs":session.verification_inputs(),"relations":execution["program"]["relations"],"outputs":execution["outputs"],"checks":checks,"queries":queries})
    );
    Ok(())
}
