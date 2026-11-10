//! The engine's text tools without Node (list step 18; P `spec-tooling.md` §2.2.1–2.2.2): the
//! `rerail-engine` binary (`src/bin/rerail-engine.rs`) and the napi wrappers both call these.

pub use super::logic::profile::watch;
pub use super::sql_executor::CodeRuleSqlExecutor;
use crate::code_rule_dsl::CodeRuleDslRuntime;
use crate::code_rule_dsl::transducer::packed::rir;
mod signing;

/// The package CLI's SQL tool, sharing the standalone command's request and execution path.
#[napi_derive::napi(catch_unwind)]
pub fn execute_sql_request(store_path: String, request_json: String) -> napi::Result<String> {
    sql(std::path::Path::new(&store_path), &request_json).map_err(napi::Error::from_reason)
}

/// A SQL extension request for the standalone tool. The same session serves the bindings,
/// lowered rule program and subsequent SQL queries; no AST matcher is lowered to SQL.
pub fn sql(store: &std::path::Path, request: &str) -> Result<String, String> {
    let profiling = super::logic::profile::on();
    let _run = profiling.then(super::logic::profile::one_run);
    if profiling { super::logic::profile::reset(); }
    let span = super::logic::profile::span(||"SQL extension request".into());
    let result = sql_request(store,request);
    drop(span);
    if profiling {
        if let Some(directory) = super::logic::profile::directory() {
            super::logic::profile::write(&directory,"SQL extension request").map_err(|e|e.to_string())?;
        }
    }
    result
}

