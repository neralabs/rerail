//! How long a session takes to set up for the data-layer benchmark's policies: every clause the
//! engine lowers, in one program as a workspace compiles them (`workspace::sql::SqlEngine`), and
//! one session over it, which creates every table the program's statements write. Run by hand:
//! `cargo test --release sql::tests::setup -- --ignored --nocapture`.

use crate::code_rule_dsl::CodeRuleDslRuntime;
use crate::code_rule_dsl::model::{ClauseKind, Query};
use crate::code_rule_dsl::test_support::CAPABILITIES;

#[test]
#[ignore = "reads the data-layer benchmark's policies from outside the repository"]
fn the_benchmark_policies_session_setup() {
    // Compiling the largest policies recurses deeper than a test thread's stack.
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(run)
        .expect("a setup thread")
        .join()
        .expect("the setup");
}

fn run() {
    let root = std::env::var("RERAIL_BENCH_POLICIES").unwrap_or_else(|_| {
        format!(
            "{}/rerail-bench/data-layer/policies",
            std::env::var("HOME").expect("HOME")
        )
    });
    let mut policies: Vec<_> = std::fs::read_dir(&root)
        .unwrap_or_else(|error| panic!("{root}: {error}"))
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("policy.json"))
        .filter(|path| path.exists())
        .collect();
    policies.sort();
    let compiled: Vec<_> = policies
        .iter()
        .map(|path| {
            CodeRuleDslRuntime::compile(
                std::fs::read_to_string(path).expect("policy"),
                "json".to_owned(),
                CAPABILITIES.to_owned(),
            )
            .unwrap_or_else(|error| panic!("{}: {}", path.display(), error.reason))
        })
        .collect();
    let mut grammars = Vec::new();
    for policy in &compiled {
        for clause in &policy.inner.clauses {
            if let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &clause.kind {
                for grammar in matchers {
                    grammars.push((grammar, clause.violation.each.as_deref()));
                }
            }
        }
    }
    let started = std::time::Instant::now();
    let (program, outcomes) = crate::code_rule_dsl::sql::compile_each(&grammars);
    let compiled_in = started.elapsed();
    let lowered = outcomes.iter().filter(|outcome| outcome.is_ok()).count();
    for attempt in 0..3 {
        let started = std::time::Instant::now();
        let session = program.session().expect("a session");
        let setup = started.elapsed();
        eprintln!(
            "session setup {attempt}: {:.3}s for {lowered} of {} grammar matchers ({} tables), program compiled in {:.1}s",
            setup.as_secs_f64(),
            grammars.len(),
            session.tables().expect("tables"),
            compiled_in.as_secs_f64(),
        );
    }
}
