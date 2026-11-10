//! `rerail-engine`: the engine's tools without Node (P `spec-tooling.md` §2.7).
//!
//!   rerail-engine dump --rule <file>... [--clause <id>] [--fused] [--setplan] [--capabilities <json>]
//!   rerail-engine asm <file.rir>
//!   rerail-engine logic <program.rr | dir of .rr> <corpus root> [<T facts.sqlite>] <out dir> [--modes <modes> | --sql] [--metal] [--chunk <nodes>]
//!   rerail-engine gc <store.sqlite> [--history-mb <n>]
//!   rerail-engine sql <store.sqlite> <request.json>
//!   rerail-engine flatten <program.rr>
//!   rerail-engine run-rr | run-policies | check-rr | convert-rr ... (`tools::cli`)
use std::process::ExitCode;

fn main() -> ExitCode {
    // Before any thread starts: a stop signal or `RERAIL_DEADLINE` dumps the run first (decision 0084).
    rerail_engine::tools::watch();
    // Before any pool is made: the most threads the process uses (`RERAIL_THREADS`).
    rerail_engine::threads::configure();
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