fn sql_request(store: &std::path::Path, request: &str) -> Result<String, String> {
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
        #[serde(default)]
        file_changes: Vec<serde_json::Value>,
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
        for batch in request.file_changes {
            updates.push(
                session
                    .apply_file_changes(batch.to_string(), request.max_waves)
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
            // A program the reader passes still has to type: each variable one domain (a kind, a name, …), as a run
            // checks when it plans (`plan::lower::domains`). A run would stop there, so the check does too.
            Ok((program, _)) if let Err(error) = super::logic::rr::print(&program)
                .map_err(|e| e.to_string())
                .and_then(|text| super::logic::plan::corpus::combine(&[text.as_str()], "").map_err(|e| e.to_string()))
                // Pruned first, as a run is: a library's relation the outputs do not read is not planned.
                .and_then(|combined| super::logic::plan::lower::domains(&super::logic::front::prune(combined)).map(|_| ()).map_err(|e| e.to_string())) =>
            {
                serde_json::json!({"path": path, "status": "types-failed", "error": error})
            }
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
        &with_reuse(options),
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

/// Several `.rr` policies over one repository (O105): each runs over the documents of the language it names
/// (`.language python`, `rust`, `rr`), into its own `out/<k>`, through the one content store, so a file's tape is
/// made once whichever policy reads it. The findings of all of them, in the order of `paths`, and each run's
/// report.
pub fn run_policies(
    paths: &[std::path::PathBuf],
    root: &std::path::Path,
    out: &std::path::Path,
    options: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let (mut diagnostics, mut runs) = (Vec::new(), Vec::new());
    for (k, path) in paths.iter().enumerate() {
        let report = run_rr(path, root, &out.join(k.to_string()), options).map_err(|e| format!("{}: {e}", path.display()))?;
        diagnostics.extend(report["diagnostics"].as_array().cloned().unwrap_or_default());
        runs.push(serde_json::json!({"path": path.display().to_string(), "files": report["files"], "metadata": report["metadata"]}));
    }
    Ok(serde_json::json!({"diagnostics": diagnostics, "runs": runs}))
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
    program_texts(&[path]).map(|mut texts| texts.remove(0))
}

/// [`program_text`] for each of `paths`: phase 0 resolves all their `#include`s in one run of `bootstrap.rr`
/// (`plan::corpus::phase0`), not one run each.
pub fn program_texts(paths: &[&std::path::Path]) -> Result<Vec<String>, String> {
    use super::logic::plan::corpus::{observe, phase0};
    let read = |path: &std::path::Path| -> Result<String, String> {
        let (bytes, _) = observe(|o| o.read(path)).ok_or_else(|| format!("{}: cannot read the file", path.display()))?;
        String::from_utf8(bytes).map_err(|error| format!("{}: {error}", path.display()))
    };
    let texts: Vec<String> = paths.iter().map(|p| read(p)).collect::<Result<_, _>>()?;
    // A policy (`.policy`, `.language`) is read as `run-rr` reads it: its language's base relations are declared by
    // reading the file (O111), which a policy's text does not do itself.
    let policy = |text: &str| text.lines().map(str::trim_start).any(|l| l.starts_with(".policy") || l.starts_with(".language"));
    let includes = |text: &str| text.lines().any(|l| l.trim_start().starts_with("#include"));
    let mut seeds = Vec::new();
    for (path, text) in paths.iter().zip(&texts) {
        if includes(text) {
            seeds.push(phase0::Seed::File(path.canonicalize().map_err(|e| format!("{}: {e}", path.display()))?));
        }
    }
    let graphs = phase0::graphs(&seeds).map_err(|e| e.to_string())?;
    let mut graphs = graphs.into_iter();
    let mut out = Vec::with_capacity(texts.len());
    for (path, text) in paths.iter().zip(texts) {
        if !includes(&text) {
            out.push(if policy(&text) { format!("#include \"core.rr\"\n{text}") } else { text });
            continue;
        }
        let graph = graphs.next().expect("a graph per seed");
        // The program as modules (decision 0089): the library files it includes stay `#include`s, read once by a
        // run however many of its programs include them; a file beside it is written in. A policy reads `core.rr`.
        let canonical = path.canonicalize().map_err(|e| format!("{}: {e}", path.display()))?;
        let own = module_text(&canonical, &text, &graph, &mut Vec::new(), &read)?;
        let own = if policy(&text) && !includes(&own) { format!("#include \"core.rr\"\n{own}") } else { own };
        // The text's own graph is the file's, cut to what the text includes: kept, so no second run derives it.
        if let Ok(Some(seed)) = super::logic::rr::text_seed(&own) {
            phase0::keep_text(&seed, &graph);
        }
        out.push(own);
    }
    Ok(out)
}

/// `text` (the file `path`'s) with each `#include` phase 0 resolved to a file outside the library written in, each
/// once.
fn module_text(
    path: &std::path::Path,
    text: &str,
    graph: &super::logic::plan::corpus::phase0::Graph,
    seen: &mut Vec<std::path::PathBuf>,
    read: &dyn Fn(&std::path::Path) -> Result<String, String>,
) -> Result<String, String> {
    let library = super::logic::rr::library().canonicalize().map_err(|e| e.to_string())?;
    let from = path.to_string_lossy().into_owned();
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let name = line.trim_start().strip_prefix("#include").and_then(|rest| rest.split('"').nth(1));
        match name {
            Some(name) => {
                let found = graph.includes.get(&from).into_iter().flatten().find(|(_, n, _)| n == name).map(|(_, _, p)| p.clone());
                let Some(found) = found else {
                    return Err(format!("{}: #include {name:?}: no such file", path.display()));
                };
                if found.starts_with(&library) {
                    out.push_str(line);
                } else if !seen.contains(&found) {
                    seen.push(found.clone());
                    let inner = read(&found)?;
                    out.push_str(&module_text(&found, &inner, graph, seen, read)?);
                }
            }
            None => out.push_str(line),
        }
        out.push('\n');
    }
    Ok(out)
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
    // A text in module form (decision 0089) reads `core.rr`, which declares `env`.
    let declares = declares || program.lines().any(|l| l.trim_start().starts_with("#include"));
    if !declares || settings.is_empty() {
        return program.to_owned();
    }
    let quote = super::logic::rr::quote;
    let facts: String = settings.iter().map(|(k, v)| format!("env({}, {}).\n", quote(k), quote(v))).collect();
    format!("{program}\n// The run's settings (--env, rerail.env).\n{facts}")
}

/// The command line's runs keep their outputs for phase 0's cutoff (`corpus::reuse`), unless the caller said.
fn with_reuse(options: &serde_json::Value) -> serde_json::Value {
    let mut options = options.clone();
    if let Some(map) = options.as_object_mut() {
        map.entry("reuse").or_insert(serde_json::Value::Bool(true));
    }
    options
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
    let options = serde_json::json!({ "env": settings, "reuse": true });
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
    if report.prepared != "prepared" {
        text += if report.prepared == "store" { "  program prepared: read from the store\n" } else { "  program prepared: kept in memory\n" };
    }
    if report.memo.0 > 0 {
        text += &format!("  {} of {} files from the per-file memo\n", report.memo.0, report.files);
    }
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

// ---------------------------------------------------------------- the command line
//
//   rerail-engine dump --rule <file>... [--clause <id>] [--fused] [--setplan] [--capabilities <json>]
//   rerail-engine asm <file.rir>
//   rerail-engine logic <program.rr | dir of .rr> <corpus root> [<T facts.sqlite>] <out dir> [--modes <modes>] [--metal] [--chunk <nodes>] [--full]
//   rerail-engine gc <store.sqlite> [--history-mb <n>]
//   rerail-engine sql <store.sqlite> <request.json>
//   rerail-engine flatten <program.rr>
//   rerail-engine run-rr <policy.rr> <corpus root> <out dir> [<options.json>]
//   rerail-engine run-policies <corpus root> <out dir> <policy.rr>...
//   rerail-engine check-rr <rule>... | convert-rr <rule>...
use std::path::Path;

const USAGE: &str = "usage:\n  rerail-engine dump --rule <file>... [--clause <id>] [--fused] [--setplan] [--capabilities <file>]\n  rerail-engine asm <file.rir>\n  rerail-engine logic <program.rr | dir of .rr> <corpus root> [<T facts.sqlite>] <out dir> [--modes <modes> | --sql] [--metal] [--chunk <nodes>] [--env key=value]... [--full] [--facts <dir>] [--require-facts] [--base <rev>]\n  rerail-engine facts-stamp <raw dir> --out <facts dir> --root <repository> <policy.rr>...\n  rerail-engine gc <store.sqlite> [--history-mb <n>]\n  rerail-engine sql <store.sqlite> <request.json>\n  rerail-engine sign-ed25519 <request.json>\n  rerail-engine flatten <program.rr>\n  rerail-engine run-rr <policy.rr> <corpus root> <out dir> [<options.json>]\n  rerail-engine run-policies <corpus root> <out dir> <policy.rr>...\n  rerail-engine check-rr <rule>...\n  rerail-engine convert-rr <rule>...";

/// One `rerail-engine` command (`args` after the program's name): its output, or the error to report. The binary
/// runs one per process; a long-lived instance (the browser's engine: `reactor`) runs one per call, and with
/// `RERAIL_SESSION` (a directory) keeps a corpus session across them (decision 0063).
pub fn cli(args: &[String]) -> Result<String, String> {
    // However the command ends, the cost samples it made are written (decision 0100 §14).
    struct Flush;
    impl Drop for Flush {
        fn drop(&mut self) {
            crate::code_rule_dsl::logic::costs::flush();
        }
    }
    let _flush = Flush;
    if let Some(dir) = std::env::var_os("RERAIL_SESSION") {
        crate::code_rule_dsl::logic::plan::corpus::session::start(std::path::Path::new(&dir))?;
    }
    match args.first().map(String::as_str) {
        Some("sign-ed25519") => match &args[1..] {
            [request] => signing::run(&read(request)?),
            _ => Err("usage: rerail-engine sign-ed25519 <request.json>".into()),
        },
        #[cfg(feature = "dump-cli")]
        Some("dump") => dump_command(&args[1..]),
        Some("asm") => match &args[1..] {
            [file] => crate::tools::assemble(&read(file)?),
            _ => Err(USAGE.into()),
        },
        Some("sql") => match &args[1..] {
            [store, request] => crate::tools::sql(Path::new(store), &read(request)?),
            _ => Err("usage: rerail-engine sql <store.sqlite> <request.json>".into()),
        },
        Some("logic") => logic_command(&args[1..]),
        Some("facts-stamp") => facts_stamp_command(&args[1..]),
        Some("run-rr") if args.len() == 4 || args.len() == 5 => {
            let options = if args.len() == 5 { serde_json::from_str(&read(&args[4])?).map_err(|e| e.to_string())? } else { serde_json::Value::Null };
            serde_json::to_string(&crate::tools::run_rr(Path::new(&args[1]), Path::new(&args[2]), Path::new(&args[3]), &options)?).map_err(|e| e.to_string())
        }
        // Several policies over one repository, each over its own language's files (O105).
        Some("run-policies") if args.len() > 3 => {
            let paths = args[3..].iter().map(std::path::PathBuf::from).collect::<Vec<_>>();
            serde_json::to_string(&crate::tools::run_policies(&paths, Path::new(&args[1]), Path::new(&args[2]), &serde_json::Value::Null)?).map_err(|e| e.to_string())
        }
        Some("check-rr") if args.len() > 1 => {
            let paths = args[1..].iter().map(std::path::PathBuf::from).collect::<Vec<_>>();
            let report = crate::tools::check_rr(&paths);
            let failures = report.as_array().expect("rule reports").iter().filter(|row| row["status"] == "reader-failed").count();
            let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
            if failures > 0 {
                println!("{text}");
                Err(format!("{failures} rules failed the .rr reader"))
            } else { Ok(text) }
        }
        Some("convert-rr") if args.len() > 1 => {
            let paths = args[1..].iter().map(std::path::PathBuf::from).collect::<Vec<_>>();
            serde_json::to_string_pretty(&crate::tools::convert_rr(&paths)).map_err(|e| e.to_string())
        }
        // The program `logic` runs for a file: `core.rr` and its includes resolved, lowered and printed as `.rr`.
        Some("flatten") => match &args[1..] {
            [file] => crate::tools::flatten(Path::new(file.as_str())),
            _ => Err(USAGE.into()),
        },
        #[cfg(feature = "tape-cache")]
        Some("gc") => gc_command(&args[1..]),
        #[cfg(not(all(feature = "dump-cli", feature = "tape-cache")))]
        Some(command @ ("dump" | "gc")) => Err(format!("{command} is not in this build (its feature, `dump-cli` or `tape-cache`, is off)")),
        _ => Err(USAGE.into()),
    }
}

/// `logic`: a program over a corpus, its outputs as T's (list step 23).
fn logic_command(args: &[String]) -> Result<String, String> {
    let (mut positional, mut modes, mut metal, mut chunk) = (Vec::new(), None, false, 1_usize << 18);
    let (mut sql, mut env, mut full) = (false, Vec::new(), false);
    let mut args = args.iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--sql" => sql = true,
            "--metal" => metal = true,
            // A full run in a session: compiled and evaluated afresh, as without one (decision 0063).
            "--full" => full = true,
            "--modes" => modes = Some(args.next().ok_or("--modes requires a value")?.clone()),
            "--env" => env.push(args.next().ok_or("--env requires key=value")?.clone()),
            "--chunk" => chunk = args.next().ok_or("--chunk requires a value")?.parse().map_err(|_| "--chunk takes a node count")?,
            // Where `artifact=` inputs are read (data-layer/RUN-FACTS.md section 2); `<root>/.rerail/facts` without it.
            "--facts" => crate::code_rule_dsl::logic::plan::corpus::inputs::set_facts_dir(args.next().ok_or("--facts requires a directory")?.into())?,
            // In CI: a missing, stale or malformed artifact fails the run (RUN-FACTS.md section 5).
            "--require-facts" => crate::code_rule_dsl::logic::plan::corpus::inputs::require_facts(),
            // The revision `at="base"` inputs are read at: a commit id or a reference name.
            "--base" => crate::code_rule_dsl::logic::plan::corpus::inputs::set_base(args.next().ok_or("--base requires a revision")?.clone())?,
            other => positional.push(other.to_owned()),
        }
    }
    let modes = match (sql, modes) {
        (true, Some(_)) => return Err("--sql cannot be combined with --modes".into()),
        (true, None) => String::new(),
        (false, modes) => modes.unwrap_or_else(|| "program".into()),
    };
    // `<program> <corpus> <out>` on the typing programs' own ingestion, or `<program> <corpus> <T facts> <out>` to
    // reproduce T (its statistics and node ids).
    let (program, root, facts, out) = match positional.as_slice() {
        [program, root, out] => (program, root, None, out),
        [program, root, facts, out] => (program, root, Some(Path::new(facts.as_str())), out),
        _ => return Err(USAGE.into()),
    };
    // A directory: every `.rr` in it, run as one program (each file parsed once for all of them); each file's
    // `#include`s are read in.
    let programs = if Path::new(program.as_str()).is_dir() {
        let mut files: Vec<_> = std::fs::read_dir(program)
            .map_err(|e| format!("{program}: {e}"))?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| path.extension().is_some_and(|x| x == "rr"))
            .collect();
        files.sort();
        // A file another one `#include`s is a library of it (`…confusables.rr`), not a program of its own: alone it
        // has neither the declarations it reads nor the rules that fill its inputs.
        let included: std::collections::BTreeSet<std::ffi::OsString> = files
            .iter()
            .filter_map(|path| std::fs::read_to_string(path).ok())
            .flat_map(|text| {
                text.lines()
                    .filter_map(|l| l.trim().strip_prefix("#include"))
                    .filter_map(|rest| rest.trim().strip_prefix('"')?.strip_suffix('"').map(|p| Path::new(p).file_name().map(ToOwned::to_owned)))
                    .flatten()
                    .collect::<Vec<_>>()
            })
            .collect();
        files.retain(|path| !path.file_name().is_some_and(|n| included.contains(n)));
        crate::tools::program_texts(&files.iter().map(|p| p.as_ref()).collect::<Vec<&std::path::Path>>())?
    } else {
        vec![crate::tools::program_text(Path::new(program.as_str()))?]
    };
    if programs.is_empty() {
        return Err(format!("{program} has no .rr programs"));
    }
    let run = || crate::tools::logic(&programs, Path::new(root), facts, Path::new(out), &modes, metal, chunk, &env);
    if full { crate::code_rule_dsl::logic::plan::corpus::session::full(run) } else { run() }
}

