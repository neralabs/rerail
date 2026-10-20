//! Differential tests of the SQL engine against the native matcher, the parity oracle.

mod census;
mod differential;
mod hard_cases;
mod keyed;
mod leaves;
mod nodes;
mod probe;
mod recursion;
mod setops;
mod setup;
mod three_valued;

use crate::code_rule_dsl::CompiledCodeRuleDsl;
use serde_json::{Value, json};

/// A snapshot of `files`, each `(language, path, source)`.
pub(super) fn snapshot(files: &[(&str, &str, &str)]) -> Value {
    json!({
        "facts": [],
        "files": files
            .iter()
            .map(|(language, path, source)| json!({ "language": language, "path": path, "source": source }))
            .collect::<Vec<_>>(),
        "format": "rerail.repository-snapshot/v1",
        "revision": 0,
    })
}

/// The evaluation of `compiled` over `snapshot` by `engine`, as JSON.
pub(super) fn evaluate(compiled: &CompiledCodeRuleDsl, snapshot: &Value, engine: &str) -> Value {
    let output = compiled
        .evaluate(
            snapshot.to_string(),
            "report".to_owned(),
            Some(engine.to_owned()),
        )
        .unwrap_or_else(|error| panic!("{engine} evaluation fails: {}", error.reason));
    serde_json::from_str(&output).expect("evaluation is JSON")
}

/// Asserts that both engines evaluate `compiled` over `snapshot` alike, and returns the result.
pub(super) fn same(compiled: &CompiledCodeRuleDsl, snapshot: &Value) -> Value {
    let native = evaluate(compiled, snapshot, "native");
    let sql = evaluate(compiled, snapshot, "sql");
    assert_eq!(native, sql, "native and SQL disagree");
    native
}

/// The roots `grammar` matches in each of `docs`, by the SQL engine in batches of `batch` files on
/// one session.
pub(super) fn probe_sql(
    grammar: &crate::code_rule_dsl::matcher::GrammarMatcher,
    docs: &[crate::code_rule_dsl::tape::TapeDoc],
    batch: usize,
) -> (
    Vec<Vec<u32>>,
    std::time::Duration,
    crate::code_rule_dsl::sql::Stats,
) {
    use crate::code_rule_dsl::sql::ClauseRun;
    let started = std::time::Instant::now();
    let compiled = crate::code_rule_dsl::sql::compile(&[grammar]).expect("compiles");
    let mut session = compiled.session().expect("a session");
    let mut roots = Vec::with_capacity(docs.len());
    for chunk in docs.chunks(batch.max(1)) {
        let run = ClauseRun {
            statement: 0,
            files: (0..chunk.len()).map(|file| (file, None)).collect(),
        };
        let files = chunk
            .iter()
            .map(|doc| {
                (
                    doc.clone(),
                    crate::code_rule_dsl::sql::FileContext::default(),
                )
            })
            .collect();
        let found = session.run(files, &[run]).expect("runs");
        roots.extend(
            found
                .into_iter()
                .flatten()
                .map(|matches| matches.into_iter().map(|found| found.root).collect()),
        );
    }
    (roots, started.elapsed(), session.stats())
}

