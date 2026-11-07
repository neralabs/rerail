//! `rerail-engine`: the engine's tools without Node (P `spec-tooling.md` §2.7).
//!
//!   rerail-engine dump --rule <file>... [--clause <id>] [--fused] [--setplan] [--capabilities <json>]
//!   rerail-engine asm <file.rir>
//!   rerail-engine logic <program.rr | dir of .rr> <corpus root> [<T facts.sqlite>] <out dir> [--modes <modes>] [--metal] [--chunk <nodes>]
//!   rerail-engine gc <store.sqlite> [--history-mb <n>]
//!   rerail-engine flatten <program.rr>
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match rerail_engine::tools::cli(&args) {
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