/// `facts-stamp <raw> --out <dir> --root <repository> <policy.rr>…` (data-layer/RUN-FACTS.md section 1, `rerail facts
/// stamp`): wraps each producer's output with the digest of the inputs the policies declare for it.
fn facts_stamp_command(args: &[String]) -> Result<String, String> {
    use crate::code_rule_dsl::logic::plan::corpus::inputs;
    let (mut raw, mut out, mut root, mut policies) = (None, None, None, Vec::new());
    let mut args = args.iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--out" => out = Some(args.next().ok_or("--out requires a directory")?.clone()),
            "--root" => root = Some(args.next().ok_or("--root requires a directory")?.clone()),
            other if raw.is_none() => raw = Some(other.to_owned()),
            other => policies.push(other.to_owned()),
        }
    }
    let (Some(raw), Some(out)) = (raw, out) else { return Err(USAGE.into()) };
    let root = root.unwrap_or_else(|| ".".into());
    // The declarations, from each policy with its `#include`s read in: `.artifact` lines only.
    let mut settings = Vec::new();
    for policy in &policies {
        let text = crate::tools::program_text(Path::new(policy))?;
        settings.extend(text.lines().filter_map(|l| l.trim().strip_prefix(".artifact ")).map(|rest| ("artifact".to_owned(), rest.to_owned())));
    }
    let declared = inputs::artifacts(&settings).map_err(|e| e.to_string())?;
    inputs::stamp(Path::new(&raw), Path::new(&out), Path::new(&root), &declared)
}

