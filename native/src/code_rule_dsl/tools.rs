//! The engine's text tools without Node (list step 18; P `spec-tooling.md` §2.2.1–2.2.2): the
//! `rerail-engine` binary (`src/bin/rerail-engine.rs`) and the napi wrappers both call these.

pub use super::sql_executor::CodeRuleSqlExecutor;
use crate::code_rule_dsl::CodeRuleDslRuntime;
use crate::code_rule_dsl::transducer::packed::rir;

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
        #[serde(default)]
        snapshots: Vec<serde_json::Value>,
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Index {
        relation: String,
        columns: Vec<u32>,
    }
    let request: Request = serde_json::from_str(request).map_err(|e| e.to_string())?;
    if request.snapshot.is_some() && request.stored.is_some() {
        return Err("a SQL request selects either stored documents or a snapshot".into());
    }
    let mut session = CodeRuleSqlExecutor::new(store.to_string_lossy().into_owned())
        .map_err(|e| e.to_string())?;
    let result = (|| -> napi::Result<serde_json::Value> {
        if let Some(limits) = request.limits {
            session.set_limits(limits.to_string())?;
        }
        if let Some(snapshot) = request.snapshot {
            session.bind_snapshot(snapshot.to_string())?;
        }
        if let Some(stored) = request.stored {
            session.bind_stored(stored.scope, stored.paths)?;
        }
        for relation in request.relations {
            session.bind_relation(relation.to_string())?;
        }
        let execution = request
            .program
            .map(|p| {
                session.run(
                    p,
                    request.dialect.unwrap_or_else(|| "python".into()),
                    request.max_waves,
                )
            })
            .transpose()?;
        for index in request.indexes {
            session.create_index(index.relation, index.columns)?;
        }
        let mut updates = request
            .changes
            .into_iter()
            .map(|batch| {
                session
                    .apply_changes(
                        serde_json::Value::Array(batch).to_string(),
                        request.max_waves,
                    )
                    .and_then(|s| {
                        serde_json::from_str::<serde_json::Value>(&s)
                            .map_err(|e| napi::Error::from_reason(e.to_string()))
                    })
            })
            .collect::<napi::Result<Vec<_>>>()?;
        for snapshot in request.snapshots {
            updates.push(
                session
                    .apply_snapshot(snapshot.to_string(), request.max_waves)
                    .and_then(|s| {
                        serde_json::from_str::<serde_json::Value>(&s)
                            .map_err(|e| napi::Error::from_reason(e.to_string()))
                    })?,
            );
        }
        let queries = request
            .queries
            .into_iter()
            .map(|query| {
                let result = session.query(
                    query.sql,
                    Some(
                        serde_json::to_string(&query.parameters)
                            .map_err(|e| napi::Error::from_reason(e.to_string()))?,
                    ),
                    query.max_rows,
                )?;
                serde_json::from_str::<serde_json::Value>(&result)
                    .map_err(|e| napi::Error::from_reason(e.to_string()))
            })
            .collect::<napi::Result<Vec<_>>>()?;
        let execution = execution
            .map(|s| serde_json::from_str::<serde_json::Value>(&s))
            .transpose()
            .map_err(|e| napi::Error::from_reason(e.to_string()))?;
        Ok(serde_json::json!({"execution": execution, "updates":updates, "queries": queries}))
    })()
    .map_err(|e| e.to_string())?;
    serde_json::to_string_pretty(&result)
        .map(|s| s + "\n")
        .map_err(|e| e.to_string())
}

/// The capabilities rules compile against, as bundled with the engine.
pub const CAPABILITIES: &str = include_str!("capabilities.json");

/// Read a public `.rr` file, including its core prelude and libraries. The CLI and migration
/// harness use the same reader as the engine; they never strip directives or run a lab interpreter.
pub fn read_rr(path: &std::path::Path) -> Result<String, String> {
    let program = super::logic::rr::parse_file(path).map_err(|e| e.to_string())?;
    super::logic::rr::print(&program).map_err(|e| e.to_string())
}

/// Convert whole AST policies for review. Unsupported clauses refuse the entire candidate.
pub fn convert_rr(paths: &[std::path::PathBuf]) -> serde_json::Value {
    serde_json::Value::Array(paths.iter().map(|path| {
        match std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|source| super::logic::convert::policy(&source)) {
            Ok(mut report) => { report["path"] = serde_json::json!(path); report },
            Err(error) => serde_json::json!({"path": path, "status": "conversion-failed", "error": error})
        }
    }).collect())
}

