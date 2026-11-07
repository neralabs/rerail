//! `rerail-engine`: the engine's tools without Node (P `spec-tooling.md` §2.7).
//!
//!   rerail-engine dump --rule <file>... [--clause <id>] [--fused] [--setplan] [--capabilities <json>]
//!   rerail-engine asm <file.rir>
//!   rerail-engine logic <program.rr> <corpus root> [<T facts.sqlite>] <out dir> [--modes <modes>] [--metal] [--chunk <nodes>]
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "usage:\n  rerail-engine dump --rule <file>... [--clause <id>] [--fused] [--setplan] [--capabilities <file>]\n  rerail-engine asm <file.rir>\n  rerail-engine logic <program.rr> <corpus root> [<T facts.sqlite>] <out dir> [--modes <modes>] [--metal] [--chunk <nodes>]";

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
        Some("logic") => logic(&args[1..]),
        _ => Err(USAGE.into()),
    }
}

/// `logic`: a program over a corpus, its outputs as T's (list step 23).
fn logic(args: &[String]) -> Result<String, String> {
    let (mut positional, mut modes, mut metal, mut chunk) = (Vec::new(), "program".to_owned(), false, 1_usize << 18);
    let mut args = args.iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--metal" => metal = true,
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
    rerail_engine::tools::logic(&rerail_engine::tools::program_text(Path::new(program.as_str()))?, Path::new(root), facts, Path::new(out), &modes, metal, chunk)
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