/// `gc`: collects a store until nothing is left to collect (decision 0053): converts a store made before 0053 to
/// incremental vacuum, decides which corpus runs to keep (`--history-mb`: the high-water mark, default 512; the low
/// water is three quarters of it), and runs passes until two in a row find nothing.
#[cfg(feature = "tape-cache")]
fn gc_command(args: &[String]) -> Result<String, String> {
    use crate::content_store::{ContentStore, CorpusPolicy};
    let (mut path, mut policy) = (None, CorpusPolicy::default());
    let mut args = args.iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--history-mb" => {
                let mb: u64 = args.next().ok_or("--history-mb requires a value")?.parse().map_err(|_| "--history-mb takes a number")?;
                (policy.high_water, policy.low_water) = (mb << 20, (mb << 20) / 4 * 3);
            }
            other if path.is_none() => path = Some(other.to_owned()),
            _ => return Err(USAGE.into()),
        }
    }
    let path = path.ok_or(USAGE)?;
    if !Path::new(&path).exists() {
        return Err(format!("{path}: no such store"));
    }
    let e = |e: crate::content_store::StoreError| e.to_string();
    let store = ContentStore::open(Path::new(&path)).map_err(e)?;
    let before = store.capacity().map_err(e)?.bytes;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let mut out = String::new();
    if store.convert_to_incremental_vacuum().map_err(e)? {
        out += "converted to incremental vacuum (a store made before decision 0053)\n";
    }
    let kept = store.keep_corpus_runs(&policy, now).map_err(e)?;
    out += &format!(
        "corpus runs: {} manifests kept, {} evicted; history {:.1} MB (high water {} MB), protected {:.1} MB\n",
        kept.manifests,
        kept.evicted,
        kept.history_bytes as f64 / 1048576.0,
        policy.high_water >> 20,
        kept.protected_bytes as f64 / 1048576.0
    );
    let (mut idle, mut passes, mut tapes, mut sources, mut plan_rows) = (0, 0, 0, 0, 0);
    while idle < 2 {
        let pass = store.collect_corpus_pass(&policy, now).map_err(e)?;
        passes += 1;
        idle = if pass.did_work() { 0 } else { idle + 1 };
        (tapes, sources, plan_rows) = (tapes + pass.collection.tapes, sources + pass.collection.sources, plan_rows + pass.plan_rows);
    }
    let after = store.capacity().map_err(e)?.bytes;
    out += &format!(
        "{passes} passes: {tapes} tapes, {sources} sources and {plan_rows} memo rows of dead plans deleted; {:.1} MB -> {:.1} MB\n",
        before as f64 / 1048576.0,
        after as f64 / 1048576.0
    );
    Ok(out)
}