/// The statements one batch runs for a sample clause set: each clause's frames, and the stages
/// the clauses share (two clauses stopping at the same rule share its stage).
#[test]
fn a_sample_clause_set_runs_a_bounded_number_of_statements_per_batch() {
    use crate::code_rule_dsl::test_support::{compiled, grammar_matchers};
    let functions =
        json!({ "any": [{ "kind": "function_declaration" }, { "kind": "arrow_function" }] });
    let rules = [
        json!({ "rule": { "kind": "call_expression", "has": { "kind": "arguments", "has": { "pattern": "$X" } } } }),
        json!({ "rule": { "pattern": "console.log($$$A)", "inside": { "kind": "function_declaration", "stopBy": functions.clone() } } }),
        json!({ "rule": { "kind": "if_statement", "not": { "has": { "kind": "else_clause" } }, "any": [{ "regex": "^if \\(a" }, { "has": { "pattern": "return $R", "stopBy": "end" } }] } }),
        json!({ "rule": { "kind": "function_declaration", "count": { "has": { "kind": "if_statement", "stopBy": functions }, "min": 2, "as": "N" } } }),
    ];
    let compiled: Vec<CompiledCodeRuleDsl> = rules.into_iter().map(compiled).collect();
    let grammars: Vec<_> = compiled
        .iter()
        .map(|rule| &grammar_matchers(rule)[0])
        .collect();
    let program = crate::code_rule_dsl::sql::compile(&grammars).expect("compiles");
    let (stages, clauses) = program.statement_count();
    eprintln!("sample clause set: {stages} stage statements, {clauses} clause statements");
    // `count` groups into a table keyed by row (`Emitter::grouped`). No rule walks over a frame
    // that repeats a key (the nested `has` walks children, one row each): nothing is keyed. No
    // kind test is run on a frame whose nodes all pass it (`Emitter::kinds`): neither a root
    // frame's nor a walk's or a count's candidates are re-tested for the kinds they were read by.
    assert_eq!((stages, clauses), (4, 52));
}

/// Terminals run once per statement over their whole input set, never once per row: a fresh
/// pattern, an env-input pattern and a regex over a file of many candidate nodes. The fresh
/// pattern matches each of its 200 candidate roots once.
#[test]
fn terminals_run_once_per_statement_never_per_row() {
    use crate::code_rule_dsl::sql::terminals::calls::{
        FRESH_MATCHES, PATTERN_FILTERS, TEXT_FILTERS,
    };
    use crate::code_rule_dsl::test_support::compiled;
    let rule = compiled(json!({ "rule": {
        "pattern": "$F($$$A)",
        "regex": "^[a-z]",
        "has": { "stopBy": "end", "pattern": "$F" },
    } }));
    let source = "f(f, g);\n".repeat(200);
    for counter in [&FRESH_MATCHES, &PATTERN_FILTERS, &TEXT_FILTERS] {
        counter.set(0);
    }
    let result = same(&rule, &snapshot(&[("typescript", "a.ts", &source)]));
    assert_eq!(result["violations"].as_array().map(Vec::len), Some(200));
    assert_eq!(FRESH_MATCHES.get(), 200);
    assert_eq!(PATTERN_FILTERS.get(), 2);
    assert_eq!(TEXT_FILTERS.get(), 1);
}

/// A walk builds candidate rows only for the nodes of its sub-rule's root kinds (`not { not { R }
/// }` has `R`'s), and that double negation is one semi-join, not two anti-joins. Both engines
/// agree, three-valued too, where a sub-rule term after an unknown one may be withheld.
#[test]
fn walks_read_their_sub_rule_kinds_and_a_double_negation_is_a_semi_join() {
    use crate::code_rule_dsl::test_support::{compiled, grammar_matchers};
    let source = "function g() { f(1); h(x, 2); u(3); }\nconst y = k(u);\n";
    let calls = json!({ "not": { "not": { "kind": "call_expression" } } });
    let cases = [
        (
            json!({ "all": [calls.clone(), { "pattern": "$F($$$A)" }] }),
            true,
        ),
        // Three-valued: `$F` may be withheld past the unknown term, so only the kind test bounds.
        (
            json!({ "all": [{ "kind": "call_expression" }, { "has": { "kind": "identifier", "pattern": "$F", "testUnknown": "^u" } }, { "pattern": "$F($$$A)" }] }),
            true,
        ),
        // Nothing narrows a bare negation's candidates.
        (json!({ "not": { "kind": "call_expression" } }), false),
    ];
    for (sub, narrowed) in cases {
        let mut has = sub.clone();
        has["stopBy"] = json!("end");
        let rule = compiled(json!({ "rule": { "kind": "program", "has": has } }));
        same(&rule, &snapshot(&[("typescript", "a.ts", source)]));
        let grammars: Vec<_> = grammar_matchers(&rule).iter().collect();
        let program = crate::code_rule_dsl::sql::compile(&grammars).expect("compiles");
        let statements: Vec<&String> = program.statements().collect();
        let walk = statements
            .iter()
            .find(|statement| statement.contains("c.id <= fn.\"last\""))
            .expect("a has-end walk");
        assert_eq!(walk.contains("c.kind IN ("), narrowed, "{walk}");
        if sub.get("all").is_some_and(|all| all[0] == calls) {
            assert!(
                !statements
                    .iter()
                    .any(|statement| statement.contains("NOT IN")),
                "a double negation emits no anti-join"
            );
        }
    }
}

