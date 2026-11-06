//! The engine's text tools without Node (list step 18; P `spec-tooling.md` §2.2.1–2.2.2): the
//! `rerail-engine` binary (`src/bin/rerail-engine.rs`) and the napi wrappers both call these.

use crate::code_rule_dsl::CodeRuleDslRuntime;
use crate::code_rule_dsl::transducer::packed::rir;

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
pub fn run_rr(path: &std::path::Path, root: &std::path::Path, out: &std::path::Path, options: &serde_json::Value) -> Result<serde_json::Value, String> {
    use super::{logic::{rr, plan::corpus}, typing::file_local::Binding};
    let (program, files) = rr::read_file(path).map_err(|e| e.to_string())?;
    if !program.outputs.iter().any(|name| name == "rr_diagnostic") {
        return Err(format!("{}: no .violation declarations", path.display()));
    }
    let source = rr::print(&program).map_err(|e| e.to_string())?;
    let report = corpus::run_with_options(&source, root, None, out, "program", Binding::Cpu, 1 << 18, &Default::default(), options).map_err(|e| e.to_string())?;
    let rows = match std::fs::read_to_string(out.join("rr_diagnostic.csv")) {
        Ok(text) => text.lines().map(|line| serde_json::from_str::<serde_json::Value>(line).map_err(|e| e.to_string())).collect::<Result<Vec<_>, _>>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e.to_string()),
    };
    Ok(serde_json::json!({"diagnostics": rows, "files": report.files, "nodes": report.nodes,
                         "programInputs": files.iter().map(|file| serde_json::json!({
                             "path": file.path,
                             "sha256": file.sha256.map(|hash| hash.iter().map(|b| format!("{b:02x}")).collect::<String>())
                         })).collect::<Vec<_>>(),
                         "phases": report.phases.iter().map(|(phase,user,system,rss,statements)| serde_json::json!({
                             "phase": phase, "userCpuSeconds": user, "systemCpuSeconds": system,
                             "peakRssMB": rss, "submittedStatements": statements
                         })).collect::<Vec<_>>()}))
}

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
/// one (decision 0049): each file is parsed and batched once for all of them, and `violation` holds every one's
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
    let mut text = format!("{} programs; {} files, {} nodes in {} chunks, {} components native ({modes}, nearest on {binding:?})\n", programs.len(), report.files, report.nodes, report.chunks, report.native);
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
