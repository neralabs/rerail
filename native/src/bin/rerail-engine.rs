//! `rerail-engine`: the engine's tools without Node (P `spec-tooling.md` §2.7).
//!
//!   rerail-engine dump --rule <file>... [--clause <id>] [--fused] [--setplan] [--capabilities <json>]
//!   rerail-engine asm <file.rir>
//!   rerail-engine logic <program.rr | dir of .rr> <corpus root> [<T facts.sqlite>] <out dir> [--modes <modes> | --sql] [--metal] [--chunk <nodes>]
//!   rerail-engine gc <store.sqlite> [--history-mb <n>]
//!   rerail-engine sql <store.sqlite> <request.json>
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "usage:\n  rerail-engine dump --rule <file>... [--clause <id>] [--fused] [--setplan] [--capabilities <file>]\n  rerail-engine asm <file.rir>\n  rerail-engine logic <program.rr | dir of .rr> <corpus root> [<T facts.sqlite>] <out dir> [--modes <modes> | --sql] [--metal] [--chunk <nodes>] [--env key=value]...\n  rerail-engine gc <store.sqlite> [--history-mb <n>]\n  rerail-engine sql <store.sqlite> <request.json>";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(text) => {
            print!("{text}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("rerail-engine: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<String, String> {
    match args.first().map(String::as_str) {
        Some("dump") => dump(&args[1..]),
        Some("asm") => match &args[1..] {
            [file] => rerail_engine::tools::assemble(&read(file)?),
            _ => Err(USAGE.into()),
        },
        Some("sql") => match &args[1..] {
            [store, request] => rerail_engine::tools::sql(Path::new(store), &read(request)?),
            _ => Err("usage: rerail-engine sql <store.sqlite> <request.json>".into()),
        },
        Some("logic") => logic(&args[1..]),
        Some("run-rr") if args.len() == 4 || args.len() == 5 => {
            let options = if args.len() == 5 { serde_json::from_str(&read(&args[4])?).map_err(|e| e.to_string())? } else { serde_json::Value::Null };
            serde_json::to_string(&rerail_engine::tools::run_rr(Path::new(&args[1]), Path::new(&args[2]), Path::new(&args[3]), &options)?).map_err(|e| e.to_string())
        }
        Some("check-rr") if args.len() > 1 => {
            let paths = args[1..].iter().map(std::path::PathBuf::from).collect::<Vec<_>>();
            let report = rerail_engine::tools::check_rr(&paths);
            let failures = report.as_array().expect("rule reports").iter().filter(|row| row["status"] == "reader-failed").count();
            let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
            if failures > 0 {
                println!("{text}");
                Err(format!("{failures} rules failed the .rr reader"))
            } else { Ok(text) }
        }
        Some("convert-rr") if args.len() > 1 => {
            let paths = args[1..].iter().map(std::path::PathBuf::from).collect::<Vec<_>>();
            serde_json::to_string_pretty(&rerail_engine::tools::convert_rr(&paths)).map_err(|e| e.to_string())
        }
        Some("gc") => gc(&args[1..]),
        _ => Err(USAGE.into()),
    }
}

/// `logic`: a program over a corpus, its outputs as T's (list step 23).
fn logic(args: &[String]) -> Result<String, String> {
    let (mut positional, mut modes, mut metal, mut chunk) = (Vec::new(), None, false, 1_usize << 18);
    let (mut sql, mut env) = (false, Vec::new());
    let mut args = args.iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--sql" => sql = true,
            "--metal" => metal = true,
            "--modes" => modes = Some(args.next().ok_or("--modes requires a value")?.clone()),
            "--env" => env.push(args.next().ok_or("--env requires key=value")?.clone()),
            "--chunk" => chunk = args.next().ok_or("--chunk requires a value")?.parse().map_err(|_| "--chunk takes a node count")?,
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
        files.iter().map(|path| rerail_engine::tools::program_text(path)).collect::<Result<Vec<_>, _>>()?
    } else {
        vec![rerail_engine::tools::program_text(Path::new(program.as_str()))?]
    };
    if programs.is_empty() {
        return Err(format!("{program} has no .rr programs"));
    }
    let settings = rerail_engine::tools::settings(Path::new(root.as_str()), &env)?;
    let programs: Vec<String> = programs.iter().map(|p| rerail_engine::tools::with_settings(p, &settings)).collect();
    rerail_engine::tools::logic(&programs, Path::new(root), facts, Path::new(out), &modes, metal, chunk)
}

/// `gc`: collects a store until nothing is left to collect (decision 0053): converts a store made before 0053 to
/// incremental vacuum, decides which corpus runs to keep (`--history-mb`: the high-water mark, default 512; the low
/// water is three quarters of it), and runs passes until two in a row find nothing.
fn gc(args: &[String]) -> Result<String, String> {
    use rerail_engine::content_store::{ContentStore, CorpusPolicy};
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
    let e = |e: rerail_engine::content_store::StoreError| e.to_string();
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

fn dump(args: &[String]) -> Result<String, String> {
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
    let capabilities = capabilities.unwrap_or_else(|| rerail_engine::tools::CAPABILITIES.to_string());
    rerail_engine::tools::dump(&policies, &capabilities, clause.as_deref(), fused, setplan)
}

/// A rule file as (name, source, format): named by its stem, its format from the extension.
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
