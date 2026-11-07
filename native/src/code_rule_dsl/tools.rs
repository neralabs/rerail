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

/// A `.rr` file with `core.rr` and everything it includes, lowered and printed back as one program's text, which
/// `logic` reads: a rule as written in the library (`#include "python/…"`) runs as the programs `logic` takes do.
pub fn flatten(path: &std::path::Path) -> Result<String, String> {
    let program = super::logic::rr::parse_file(path).map_err(|error| error.to_string())?;
    super::logic::rr::print(&program).map_err(|error| error.to_string())
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
    if report.memo_hits > 0 {
        text += &format!("  {} of {} files from the per-file memo\n", report.memo_hits, report.files);
    }
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

// ---------------------------------------------------------------- the command line
//
//   rerail-engine dump --rule <file>... [--clause <id>] [--fused] [--setplan] [--capabilities <json>]
//   rerail-engine asm <file.rir>
//   rerail-engine logic <program.rr | dir of .rr> <corpus root> [<T facts.sqlite>] <out dir> [--modes <modes>] [--metal] [--chunk <nodes>] [--full]
//   rerail-engine gc <store.sqlite> [--history-mb <n>]
//   rerail-engine flatten <program.rr>
use std::path::Path;

const USAGE: &str = "usage:\n  rerail-engine dump --rule <file>... [--clause <id>] [--fused] [--setplan] [--capabilities <file>]\n  rerail-engine asm <file.rir>\n  rerail-engine logic <program.rr | dir of .rr> <corpus root> [<T facts.sqlite>] <out dir> [--modes <modes>] [--metal] [--chunk <nodes>] [--full]\n  rerail-engine gc <store.sqlite> [--history-mb <n>]\n  rerail-engine flatten <program.rr>";

/// One `rerail-engine` command (`args` after the program's name): its output, or the error to report. The binary
/// runs one per process; a long-lived instance (the browser's engine: `reactor`) runs one per call, and with
/// `RERAIL_SESSION` (a directory) keeps a corpus session across them (decision 0056).
pub fn cli(args: &[String]) -> Result<String, String> {
    if let Some(dir) = std::env::var_os("RERAIL_SESSION") {
        crate::code_rule_dsl::logic::plan::corpus::session::start(std::path::Path::new(&dir))?;
    }
    match args.first().map(String::as_str) {
        #[cfg(feature = "dump-cli")]
        Some("dump") => dump_command(&args[1..]),
        Some("asm") => match &args[1..] {
            [file] => crate::tools::assemble(&read(file)?),
            _ => Err(USAGE.into()),
        },
        Some("logic") => logic_command(&args[1..]),
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
    let (mut positional, mut modes, mut metal, mut chunk, mut full) = (Vec::new(), "program".to_owned(), false, 1_usize << 18, false);
    let mut args = args.iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--metal" => metal = true,
            // A full run in a session: compiled and evaluated afresh, as without one (decision 0056).
            "--full" => full = true,
            "--modes" => modes = args.next().ok_or("--modes requires a value")?.clone(),
            "--chunk" => chunk = args.next().ok_or("--chunk requires a value")?.parse().map_err(|_| "--chunk takes a node count")?,
            other => positional.push(other.to_owned()),
        }
    }
    // `<program> <corpus> <out>` on the typing programs' own ingestion, or `<program> <corpus> <T facts> <out>` to
    // reproduce T (its statistics and node ids).
    let (program, root, facts, out) = match positional.as_slice() {
        [program, root, out] => (program, root, None, out),
        [program, root, facts, out] => (program, root, Some(Path::new(facts.as_str())), out),
        _ => return Err(USAGE.into()),
    };
    // A directory: every `.rr` in it, run as one program (each file parsed once for all of them).
    let programs = if Path::new(program.as_str()).is_dir() {
        let mut files: Vec<_> = std::fs::read_dir(program)
            .map_err(|e| format!("{program}: {e}"))?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| path.extension().is_some_and(|x| x == "rr"))
            .collect();
        files.sort();
        files.iter().map(|path| read(&path.to_string_lossy())).collect::<Result<Vec<_>, _>>()?
    } else if read(program)?.lines().any(|line| line.trim_start().starts_with("#include")) {
        // A rule as written in the library: its includes and `core.rr` resolved, lowered to one program.
        vec![crate::tools::flatten(Path::new(program.as_str()))?]
    } else {
        vec![read(program)?]
    };
    if programs.is_empty() {
        return Err(format!("{program} has no .rr programs"));
    }
    let run = || crate::tools::logic(&programs, Path::new(root), facts, Path::new(out), &modes, metal, chunk);
    if full { crate::code_rule_dsl::logic::plan::corpus::session::full(run) } else { run() }
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
