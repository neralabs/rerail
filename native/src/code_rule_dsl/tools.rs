//! The engine's text tools without Node (list step 18; P `spec-tooling.md` §2.2.1–2.2.2): the
//! `rerail-engine` binary (`src/bin/rerail-engine.rs`) and the napi wrappers both call these.

use crate::code_rule_dsl::CodeRuleDslRuntime;
use crate::code_rule_dsl::transducer::packed::rir;
pub use super::sql_executor::CodeRuleSqlExecutor;

/// The package CLI's SQL tool, sharing the standalone command's request and execution path.
#[napi_derive::napi(catch_unwind)]
pub fn execute_sql_request(store_path: String, request_json: String) -> napi::Result<String> {
    sql(std::path::Path::new(&store_path), &request_json).map_err(napi::Error::from_reason)
}

/// A SQL extension request for the standalone tool. The same session serves the bindings,
/// lowered rule program and subsequent SQL queries; no AST matcher is lowered to SQL.
pub fn sql(store: &std::path::Path, request: &str) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Query {
        sql: String,
        #[serde(default)]
        parameters: Vec<serde_json::Value>,
        max_rows: Option<u32>,
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Stored {
        scope: String,
        paths: Option<Vec<String>>,
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        snapshot: Option<serde_json::Value>,
        stored: Option<Stored>,
        #[serde(default)]
        relations: Vec<serde_json::Value>,
        program: Option<String>,
        dialect: Option<String>,
        max_waves: Option<u32>,
        limits: Option<serde_json::Value>,
        #[serde(default)]
        indexes: Vec<Index>,
        #[serde(default)]
        queries: Vec<Query>,
        #[serde(default)]
        changes: Vec<Vec<serde_json::Value>>,
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Index { relation: String, columns: Vec<u32> }
    let request: Request = serde_json::from_str(request).map_err(|e| e.to_string())?;
    if request.snapshot.is_some() && request.stored.is_some() {
        return Err("a SQL request selects either stored documents or a snapshot".into());
    }
    let mut session = CodeRuleSqlExecutor::new(store.to_string_lossy().into_owned()).map_err(|e| e.to_string())?;
    let result = (|| -> napi::Result<serde_json::Value> {
        if let Some(limits) = request.limits {session.set_limits(limits.to_string())?;}
        if let Some(snapshot) = request.snapshot {
            session.bind_snapshot(snapshot.to_string())?;
        }
        if let Some(stored) = request.stored {
            session.bind_stored(stored.scope, stored.paths)?;
        }
        for relation in request.relations {
            session.bind_relation(relation.to_string())?;
        }
        let execution = request.program.map(|p| session.run(p, request.dialect.unwrap_or_else(|| "python".into()), request.max_waves)).transpose()?;
        for index in request.indexes {session.create_index(index.relation,index.columns)?;}
        let updates = request.changes.into_iter().map(|batch| {
            session.apply_changes(serde_json::Value::Array(batch).to_string(),request.max_waves).and_then(|s|serde_json::from_str::<serde_json::Value>(&s).map_err(|e|napi::Error::from_reason(e.to_string())))
        }).collect::<napi::Result<Vec<_>>>()?;
        let queries = request.queries.into_iter().map(|query| {
            let result = session.query(query.sql, Some(serde_json::to_string(&query.parameters).map_err(|e| napi::Error::from_reason(e.to_string()))?), query.max_rows)?;
            serde_json::from_str::<serde_json::Value>(&result).map_err(|e| napi::Error::from_reason(e.to_string()))
        }).collect::<napi::Result<Vec<_>>>()?;
        let execution = execution.map(|s| serde_json::from_str::<serde_json::Value>(&s)).transpose().map_err(|e| napi::Error::from_reason(e.to_string()))?;
        Ok(serde_json::json!({"execution": execution, "updates":updates, "queries": queries}))
    })().map_err(|e| e.to_string())?;
    serde_json::to_string_pretty(&result).map(|s| s + "\n").map_err(|e| e.to_string())
}

/// The capabilities rules compile against, as bundled with the engine.
pub const CAPABILITIES: &str = include_str!("capabilities.json");

/// `policies` ((name, source, format `json`/`yaml`)) lowered with no store and dumped as `rir`:
/// every clause, or the one named `clause`; `fused` adds packs and groups, `setplan` the set plan.
pub fn dump(policies: &[(String, String, String)], capabilities: &str, clause: Option<&str>, fused: bool, setplan: bool) -> Result<String, String> {
    let compiled = policies
        .iter()
        .map(|(name, source, format)| {
            CodeRuleDslRuntime::compile(source.clone(), format.clone(), capabilities.to_string())
                .map(|rule| (name.clone(), rule))
                .map_err(|error| format!("{name}: {}", error.reason))
        })
        .collect::<Result<Vec<_>, _>>()?;
    super::slice::dump(&compiled, clause, fused, setplan).map_err(|error| error.to_string())
}

/// Every `program` block of `text` assembled through the verifier and dumped again by its own
/// grammar, or the first refusal.
pub fn assemble(text: &str) -> Result<String, String> {
    let programs = rir::assemble_all(text)?;
    Ok(programs.iter().map(|(program, grammar)| rir::disassemble(program, grammar.as_ref())).collect())
}

/// `programs` (`.rr` texts) over the corpus T's `t_facts` names under `root` (list step 23): the outputs as T's
/// `rdl.py sql` writes them, into `out`, and a report of each phase's CPU and the peak RSS. Several programs run as
/// one (decision 0051): each file is parsed and batched once for all of them, and `violation` holds every one's
/// findings. T's facts reproduce one program's run only.
pub fn logic(programs: &[String], root: &std::path::Path, t_facts: Option<&std::path::Path>, out: &std::path::Path, modes: &str, metal: bool, chunk_nodes: usize) -> Result<String, String> {
    use super::typing::file_local::Binding;
    let binding = if metal { Binding::Metal } else { Binding::Cpu };
    let report = match programs {
        [one] => super::logic::plan::corpus::run(one, root, t_facts, out, modes, binding, chunk_nodes),
        _ if t_facts.is_some() => return Err("T facts reproduce one program's run, not several".into()),
        _ => super::logic::plan::corpus::run_all(&programs.iter().map(String::as_str).collect::<Vec<_>>(), root, out, modes, binding, chunk_nodes),
    }
    .map_err(|error| error.to_string())?;
    let execution = if modes.is_empty() { "sql" } else { modes };
    let mut text = format!("{} programs; {} files, {} nodes in {} chunks, {} components native ({execution}, nearest on {binding:?})\n", programs.len(), report.files, report.nodes, report.chunks, report.native);
    for (phase, user, sys, rss, statements) in &report.phases {
        text += &format!("  {phase:<6} {user:.2} s user + {sys:.2} s sys; peak RSS {rss:.0} MB; {statements} SQLite statements\n");
    }
    for (rel, rows) in &report.rows {
        text += &format!("  {rel}: {rows} rows\n");
    }
    if t_facts.is_some() {
        text += "  (planner statistics and node ids read from the T facts file)\n";
    }
    Ok(text)
}