/// A walk to sites that need a field child of some kinds reads only candidates that have it
/// (`plan::Shape::Field`); no kind test is re-run on a frame whose nodes all pass it, under a
/// double negation either (`Emitter::kinds`); and a key leaves out the slots no input row binds
/// (`Emitter::nulls`): the candidates of a walk from the roots are keyed by `(file, node)` alone.
/// Both engines agree.
#[test]
fn walks_test_field_shapes_skip_implied_kind_tests_and_key_without_unbound_slots() {
    use crate::code_rule_dsl::test_support::{compiled, grammar_matchers};
    let source = "function g() { a.f(1); h(x); b.c.k(2); }\nconst y = u.v(3) + w(4);\n";
    let site = json!({ "all": [
        { "kind": "call_expression" },
        { "has": { "field": "function", "kind": "member_expression" } },
    ] });
    let rule = compiled(json!({ "rule": { "kind": "program", "has": {
        "stopBy": "end",
        "all": [
            { "not": { "not": site } },
            { "not": { "not": { "has": { "field": "function", "pattern": "$O.$M" } } } },
        ],
    } } }));
    same(&rule, &snapshot(&[("typescript", "a.ts", source)]));
    let grammars: Vec<_> = grammar_matchers(&rule).iter().collect();
    let program = crate::code_rule_dsl::sql::compile(&grammars).expect("compiles");
    let statements: Vec<&String> = program.statements().collect();
    let walk = statements
        .iter()
        .find(|statement| statement.contains("c.id <= fn.\"last\""))
        .expect("a has-end walk");
    assert!(walk.contains(".field = "), "the walk tests the field shape: {walk}");
    assert!(
        !statements.iter().any(|statement| statement
            .contains("JOIN node n ON n.file = f.file AND n.id = f.node WHERE n.kind")),
        "no kind test is re-run"
    );
    let keys: Vec<&&String> = statements
        .iter()
        .filter(|statement| statement.contains("SELECT DISTINCT f.file AS file, f.node AS node"))
        .collect();
    assert!(!keys.is_empty(), "the candidates are keyed");
    for key in keys {
        assert!(
            !key.contains("f.node AS node, f.s"),
            "a key holds no unbound slot: {key}"
        );
    }
}

/// A frame holds only the slots read after it (`emit`'s liveness), as `spt-022`'s walks from a
/// statement of many captures do: a walk's candidates carry the slots its sub-rule reads and
/// those it returns to the row, not every capture of the clause. A semi-join's carry none; a
/// first match that binds `$X` carries `$X` alone. Both engines agree.
#[test]
fn a_walk_s_candidates_hold_only_the_slots_read_after_them() {
    use crate::code_rule_dsl::test_support::{compiled, grammar_matchers};
    let source = "function g() { f(a, b, c, d); h(x, 2); }\nf(x, y, z, w);\n";
    // The candidates' slot columns of the has-end walk, and the clause's own.
    let widths = |rule: &CompiledCodeRuleDsl| -> (usize, usize) {
        let grammars: Vec<_> = grammar_matchers(rule).iter().collect();
        let program = crate::code_rule_dsl::sql::compile(&grammars).expect("compiles");
        let statements: Vec<&String> = program.statements().collect();
        let walk = statements
            .iter()
            .find(|statement| statement.contains("c.id <= fn.\"last\""))
            .expect("a has-end walk");
        // Each held slot's column: copied from the row, or `NULL` where no row binds it.
        let copied = walk
            .split_once("SELECT NULL, c.file, c.node")
            .and_then(|(_, rest)| rest.split_once(", c.pk"))
            .expect("a candidate row")
            .0
            .matches(", ")
            .count();
        let selected = statements
            .iter()
            .find_map(|statement| statement.strip_prefix("SELECT file, node"))
            .and_then(|rest| rest.split_once(" FROM "))
            .expect("the clause's select")
            .0
            .matches(", s")
            .count();
        (copied, selected)
    };
    let wide = json!({ "pattern": "$F($A, $B, $C, $D)" });
    let cases = [
        (
            json!({ "stopBy": "end", "kind": "identifier", "regex": "^x$" }),
            0,
        ),
        (
            json!({ "stopBy": "end", "kind": "identifier", "pattern": "$X" }),
            1,
        ),
    ];
    for (has, carried) in cases {
        let rule = compiled(json!({ "rule": {
            "kind": "call_expression",
            "all": [wide.clone(), { "inside": { "stopBy": "end", "kind": "program", "has": has } }],
        } }));
        same(&rule, &snapshot(&[("typescript", "a.ts", source)]));
        let (copied, selected) = widths(&rule);
        assert_eq!(
            copied, carried,
            "the walk's candidates carry {copied} slots"
        );
        assert!(
            selected > carried + 4,
            "the clause selects {selected} slots"
        );
    }
}

