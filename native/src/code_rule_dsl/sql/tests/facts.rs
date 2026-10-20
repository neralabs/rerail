//! Shared literal facts (`sql::facts`): clauses whose sub-rules differ only in their regexes read
//! one fact, each its own member, and find what each finds alone and what the native matcher
//! finds.

use super::{probe_sql, same, snapshot};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::sql::pushdown::SqlLowering;
use crate::code_rule_dsl::sql::tests::differential;
use crate::code_rule_dsl::sql::{ClauseRun, FileContext, compile_each};
use crate::code_rule_dsl::test_support::{compiled, grammar_matchers};
use serde_json::{Value, json};

const SOURCE: &str = "function g(a) { console.log(a); f(1); return a.push(2); }\n\
function h() { console.warn(2); f(3); a.sort(); log(5); }\n\
class C { m() { this.log(1); push(4); console.warn(7); } }\n\
function k() { return [1].map((x) => x.log(x)); }\n";

/// A call whose callee is named by `name` (an identifier) or `property` (a member's property),
/// and, where `filter`, no call `console.warn(…)` of a pattern: a test no transducer program takes.
fn callee(name: &str, property: &str, filter: bool) -> Value {
    let mut call = json!({
        "kind": "call_expression",
        "has": { "field": "function", "any": [
            { "kind": "identifier", "regex": name },
            { "kind": "member_expression", "has": { "field": "property", "regex": property } },
        ] },
    });
    if filter {
        call["not"] = json!({ "pattern": "console.warn(7)" });
    }
    call
}

/// The rules: one member each, one of two members, a member under `not` and one whose holes hold
/// different regexes.
fn rules(filter: bool) -> Vec<Value> {
    let mut rules: Vec<Value> = ["^f$", "^log$", "^push$", "^sort$", "^nope$", "o"]
        .iter()
        .map(|name| {
            json!({ "rule": {
                "kind": "function_declaration",
                "has": { "stopBy": "end", "all": [callee(name, name, filter)] },
            } })
        })
        .collect();
    rules.push(json!({ "rule": {
        "kind": "method_definition",
        "any": [
            { "has": { "stopBy": "end", "all": [callee("^push$", "^push$", filter)] } },
            { "has": { "stopBy": "end", "all": [callee("^log$", "^log$", filter)] } },
        ],
    } }));
    rules.push(json!({ "rule": {
        "kind": "function_declaration",
        "not": { "has": { "stopBy": "end", "all": [callee("^f$", "^f$", filter)] } },
    } }));
    rules.push(json!({ "rule": {
        "kind": "arrow_function",
        "inside": { "stopBy": "end", "kind": "call_expression", "has": { "stopBy": "end", "all": [callee("^g$", "^log$", filter)] } },
    } }));
    rules
}

/// Compiles `rules` into one program and checks it reads one fact with several members, and
/// that each clause finds there what the native matcher and the clause alone find.
/// What a program of the rules holds: one fact several clauses read, facts of their own, none.
enum Facts {
    Shared,
    Own,
    None,
}

fn agree(rules: &[Value], facts: Facts) {
    crate::code_rule_dsl::sql::VERIFY_SHAPES.store(true, std::sync::atomic::Ordering::Relaxed);
    let files = snapshot(&[("typescript", "a.ts", SOURCE)]);
    let compiled: Vec<_> = rules.iter().map(|rule| compiled(rule.clone())).collect();
    let mut found_any = 0;
    for rule in &compiled {
        found_any += same(rule, &files)["violations"]
            .as_array()
            .map_or(0, Vec::len);
    }
    assert!(found_any > 0, "the rules find something");
    let grammars: Vec<_> = compiled
        .iter()
        .map(|rule| &grammar_matchers(rule)[0])
        .collect();
    let (program, outcomes) = compile_each(
        &grammars
            .iter()
            .map(|grammar| (*grammar, None))
            .collect::<Vec<_>>(),
    );
    let statements = program.planned_statements();
    let fills = statements
        .iter()
        .filter(|sql| sql.contains("(lit, file, node)"))
        .count();
    // Each fact's members its readers read: `FROM stage_<n> st WHERE st.lit = <m>`.
    let mut members: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        Default::default();
    for sql in &statements {
        for (at, _) in sql.match_indices(" st WHERE st.lit = ") {
            let stage = sql[..at].rsplit(' ').next().unwrap_or_default().to_owned();
            let member = sql[at..].split(' ').nth(5).unwrap_or_default().to_owned();
            members.entry(stage).or_default().insert(member);
        }
    }
    match facts {
        Facts::Shared => assert!(
            fills >= 1 && members.values().any(|read| read.len() >= 5),
            "the clauses read their own members of one fact: {members:?}"
        ),
        // A pattern's terminal is its clause's own, so each clause's filter stage, and its
        // fact, are too.
        Facts::Own => assert!(fills >= 1, "the clauses read facts: {members:?}"),
        Facts::None => assert_eq!(fills, 0, "pushdown's rules make no fact"),
    }
    let doc = differential::tape(GrammarDialect::TypeScript, SOURCE);
    let runs: Vec<ClauseRun> = outcomes
        .iter()
        .map(|outcome| ClauseRun {
            statement: *outcome.as_ref().expect("compiles"),
            files: vec![(0, None)],
        })
        .collect();
    let mut session = program.session().expect("a session");
    let found = session
        .run(vec![(doc.clone(), FileContext::default())], &runs)
        .expect("runs");
    for (grammar, found) in grammars.iter().zip(found) {
        let together: Vec<u32> = found[0].iter().map(|found| found.root).collect();
        let (alone, ..) = probe_sql(grammar, std::slice::from_ref(&doc), 1);
        assert_eq!(together, alone[0], "a clause finds what it finds alone");
    }
}

/// Without pushdown, the callee tests of every clause are one fact.
#[test]
fn clauses_differing_in_a_regex_read_one_fact_and_agree() {
    let _sql = SqlLowering::hold();
    agree(&rules(false), Facts::Shared);
}

/// With pushdown, the node-local callee tests are transducer programs, and no fact; with a test
/// no program takes (a pattern), they are one fact again.
#[test]
fn a_fact_takes_only_what_pushdown_does_not() {
    agree(&rules(false), Facts::None);
    agree(&rules(true), Facts::Own);
}
