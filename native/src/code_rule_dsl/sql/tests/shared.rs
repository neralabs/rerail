//! Units whose answers are the batch's (`emit::demand_table`): a non-recursive, capture-free,
//! context-free and two-valued utility's unit, whose demand and answer tables every clause of a
//! batch reaching an equal utility shares under its memo term.

use super::{differential, probe_sql, same, snapshot};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::sql::exec::STATEMENT_PROFILE;
use crate::code_rule_dsl::sql::{ClauseRun, FileContext, compile_each};
use crate::code_rule_dsl::tape::TapeDoc;
use crate::code_rule_dsl::test_support::{compiled, grammar_matchers};
use serde_json::{Value, json};
use std::collections::BTreeSet;

const SOURCE: &str = "function g(a) { f(a, 1); return a.push(b); }\n\
class C { m(x) { k(x, 2); } }\n\
const y = h(z, 3);\n\
function n() { if (q) { w(4); } }\n";

/// A capture-free, context-free utility: whether a node lies in a function declaration.
fn in_function() -> Value {
    json!({ "inside": { "stopBy": "end", "kind": "function_declaration" } })
}

/// The identifiers, each tested twice against `in-function` (so the utility is a unit), in or
/// out of a function declaration: every identifier is demanded.
fn identifiers() -> Value {
    json!({
        "rule": { "kind": "identifier", "any": [
            { "matches": "in-function" },
            { "not": { "matches": "in-function" } },
        ] },
        "utils": { "in-function": in_function() },
    })
}

/// The numbers in a function declaration, or in arguments that are: numbers and arguments are
/// demanded, no identifier.
fn numbers() -> Value {
    json!({
        "rule": { "kind": "number", "any": [
            { "matches": "in-function" },
            { "inside": { "kind": "arguments", "matches": "in-function" } },
        ] },
        "utils": { "in-function": in_function() },
    })
}

/// The identifiers whose parent is in a function declaration: the identifiers are demanded
/// again, and their parents.
fn parents() -> Value {
    json!({
        "rule": { "kind": "identifier", "any": [
            { "inside": { "matches": "in-function" } },
            { "matches": "in-function", "inside": { "kind": "arguments" } },
        ] },
        "utils": { "in-function": in_function() },
    })
}

/// What a program of `rules`, in order, over one batch of `doc` does: the roots each clause
/// finds, the keys its units' batch tables evaluate (the rows the loops mark clean in them) and
/// the batch tables its statements name.
fn program(rules: &[Value], doc: &TapeDoc) -> (Vec<Vec<u32>>, usize, BTreeSet<String>) {
    let compiled: Vec<_> = rules.iter().map(|rule| compiled(rule.clone())).collect();
    let grammars: Vec<_> = compiled
        .iter()
        .map(|rule| (&grammar_matchers(rule)[0], None))
        .collect();
    let (program, outcomes) = compile_each(&grammars);
    let tables: BTreeSet<String> = program
        .planned_statements()
        .iter()
        .flat_map(|sql| sql.split([' ', '(', ')', ',']))
        .filter(|word| word.starts_with("memo_"))
        .map(str::to_owned)
        .collect();
    let runs: Vec<ClauseRun> = outcomes
        .iter()
        .map(|outcome| ClauseRun {
            statement: *outcome.as_ref().expect("compiles"),
            files: vec![(0, None)],
        })
        .collect();
    let mut session = program.session().expect("a session");
    STATEMENT_PROFILE.set(Some(Default::default()));
    let found = session
        .run(vec![(doc.clone(), FileContext::default())], &runs)
        .expect("runs");
    let profile = STATEMENT_PROFILE.take().unwrap_or_default();
    let evaluated = profile
        .iter()
        .filter(|(sql, _)| sql.starts_with("UPDATE memo_") && sql.contains("SET dirty = 0"))
        .map(|(_, (_, rows, _))| rows)
        .sum();
    let roots = found
        .into_iter()
        .map(|found| found[0].iter().map(|found| found.root).collect())
        .collect();
    (roots, evaluated, tables)
}

/// Three clauses reaching one utility, each demanding keys of its own (identifiers; numbers and
/// arguments; identifiers and their parents) read one pair of tables named by the utility's memo
/// term: each key is evaluated once in the batch, by the first clause demanding it, and every
/// clause finds what it finds alone and what the native matcher finds.
#[test]
fn clauses_reaching_one_context_free_utility_share_its_answers_in_the_batch() {
    crate::code_rule_dsl::sql::VERIFY_SHAPES.store(true, std::sync::atomic::Ordering::Relaxed);
    let doc = differential::tape(GrammarDialect::TypeScript, SOURCE);
    let files = snapshot(&[("typescript", "a.ts", SOURCE)]);
    let rules = [identifiers(), numbers(), parents()];
    let (together, all, tables) = program(&rules, &doc);
    assert_eq!(
        tables.len(),
        2,
        "one demand and one answer table for every clause: {tables:?}"
    );
    let mut alone = Vec::new();
    for (rule, together) in rules.iter().zip(&together) {
        let rule_compiled = compiled(rule.clone());
        same(&rule_compiled, &files);
        let grammar = &grammar_matchers(&rule_compiled)[0];
        let (roots, ..) = probe_sql(grammar, std::slice::from_ref(&doc), 1);
        assert_eq!(*together, roots[0], "a clause finds what it finds alone");
        let (_, evaluated, _) = program(std::slice::from_ref(rule), &doc);
        alone.push(evaluated);
    }
    assert!(
        together.iter().all(|roots| !roots.is_empty()),
        "every clause finds something: {together:?}"
    );
    let (_, twice, _) = program(&[identifiers(), identifiers()], &doc);
    assert_eq!(
        twice, alone[0],
        "a clause demanding only keys evaluated before evaluates none"
    );
    // The first two demand no key alike; the third adds the parents to the first's identifiers.
    let (_, first_two, _) = program(&rules[..2], &doc);
    assert_eq!(first_two, alone[0] + alone[1]);
    assert!(
        alone[0] + alone[1] < all && all < alone.iter().sum::<usize>(),
        "the third clause evaluates only its keys the first did not demand: {all} of {alone:?}"
    );
}

/// A unit whose answer depends on more than its node keeps tables of its statement's own: one
/// that binds a capture, one in a three-valued clause.
#[test]
fn units_reading_more_than_their_node_keep_their_statement_s_tables() {
    let doc = differential::tape(GrammarDialect::TypeScript, SOURCE);
    let binding = json!({
        "rule": { "kind": "identifier", "any": [
            { "matches": "named" },
            { "inside": { "matches": "named" } },
        ] },
        "utils": { "named": { "pattern": "$N", "inside": { "stopBy": "end", "kind": "function_declaration" } } },
    });
    let three = json!({
        "rule": { "kind": "identifier", "any": [
            { "matches": "in-function", "testUnknown": "^a" },
            { "inside": { "matches": "in-function" } },
        ] },
        "utils": { "in-function": in_function() },
    });
    for rule in [binding, three] {
        let (_, evaluated, tables) = program(std::slice::from_ref(&rule), &doc);
        assert!(
            tables.is_empty() && evaluated == 0,
            "no batch table: {tables:?} for {rule}"
        );
    }
}