/// A stop set or `ofRule` set whose rule is a capture-free `not R` is `R`'s stage read as its
/// complement: no stage holds the negation's many nodes, and every reader (`nearest`, the
/// field walk's stop, `nthChild`'s ranks) inverts membership. So is one whose rule is an `any`
/// with a `not` branch, read by De Morgan as its dual `all`'s complement: the dual's stage reads
/// only its kinds' nodes. Both engines agree, three-valued too, where the rule cannot be unknown.
#[test]
fn a_negated_stop_or_of_rule_is_its_rule_s_stage_complemented() {
    use crate::code_rule_dsl::test_support::{compiled, grammar_matchers};
    let source =
        "function g(a) { if (a) { return (((a))); } f((1), [2], k(3)); }\nconst y = [k(u), (v)];\n";
    let wrappers =
        json!({ "not": { "any": [{ "kind": "parenthesized_expression" }, { "kind": "array" }] } });
    let dual = json!({ "any": [wrappers.clone(), { "kind": "return_statement" }] });
    for (stop, plain) in [(wrappers, true), (dual, false)] {
        let rules = [
            json!({ "kind": "identifier", "inside": { "stopBy": stop, "kind": "call_expression" } }),
            json!({ "kind": "statement_block", "has": { "stopBy": stop, "kind": "identifier" } }),
            json!({ "kind": "return_statement", "precedes": { "stopBy": stop, "kind": "expression_statement" } }),
            json!({ "kind": "call_expression", "has": { "field": "arguments", "stopBy": stop, "kind": "number" } }),
            json!({ "kind": "number", "nthChild": { "position": 1, "ofRule": stop } }),
            json!({ "kind": "identifier", "inside": { "stopBy": stop, "kind": "call_expression", "testUnknown": "^k" } }),
        ];
        for rule in rules {
            let compiled = compiled(json!({ "rule": rule }));
            same(&compiled, &snapshot(&[("typescript", "a.ts", source)]));
            let grammars: Vec<_> = grammar_matchers(&compiled).iter().collect();
            let program = crate::code_rule_dsl::sql::compile(&grammars).expect("compiles");
            let statements: Vec<&String> = program.statements().collect();
            assert!(
                statements
                    .iter()
                    .any(|statement| statement.contains("complement = 1")
                        || statement.contains("NOT EXISTS (SELECT 1 FROM stage_")),
                "{rule}: the negation is read as a complement"
            );
            // A stage's frames are named `s<stage>_…`: none reads every node of the batch.
            assert!(
                !statements
                    .iter()
                    .any(|statement| statement.starts_with("INSERT INTO s")
                        && statement
                            .contains("FROM node n WHERE n.file IN (SELECT file FROM runs")
                        && !statement.contains("AND n.kind IN (")),
                "{rule}: a stage reads only its rule's kinds"
            );
            // None computes the plain negation's anti-join (the dual's `not return_statement`
            // is one, over the wrappers' nodes alone).
            assert!(
                !plain
                    || !statements
                        .iter()
                        .any(|statement| statement.contains("NOT IN (SELECT k FROM s")),
                "{rule}: no stage holds the negation"
            );
        }
    }
}

