//! What a compiled document holds, read from jemalloc's counters: Windows builds use the system
//! allocator, which keeps none.

use crate::code_rule_dsl::test_support::*;
use serde_json::{Map, Value, json};

/// The bytes `build` leaves allocated on this thread, with what it built: jemalloc's
/// per-thread counters, which other tests' threads do not move.
fn retained<T>(build: impl FnOnce() -> T) -> (T, i64) {
    use tikv_jemalloc_ctl::thread::{allocatedp, deallocatedp};
    let allocated = allocatedp::read().expect("jemalloc counts the thread's allocations");
    let deallocated = deallocatedp::read().expect("jemalloc counts the thread's frees");
    let held = || allocated.get() as i64 - deallocated.get() as i64;
    let before = held();
    let built = build();
    (built, held() - before)
}

/// The bytes a compiled document of one define, `utils`, holds, with a query per rule of
/// `reaches`. Each query declares a global of its own, as a rule generated from a linter's
/// list of built-in APIs declares the ones it reads, so its `languageOptions` compile the
/// utilities it reaches in a shared table of their own, once per grammar.
fn held_by(utils: Map<String, Value>, reaches: Vec<Value>) -> i64 {
    let mut clauses =
        vec![json!({ "id": "table", "define": { "language": "typescript", "utils": utils } })];
    clauses.extend(reaches.into_iter().enumerate().map(|(query, rule)| {
        json!({ "id": format!("q{query}"), "violation": violation(), "forbid": { "ast": {
            "language": "typescript",
            "languageOptions": { "globals": { format!("g{query}"): "readonly" } },
            "rule": rule,
        } } })
    }));
    let document = json!({ "format": "rerail.policy/v1", "id": "table", "revision": 1, "mode": "steer", "clauses": clauses });
    let capabilities = serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities");
    // A copy goes in: the compilation frees what it is given, which `retained` would take
    // off what the rule holds.
    let (compiled, held) = retained(|| compile_document_with(document.clone(), &capabilities));
    compiled.unwrap_or_else(|error| panic!("{}", error.reason));
    held
}

#[test]
fn a_document_holds_each_regex_pattern_once_however_many_tables_compile_it() {
    // 120 utilities of 2 regexes each over 8 patterns, all reached by 8 queries in 3
    // grammars: 5,760 regex terms. Compiled one per term they held about 85 MiB.
    let utils = (0..120)
        .map(|index| {
            let rule = json!({ "any": [
                { "kind": "property_identifier", "regex": format!("^name{}$", index % 4) },
                { "kind": "string", "value": { "regex": format!("^(?:node:)?module{}$", index % 4) } },
            ] });
            (format!("u{index}"), rule)
        })
        .collect();
    let every = (0..120)
        .map(|index| json!({ "matches": format!("u{index}") }))
        .collect::<Vec<_>>();
    let held = held_by(utils, vec![json!({ "any": every }); 8]);
    assert!(held < 16 << 20, "the document holds {} MiB", held >> 20);
}

#[test]
fn a_shared_table_holds_only_the_utilities_its_queries_reach() {
    // 24 queries in 3 grammars, each reaching one of 2,000 utilities: 72 shared tables. With
    // room for every utility's rule and memo in each, they held about 30 MiB.
    let utils = (0..2000)
        .map(|index| (format!("u{index}"), json!({ "kind": "identifier" })))
        .collect();
    let reaches = (0..24)
        .map(|query| json!({ "matches": format!("u{query}") }))
        .collect();
    let held = held_by(utils, reaches);
    assert!(held < 8 << 20, "the document holds {} MiB", held >> 20);
}
