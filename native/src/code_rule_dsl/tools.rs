//! The engine's text tools without Node (list step 18; P `spec-tooling.md` §2.2.1–2.2.2): the
//! `rerail-engine` binary (`src/bin/rerail-engine.rs`) and the napi wrappers both call these.

use crate::code_rule_dsl::CodeRuleDslRuntime;
use crate::code_rule_dsl::transducer::packed::rir;

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

/// A program file's `.rr` text, with what it `#include`s from the library (`docs/rule-syntax/lib`) read in.
pub fn program_text(path: &std::path::Path) -> Result<String, String> {
    let text = std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if !text.lines().any(|l| l.trim_start().starts_with("#include")) {
        return Ok(text);
    }
    let program = super::logic::rr::parse_file(path).map_err(|error| format!("{}: {error}", path.display()))?;
    super::logic::rr::print(&program).map_err(|error| format!("{}: {error}", path.display()))
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
    let mut text = format!("{} programs; {} files, {} nodes in {} chunks, {} components native ({modes}, nearest on {binding:?})\n", programs.len(), report.files, report.nodes, report.chunks, report.native);
    for (phase, user, sys, rss, statements) in &report.phases {
        text += &format!("  {phase:<6} {user:.2} s user + {sys:.2} s sys; peak RSS {rss:.0} MB; {statements} SQLite statements\n");
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