#[cfg(feature = "dump-cli")]
fn dump_command(args: &[String]) -> Result<String, String> {
    let (mut rules, mut clause, mut capabilities) = (Vec::new(), None, None);
    let (mut fused, mut setplan) = (false, false);
    let mut args = args.iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--fused" => fused = true,
            "--setplan" => setplan = true,
            "--rule" | "--clause" | "--capabilities" => {
                let value = args.next().ok_or_else(|| format!("{argument} requires a value"))?;
                match argument.as_str() {
                    "--rule" => rules.push(value.clone()),
                    "--clause" => clause = Some(value.clone()),
                    _ => capabilities = Some(read(value)?),
                }
            }
            other => return Err(format!("unknown dump option `{other}`\n{USAGE}")),
        }
    }
    if rules.is_empty() {
        return Err(format!("dump needs at least one --rule <file>\n{USAGE}"));
    }
    let policies = rules.iter().map(|file| policy(file)).collect::<Result<Vec<_>, _>>()?;
    let capabilities = capabilities.unwrap_or_else(|| crate::tools::CAPABILITIES.to_string());
    crate::tools::dump(&policies, &capabilities, clause.as_deref(), fused, setplan)
}

/// A rule file as (name, source, format): named by its stem, its format from the extension.
#[cfg(feature = "dump-cli")]
fn policy(file: &str) -> Result<(String, String, String), String> {
    let path = Path::new(file);
    let format = match path.extension().and_then(|extension| extension.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("json") => "json",
        Some("yaml" | "yml") => "yaml",
        _ => return Err(format!("{file}: a rule file is .json, .yaml or .yml")),
    };
    let name = path.file_stem().and_then(|stem| stem.to_str()).unwrap_or(file).to_string();
    Ok((name, read(file)?, format.to_string()))
}

fn read(file: &str) -> Result<String, String> {
    std::fs::read_to_string(file).map_err(|error| format!("{file}: {error}"))
}
