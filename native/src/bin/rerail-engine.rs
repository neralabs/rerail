//! `rerail-engine`: the engine's tools without Node (P `spec-tooling.md` §2.7).
//!
//!   rerail-engine dump --rule <file>... [--clause <id>] [--fused] [--setplan] [--capabilities <json>]
//!   rerail-engine asm <file.rir>
//!   rerail-engine logic <program.rr | dir of .rr> <corpus root> [<T facts.sqlite>] <out dir> [--modes <modes> | --sql] [--metal] [--chunk <nodes>]
//!   rerail-engine gc <store.sqlite> [--history-mb <n>]
//!   rerail-engine sql <store.sqlite> <request.json>
//!   rerail-engine flatten <program.rr>
//!   rerail-engine csv <out dir> [<to dir>]   (a run's outputs, `outputs.rro`, as `.csv` files)
//!   rerail-engine run-rr | run-policies | check-rr | convert-rr ... (`tools::cli`)
//!
//! Every command takes `--memory-mb <n>` (the overall memory budget, 1 GB by default) and `--metal-mb <n>` (the Metal
//! budget, on top of it: half the overall budget by default); `RERAIL_MEMORY_MB` and `RERAIL_METAL_MB` set them too.
//!
//! Runaways (decision 0462): a derived relation holds at most `RERAIL_WIDEST_ROWS` rows (50 M by default). Past it the
//! run stops naming the relation's rules, or with `RERAIL_WIDEST=cut` keeps the rows so far, goes on, and exits 3.
//! A run past `RERAIL_MEMORY_MAX_MB` resident (three quarters of the machine's memory by default) is stopped, naming
//! its widest relations; a path that does not stop by then, the process stops, exit code 3.
use std::process::ExitCode;

fn main() -> ExitCode {
    // A profiler's start (dev, `RERAIL_START_PAUSE_MS`): a sampler attached by pid sees the run from its first step.
    if let Some(ms) = std::env::var("RERAIL_START_PAUSE_MS").ok().and_then(|v| v.parse::<u64>().ok()) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
    // Before any thread starts: a stop signal or `RERAIL_DEADLINE` dumps the run first (decision 0084).
    rerail_engine::tools::watch();
    // Before any block is made: the one block policy. The command's machine: its pool, of the most threads the
    // process uses (`RERAIL_THREADS`), this thread one of them (decision 0442).
    rerail_engine::threads::configure();
    let machine = rerail_engine::threads::Machine::here();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (out, incomplete) = rerail_engine::tools::cli(&args, &machine);
    match out {
        Ok(text) => {
            print!("{text}");
            // Decision 0462: the command cut a relation at the width cap (`RERAIL_WIDEST=cut`): its results are
            // incomplete. Decision 0442: it read state outside any run, which was refused.
            match incomplete {
                Some(message) => {
                    eprintln!("rerail-engine: {message}");
                    ExitCode::from(3)
                }
                None => ExitCode::SUCCESS,
            }
        }
        Err(error) => {
            eprintln!("rerail-engine: {error}");
            ExitCode::FAILURE
        }
    }
}