/// Check every requested rule and report every refusal. A reader success is not execution parity.
pub fn check_rr(paths: &[std::path::PathBuf]) -> serde_json::Value {
    serde_json::Value::Array(paths.iter().map(|path| {
        match super::logic::rr::read_file(path) {
            Ok((program, files)) => serde_json::json!({
                "path": path, "status": "reader-passed", "rules": program.rules.len(),
                "outputs": program.outputs,
                "files": files.iter().map(|file| serde_json::json!({
                    "path": file.path,
                    "sha256": file.sha256.map(|hash| hash.iter().map(|b| format!("{b:02x}")).collect::<String>())
                })).collect::<Vec<_>>()
            }),
            Err(error) => serde_json::json!({"path": path, "status": "reader-failed", "error": error.to_string()})
        }
    }).collect())
}

/// One strict `.rr` replay. The caller supplies a corpus and typed policy options; findings are
/// produced by the native relational engine, including the original message and parameters.
pub fn run_rr(
    path: &std::path::Path,
    root: &std::path::Path,
    out: &std::path::Path,
    options: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    use super::{
        logic::{plan::corpus, rr},
        typing::file_local::Binding,
    };
    let (program, files) = rr::read_file(path).map_err(|e| e.to_string())?;
    let metadata = rr::metadata(&program).map_err(|e| e.to_string())?;
    if !program.outputs.iter().any(|name| name == "rr_diagnostic") {
        return Err(format!("{}: no .violation declarations", path.display()));
    }
    let source = rr::print(&program).map_err(|e| e.to_string())?;
    let report = corpus::run_with_options(
        &[source.as_str()],
        root,
        None,
        out,
        "program",
        Binding::Cpu,
        1 << 18,
        &Default::default(),
        options,
    )
    .map_err(|e| e.to_string())?;
    let rows = match std::fs::read_to_string(out.join("rr_diagnostic.csv")) {
        Ok(text) => text
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e.to_string()),
    };
    Ok(
        serde_json::json!({"diagnostics": rows, "files": report.files, "nodes": report.nodes,
                         "metadata": metadata,
                         "programInputs": files.iter().map(|file| serde_json::json!({
                             "path": file.path,
                             "sha256": file.sha256.map(|hash| hash.iter().map(|b| format!("{b:02x}")).collect::<String>())
                         })).collect::<Vec<_>>(),
                         "phases": report.phases.iter().map(|(phase,user,system,rss,statements)| serde_json::json!({
                             "phase": phase, "userCpuSeconds": user, "systemCpuSeconds": system,
                             "peakRssMB": rss, "submittedStatements": statements
                         })).collect::<Vec<_>>()}),
    )
}

/// `policies` ((name, source, format `json`/`yaml`)) lowered with no store and dumped as `rir`:
/// every clause, or the one named `clause`; `fused` adds packs and groups, `setplan` the set plan.
pub fn dump(
    policies: &[(String, String, String)],
    capabilities: &str,
    clause: Option<&str>,
    fused: bool,
    setplan: bool,
) -> Result<String, String> {
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
    Ok(programs
        .iter()
        .map(|(program, grammar)| rir::disassemble(program, grammar.as_ref()))
        .collect())
}

/// A program file's `.rr` text, with what it `#include`s from the library (`docs/rule-syntax/lib`) read in.
pub fn program_text(path: &std::path::Path) -> Result<String, String> {
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if !text.lines().any(|l| l.trim_start().starts_with("#include")) {
        return Ok(text);
    }
    let program = super::logic::rr::parse_file(path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    super::logic::rr::print(&program).map_err(|error| format!("{}: {error}", path.display()))
}

/// The run's settings (t21 E4): `key=value` lines of `<root>/rerail.env` (blank lines and `#` comments
/// skipped), then each `--env key=value`, a later value for a key replacing an earlier one.
pub fn settings(root: &std::path::Path, flags: &[String]) -> Result<Vec<(String, String)>, String> {
    let file = root.join("rerail.env");
    let text = if file.exists() { std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))? } else { String::new() };
    let lines = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(|l| (l, file.display().to_string()));
    let mut settings: Vec<(String, String)> = Vec::new();
    for (line, from) in lines.chain(flags.iter().map(|f| (f.as_str(), "--env".to_owned()))) {
        let (key, value) = line.split_once('=').ok_or_else(|| format!("{from}: `{line}` is not key=value"))?;
        let (key, value) = (key.trim().to_owned(), value.trim().to_owned());
        settings.retain(|(k, _)| *k != key);
        settings.push((key, value));
    }
    Ok(settings)
}