/// A walk whose sub-rule needs more than a kind (here: a candidate that is a declaration or holds
/// a call below it) tests that shape on each candidate before its row is built, so a recursive
/// unit it feeds is demanded only where the sub-rule can hold. Both engines agree, three-valued
/// too.
#[test]
fn a_walk_tests_its_sub_rule_s_capture_free_shape_on_each_candidate() {
    use crate::code_rule_dsl::test_support::{compiled, grammar_matchers};
    let source = "function g(a) { let b = 1; a.x = 2; f(a); if (a) { k(b); } return b; }\n";
    let calls =
        json!({ "has": { "stopBy": "end", "kind": "call_expression", "pattern": "$C($$$)" } });
    let unknown =
        json!({ "has": { "stopBy": "end", "kind": "call_expression", "testUnknown": "^k" } });
    for branch in [calls, unknown] {
        let rule = compiled(json!({ "rule": {
            "kind": "return_statement",
            "follows": { "stopBy": "end", "any": [{ "kind": "lexical_declaration" }, branch] },
        } }));
        same(&rule, &snapshot(&[("typescript", "a.ts", source)]));
        let grammars: Vec<_> = grammar_matchers(&rule).iter().collect();
        let program = crate::code_rule_dsl::sql::compile(&grammars).expect("compiles");
        assert!(
            program
                .statements()
                .any(|statement| statement.contains("WHERE (EXISTS (SELECT 1 FROM node sh")),
            "the follows walk tests its candidates' shape"
        );
    }
}

/// Clauses alike, differing only in their patterns, share one shape: each runs its own texts (its
/// own terminals' ids, namespace and run id), which binding the shape gives back byte for byte
/// (`shapes::verify`, run on every stage and clause a test build binds), and finds on one session
/// what it finds compiled alone.
#[test]
fn clauses_alike_share_one_shape_and_find_what_each_finds_alone() {
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::sql::{ClauseRun, FileContext, compile_each};
    use crate::code_rule_dsl::test_support::{compiled, grammar_matchers};
    crate::code_rule_dsl::sql::VERIFY_SHAPES.store(true, std::sync::atomic::Ordering::Relaxed);
    let source = "function g(a) { console.log(a); f(1); return a; }\nfunction h() { console.warn(2); f(3); f(4); }\nconsole.log(5);\n";
    let mut rules = Vec::new();
    for pattern in ["console.log($A)", "console.warn($A)", "f($A)"] {
        rules.push(compiled(json!({ "rule": {
            "pattern": pattern,
            "inside": { "kind": "function_declaration", "stopBy": "end", "has": { "pattern": "return $R", "stopBy": "end" } },
        } })));
        rules.push(compiled(json!({ "rule": {
            "kind": "function_declaration",
            "count": { "has": { "pattern": pattern, "stopBy": "end" }, "min": 1, "as": "N" },
        } })));
    }
    let grammars: Vec<_> = rules
        .iter()
        .map(|rule| &grammar_matchers(rule)[0])
        .collect();
    let doc = differential::tape(GrammarDialect::TypeScript, source);
    let (program, outcomes) = compile_each(
        &grammars
            .iter()
            .map(|grammar| (*grammar, None))
            .collect::<Vec<_>>(),
    );
    let (instances, shapes, verified) = program.shared();
    assert_eq!(
        verified, instances,
        "every stage and clause gives its draft back"
    );
    assert!(
        shapes + 4 <= instances,
        "the three clauses of each form share a shape: {shapes} shapes of {instances}"
    );
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
    let mut matched = 0;
    for (grammar, found) in grammars.iter().zip(found) {
        let together: Vec<u32> = found[0].iter().map(|found| found.root).collect();
        let (alone, ..) = probe_sql(grammar, std::slice::from_ref(&doc), 1);
        assert_eq!(together, alone[0], "a clause finds what it finds alone");
        matched += together.len();
    }
    assert!(matched > 0, "the clauses find something");
}