/// `program` with the run's settings as `env(key, value)` facts, for a program that declares `env`; any other
/// program is returned as it is. The facts are part of the program's text, so the plan, and every memo keyed by
/// it, is per setting.
pub fn with_settings(program: &str, settings: &[(String, String)]) -> String {
    let declares = program.lines().any(|l| {
        let l = l.trim_start();
        l.strip_prefix(".decl").is_some_and(|r| r.trim_start().strip_prefix("env").is_some_and(|r| r.trim_start().starts_with('(')))
    });
    if !declares || settings.is_empty() {
        return program.to_owned();
    }
    let quote = super::logic::rr::quote;
    let facts: String = settings.iter().map(|(k, v)| format!("env({}, {}).\n", quote(k), quote(v))).collect();
    format!("{program}\n// The run's settings (--env, rerail.env).\n{facts}")
}

/// [`program_text`] under the name the wasm build's callers use: a rule as written in the library, its includes
/// and `core.rr` resolved, as one program's text.
pub fn flatten(path: &std::path::Path) -> Result<String, String> {
    program_text(path)
}

/// `programs` (`.rr` texts) over the corpus T's `t_facts` names under `root` (list step 23): the outputs as T's
/// `rdl.py sql` writes them, into `out`, and a report of each phase's CPU and the peak RSS. Several programs run as
/// one (decision 0051): each file is parsed and batched once for all of them, and `violation` holds every one's
/// findings. T's facts reproduce one program's run only.
/// `settings` are `--env key=value`s ([`settings`]).
#[allow(clippy::too_many_arguments)]
pub fn logic(programs: &[String], root: &std::path::Path, t_facts: Option<&std::path::Path>, out: &std::path::Path, modes: &str, metal: bool, chunk_nodes: usize, settings: &[String]) -> Result<String, String> {
    use super::typing::file_local::Binding;
    let binding = if metal { Binding::Metal } else { Binding::Cpu };
    if programs.len() > 1 && t_facts.is_some() {
        return Err("T facts reproduce one program's run, not several".into());
    }
    let options = serde_json::json!({ "env": settings });
    let programs: Vec<&str> = programs.iter().map(String::as_str).collect();
    let report = super::logic::plan::corpus::run_with_options(&programs, root, t_facts, out, modes, binding, chunk_nodes, &super::logic::plan::corpus::Typing::default(), &options)
        .map_err(|error| error.to_string())?;
    let execution = if modes.is_empty() { "sql" } else { modes };
    let mut text = format!(
        "{} programs; {} files, {} nodes in {} chunks, {} components native ({execution}, nearest on {binding:?})\n",
        programs.len(),
        report.files,
        report.nodes,
        report.chunks,
        report.native
    );
    for (phase, user, sys, rss, statements) in &report.phases {
        text += &format!(
            "  {phase:<6} {user:.2} s user + {sys:.2} s sys; peak RSS {rss:.0} MB; {statements} SQLite statements\n"
        );
    }
    for (name, _) in &report.typing_runs {
        text += &format!("  typing program {name} run first, for the relations it gives\n");
    }
    for (rel, rows) in &report.rows {
        text += &format!("  {rel}: {rows} rows\n");
    }
    if t_facts.is_some() {
        text += "  (planner statistics and node ids read from the T facts file)\n";
    }
    Ok(text)
}

#[cfg(test)]
mod settings_tests {
    #[test]
    fn settings_become_env_facts_for_a_program_that_declares_env() {
        let root = std::env::temp_dir().join(format!("rerail-settings-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("rerail.env"), "# comment\npython.version = 3.12\nquote=a\"b\n").unwrap();
        let settings = super::settings(&root, &["python.version=3.13".into()]).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(settings, vec![("quote".into(), "a\"b".into()), ("python.version".into(), "3.13".into())]);
        let program = ".decl env(key: symbol, value: symbol)\n";
        let with = super::with_settings(program, &settings);
        assert!(with.contains("env(\"quote\", \"a\\\"b\").") && with.contains("env(\"python.version\", \"3.13\")."), "{with}");
        // A program that does not declare `env` is unchanged.
        assert_eq!(super::with_settings(".decl envy(x: symbol)\n", &settings), ".decl envy(x: symbol)\n");
    }
}
